//! Compound index repository implementation
//!
//! Provides multi-column compound indexes for efficient ORDER BY + filter queries.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::scope::StorageScope;
use raisin_storage::{CompoundColumnValue, CompoundIndexRepository, CompoundIndexScanEntry};
use rocksdb::DB;
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone)]
pub struct CompoundIndexRepositoryImpl {
    db: Arc<DB>,
}

impl CompoundIndexRepositoryImpl {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Parse node_id from a compound index key.
    ///
    /// Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0cidx{_pub}\0{index_name}\0{col1}\0{col2}\0...\0{timestamp}\0{~revision}\0{node_id}
    fn parse_node_id_from_key(key: &[u8]) -> Option<String> {
        // Split by null bytes and get the last part (node_id)
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        if parts.is_empty() {
            return None;
        }

        let node_id_bytes = parts.last()?;
        if node_id_bytes.is_empty() {
            return None;
        }

        String::from_utf8(node_id_bytes.to_vec()).ok()
    }
}

/// Decide each `(tuple, node)` by its NEWEST entry at or below `max_revision`
/// (plan Phase 8 step 1) and keep the live ones, in index order, one row per
/// node.
///
/// The entries of one tuple sort by descending revision, so the first entry
/// seen for a `(tuple, node)` within the bound is the newest. A tombstone
/// decides only ITS tuple: the old reader kept one node-wide "tombstoned" set,
/// so a node whose OLD tuple was ended at revision R vanished from every
/// tuple sorting after it — and it relied on the writer overwriting keys in
/// place, which erased history. Entries above the bound are ignored, so a
/// read at `__revision = N` sees each tuple as it stood at N.
///
/// A node with two live tuples (an index left inconsistent by an older
/// writer) is reported once, at its first position in index order.
pub(crate) fn newest_per_tuple<'k>(
    entries: impl Iterator<Item = (&'k [u8], bool)>,
    max_revision: Option<&HLC>,
    limit: Option<usize>,
) -> Vec<CompoundIndexScanEntry> {
    use crate::indexing::compound::{entries::trailing_i64, parse_entry_key};
    let mut decided: HashSet<(&[u8], &str)> = HashSet::new();
    let mut emitted: HashSet<&str> = HashSet::new();
    let mut results = Vec::new();
    for (key, dead) in entries {
        let Some((group, at, node_id)) = parse_entry_key(key) else {
            continue;
        };
        if max_revision.is_some_and(|bound| at > *bound) {
            continue;
        }
        if !decided.insert((group, node_id)) || dead {
            continue;
        }
        if emitted.insert(node_id) {
            results.push(CompoundIndexScanEntry {
                node_id: node_id.to_string(),
                timestamp: trailing_i64(group),
            });
            if limit.is_some_and(|lim| results.len() >= lim) {
                break;
            }
        }
    }
    results
}

impl CompoundIndexRepository for CompoundIndexRepositoryImpl {
    async fn index_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        revision: &HLC,
        node_id: &str,
        is_published: bool,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        let key = keys::compound_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            index_name,
            column_values,
            revision,
            node_id,
            is_published,
        );

        tracing::debug!(
            "CompoundIndex: Indexing node '{}' in index '{}' (published: {})",
            node_id,
            index_name,
            is_published
        );

        self.db
            .put_cf(cf, key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(())
    }

    async fn unindex_compound(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        column_values: &[CompoundColumnValue],
        node_id: &str,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        // Build prefix for both draft and published
        for published in [false, true] {
            let prefix = keys::compound_index_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                index_name,
                column_values,
                published,
            );

            let prefix_clone = prefix.clone();
            let iter = crate::prefix_scan(&self.db, cf, prefix);

            for item in iter {
                let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                // Verify key actually starts with our prefix
                if !key.starts_with(&prefix_clone) {
                    break;
                }

                // Check if this key is for our node
                if let Some(key_node_id) = Self::parse_node_id_from_key(&key) {
                    if key_node_id == node_id {
                        self.db
                            .delete_cf(cf, &key)
                            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn scan_compound_index(
        &self,
        scope: StorageScope<'_>,
        index_name: &str,
        equality_values: &[CompoundColumnValue],
        published_only: bool,
        ascending: bool,
        limit: Option<usize>,
        max_revision: Option<&HLC>,
    ) -> Result<Vec<CompoundIndexScanEntry>> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        // A node is indexed under EXACTLY ONE tag — `cidx_pub` when
        // `published_at` is set, `cidx` otherwise. So "every node matching this
        // prefix" is the UNION of both tags, not either one. Reading only
        // `cidx` silently dropped every published row, and because the planner
        // strips the matched equality predicates from the residual filter there
        // was no filter left to catch the loss.
        let tags: &[bool] = if published_only {
            &[true]
        } else {
            &[false, true]
        };

        // Collect from each tag, keyed by the suffix AFTER the tag-bearing
        // prefix — the index-order key `…{columns}\0{~revision}\0{node_id}` —
        // so the union sorts exactly as one keyspace would.
        let mut merged: Vec<(Vec<u8>, Vec<u8>, bool)> = Vec::new();
        for &published in tags {
            let prefix = keys::compound_index_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                index_name,
                equality_values,
                published,
            );
            for item in crate::prefix_scan(&self.db, cf, &prefix) {
                let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                if !key.starts_with(&prefix) {
                    break;
                }
                let suffix = key[prefix.len()..].to_vec();
                merged.push((
                    suffix,
                    key.to_vec(),
                    crate::keys::is_tombstone_value(&value),
                ));
            }
        }
        merged.sort_by(|a, b| a.0.cmp(&b.0));

        let results = newest_per_tuple(
            merged.iter().map(|(_, key, dead)| (key.as_slice(), *dead)),
            max_revision,
            ascending.then_some(limit).flatten(),
        );
        let mut results = results;

        // Descending: the iterator stays FORWARD (a reverse walk would meet a
        // group's OLDEST revision first and resurrect superseded entries), so
        // the whole equality group is read and then reversed and truncated.
        // `Timestamp` columns are stored newest-first (`TimestampDesc`), which
        // is what keeps the common "newest first" query a bounded FORWARD scan.
        if !ascending {
            results.reverse();
            if let Some(lim) = limit {
                results.truncate(lim);
            }
        }
        tracing::debug!("CompoundIndex: Scan returned {} results", results.len());
        Ok(results)
    }

    async fn remove_all_compound_indexes_for_node(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> Result<()> {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        let cf = cf_handle(&self.db, cf::COMPOUND_INDEX)?;

        // Scan all compound indexes in this workspace (both draft and published)
        for published in [false, true] {
            let prefix = keys::compound_index_workspace_prefix(
                tenant_id, repo_id, branch, workspace, published,
            );

            let prefix_clone = prefix.clone();
            let iter = crate::prefix_scan(&self.db, cf, prefix);

            for item in iter {
                let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                // Verify key actually starts with our prefix
                if !key.starts_with(&prefix_clone) {
                    break;
                }

                // Check if this key is for our node
                if let Some(key_node_id) = Self::parse_node_id_from_key(&key) {
                    if key_node_id == node_id {
                        self.db
                            .delete_cf(cf, &key)
                            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
                    }
                }
            }
        }

        tracing::debug!(
            "CompoundIndex: Removed all index entries for node '{}'",
            node_id
        );

        Ok(())
    }
}

#[cfg(test)]
#[path = "compound_index_tests.rs"]
mod tests;
