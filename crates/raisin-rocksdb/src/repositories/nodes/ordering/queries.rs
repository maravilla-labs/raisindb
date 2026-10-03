//! Query operations for child ordering
//!
//! This module provides efficient query functions for retrieving order labels
//! and child lists without loading full node objects.
//!
//! All key parsing goes through
//! [`super::parse_ordered_child_key`] — see that module for why the key tail
//! cannot be split on null bytes.

use super::super::helpers::is_tombstone;
use super::super::NodeRepositoryImpl;
use super::parse_ordered_child_key;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use std::collections::HashSet;

impl NodeRepositoryImpl {
    /// Get the current HEAD order label for a specific child
    ///
    /// Thin wrapper over [`stored_order_label`], the one reader of a child's
    /// stored label (the delete tombstoner uses it too).
    ///
    /// Returns None if the child is not found in the ordered index.
    pub(crate) fn get_order_label_for_child(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        child_id: &str,
    ) -> Result<Option<String>> {
        stored_order_label(
            &self.db, tenant_id, repo_id, branch, workspace, parent_id, child_id,
        )
    }

    /// Get order labels for TWO children efficiently (for insert-between)
    ///
    /// Returns (before_label, after_label) with targeted queries.
    /// No full child list loading!
    pub(in crate::repositories::nodes) fn get_adjacent_labels(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        before_child: Option<&str>,
        after_child: Option<&str>,
    ) -> Result<(Option<String>, Option<String>)> {
        let before_label = if let Some(child_id) = before_child {
            self.get_order_label_for_child(
                tenant_id, repo_id, branch, workspace, parent_id, child_id,
            )?
        } else {
            None
        };

        let after_label = if let Some(child_id) = after_child {
            self.get_order_label_for_child(
                tenant_id, repo_id, branch, workspace, parent_id, child_id,
            )?
        } else {
            None
        };

        Ok((before_label, after_label))
    }

    /// Get the order label of the last child (for appending)
    ///
    /// **OPTIMIZED**: Uses cached metadata for O(1) lookup instead of O(n) scan.
    /// Falls back to full scan if cache is missing (e.g. after migration).
    ///
    /// The fallback returns the label with the highest HLC (most recently
    /// added), NOT the lexicographic maximum: sequential appends do not
    /// necessarily maintain lex order across all labels.
    pub(crate) fn get_last_order_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Result<Option<String>> {
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // OPTIMIZATION: Check metadata cache first (O(1))
        let metadata_key =
            keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
        if let Ok(Some(cached_value)) = self.db.get_cf(cf_ordered, &metadata_key) {
            return Ok(Some(String::from_utf8_lossy(&cached_value).to_string()));
        }

        // Cache miss - fall back to full scan (O(n)). Happens on first insert to
        // a parent or after cache invalidation.
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let iter = crate::prefix_scan(&self.db, cf_ordered, prefix.clone());

        let mut last_label: Option<String> = None;
        let mut highest_revision = HLC::new(0, 0);
        let mut seen_labels = HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }
            if is_tombstone(&value) {
                continue;
            }
            let Some(parsed) = parse_ordered_child_key(&key, &prefix) else {
                continue;
            };
            let Some(revision) = parsed.revision() else {
                continue;
            };

            // Due to ~HLC encoding the first occurrence per label has the
            // highest revision, but we still need the global max across labels.
            if seen_labels.insert(parsed.order_label.to_string()) && revision > highest_revision {
                highest_revision = revision;
                last_label = Some(parsed.order_label.to_string());
            }
        }

        Ok(last_label)
    }

    /// Check if the given order_label is lexicographically >= all other children's labels
    ///
    /// Used by reorder operations to intelligently update the metadata cache.
    /// Returns true if new_label should become the cache entry for last child.
    ///
    /// Optionally excludes a specific child_id from comparison (useful when that
    /// child is being reordered and may still have old entries in the index).
    pub(crate) fn is_lexicographically_last_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        test_label: &str,
        exclude_child_id: Option<&str>,
    ) -> Result<bool> {
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let iter = crate::prefix_scan(&self.db, cf_ordered, prefix.clone());

        let mut seen_labels = HashSet::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }
            if is_tombstone(&value) {
                continue;
            }
            let Some(parsed) = parse_ordered_child_key(&key, &prefix) else {
                continue;
            };

            if Some(parsed.child_id) == exclude_child_id {
                continue;
            }

            // Only check each label once (first occurrence is newest due to ~rev)
            if seen_labels.insert(parsed.order_label.to_string()) && parsed.order_label > test_label
            {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Get ordered list of child IDs at HEAD (lightweight - IDs only, no node objects)
    ///
    /// Returns child IDs in order based on their order_labels.
    /// This is more efficient than loading full nodes.
    pub(in crate::repositories::nodes) async fn get_ordered_child_ids(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<String>> {
        Ok(self
            .list_ordered_children_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                super::OrderedScanStart::Beginning,
                None,
                false,
                max_revision,
            )?
            .into_iter()
            .map(|entry| entry.child_id)
            .collect())
    }

    /// Find a child ID by name using the ordered children index
    ///
    /// Child names are stored as the entry values, so no node is loaded. Each
    /// `(label, child)` pair is decided by its NEWEST entry, through the one
    /// ordered-children scan: an older revision of an entry — a name before a
    /// rename, or an entry since tombstoned — is never matched.
    pub(in crate::repositories::nodes) fn find_child_id_by_name(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        child_name: &str,
    ) -> Result<Option<String>> {
        let mut found = None;
        self.scan_ordered_children(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            super::OrderedScanStart::Beginning,
            false,
            None,
            |child_id, _label, name| {
                if name == child_name.as_bytes() {
                    found = Some(child_id.to_string());
                    return Ok(false);
                }
                Ok(true)
            },
        )?;
        Ok(found)
    }
}

/// The label `child_id` is stored under in `parent_id`'s ORDERED_CHILDREN, at
/// HEAD: newest entry per `(label, child)` wins, tombstones hide older
/// entries. This can differ from the node's `order_key` (legacy drift, merge
/// verbatim copies), which is why a delete must tombstone THIS label.
pub(crate) fn stored_order_label(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    child_id: &str,
) -> Result<Option<String>> {
    let prefix = keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
    let cf_ordered = cf_handle(db, cf::ORDERED_CHILDREN)?;
    let iter = crate::prefix_scan(db, cf_ordered, prefix.clone());

    // Track seen (order_label, child_id) pairs to handle MVCC properly.
    // With descending HLC, newer entries come first - we want the most
    // recent non-tombstone.
    let mut seen_labels: HashSet<(String, String)> = HashSet::new();

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        if !key.starts_with(&prefix) {
            break;
        }
        let Some(parsed) = parse_ordered_child_key(&key, &prefix) else {
            continue;
        };

        // Track this (order_label, child_id) pair - skip older revisions.
        let entry_key = (parsed.order_label.to_string(), parsed.child_id.to_string());
        if !seen_labels.insert(entry_key) {
            continue;
        }

        // Skip tombstones (already tracked above so older revisions of the
        // same entry cannot resurrect it).
        if is_tombstone(&value) {
            continue;
        }

        if parsed.child_id == child_id {
            return Ok(Some(parsed.order_label.to_string()));
        }
    }

    Ok(None)
}

/// The ORDERED_CHILDREN parent key of the node at `node_path`, as of `at`
/// (HEAD when `None`): `/` for a root child, else the node id PATH_INDEX
/// gives the parent path. `Node.parent` holds the parent's NAME and must never
/// be used as this key — that is how the delete tombstoner came to tombstone
/// a key no entry lives under.
pub(crate) fn parent_index_id(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_path: &str,
    at: Option<&HLC>,
) -> Result<Option<String>> {
    let parent_path = match node_path.rsplit_once('/') {
        Some(("", _)) => "/",
        Some((parent, _)) => parent,
        None => return Ok(None),
    };
    if parent_path == "/" {
        return Ok(Some("/".to_string()));
    }
    let prefix = keys::path_index_key_prefix(tenant_id, repo_id, branch, workspace, parent_path);
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;
    Ok(
        crate::mvcc_read::newest_at_or_before_with(db, cf_path, &prefix, at, |_, value| {
            (!is_tombstone(value)).then(|| String::from_utf8_lossy(value).into_owned())
        })?
        .flatten(),
    )
}
