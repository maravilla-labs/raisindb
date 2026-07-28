//! Self-healing rebalance for child ordering.
//!
//! Inter-branch copy and three-way merge copy `ORDERED_CHILDREN` entries
//! verbatim (same `order_label`, only the branch name changes). When two
//! branches independently mint the *same* fractional `order_label` under one
//! parent — which is common after a fork, since both sides start appending
//! from the same base — merging them leaves two distinct children sharing one
//! fractional label. The next reorder then asks `fractional_index::between` to
//! insert between two equal labels and gets the
//! "First label must be less than second label" error from [`crate::fractional_index::mid`].
//!
//! This module detects that condition and repairs it by reassigning a clean,
//! strictly-increasing sequence of fractional labels to the parent's children
//! **in their current visible order** (a relabel, not a reorder — the order the
//! user sees is preserved). All children are rewritten in a single atomic batch
//! under one revision, so every new label shares the same `::HLC` suffix and
//! ordering is decided purely by the fractional part.

use super::super::helpers::{is_tombstone, TOMBSTONE};
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_storage::{BranchRepository, RevisionRepository};
use std::collections::HashSet;

/// A child as it currently lives in the ordered index.
struct LiveChild {
    child_id: String,
    /// Child name, stored as the index value.
    name: Vec<u8>,
    /// Current full order label (`{fractional}::{HLC}`) the child lives at.
    full_label: String,
}

impl NodeRepositoryImpl {
    /// Collect the current (live, non-tombstoned) children of a parent, in the
    /// same label order used everywhere else (mirrors `get_ordered_child_ids`).
    ///
    /// Each child appears once, at its current label, with the name stored in
    /// the index value.
    fn collect_live_children(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Result<Vec<LiveChild>> {
        let prefix =
            keys::ordered_children_prefix(tenant_id, repo_id, branch, workspace, parent_id);
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf_ordered, prefix);

        // Track (label, child_id) to skip older revisions, and child_id to skip
        // a child already emitted at an earlier (lower) label.
        let mut seen_entries: HashSet<(String, String)> = HashSet::new();
        let mut seen_child_ids: HashSet<String> = HashSet::new();
        let mut children: Vec<LiveChild> = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix_clone) {
                break;
            }

            // Key: {prefix(6 nulls)}{order_label}\0{~HLC-16}\0{child_id}
            // HLC can contain null bytes, so we walk to the 6th null explicitly
            // rather than splitting the whole key.
            let mut null_count = 0;
            let mut prefix_end = 0;
            for (i, &byte) in key.iter().enumerate() {
                if byte == 0 {
                    null_count += 1;
                    if null_count == 6 {
                        prefix_end = i + 1;
                        break;
                    }
                }
            }
            if null_count < 6 {
                continue;
            }

            let after_prefix = &key[prefix_end..];
            let order_label_end = match after_prefix.iter().position(|&b| b == 0) {
                Some(pos) => pos,
                None => continue,
            };
            let order_label = String::from_utf8_lossy(&after_prefix[..order_label_end]).to_string();

            // Skip metadata entries (e.g. the cached LAST label).
            if order_label.starts_with('\u{FFFD}')
                || order_label.contains("META")
                || order_label.contains("LAST")
                || order_label.contains("FIRST")
            {
                continue;
            }

            let hlc_start = order_label_end + 1;
            if after_prefix.len() < hlc_start + 16 {
                continue;
            }
            let child_id_start = hlc_start + 16 + 1;
            if after_prefix.len() < child_id_start {
                continue;
            }
            let child_id = String::from_utf8_lossy(&after_prefix[child_id_start..]).to_string();

            let entry_key = (order_label.clone(), child_id.clone());
            if seen_entries.contains(&entry_key) {
                continue;
            }
            seen_entries.insert(entry_key);

            if is_tombstone(&value) {
                continue;
            }

            if seen_child_ids.contains(&child_id) {
                continue;
            }
            seen_child_ids.insert(child_id.clone());

            children.push(LiveChild {
                child_id,
                name: value.to_vec(),
                full_label: order_label,
            });
        }

        Ok(children)
    }

    /// Returns true if the parent's ordering is broken in a way that would make
    /// `fractional_index::between` fail on the next insert: two children share a
    /// fractional label, or the fractional labels are not strictly increasing in
    /// visible order.
    pub(crate) fn parent_ordering_is_corrupt(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Result<bool> {
        let children =
            self.collect_live_children(tenant_id, repo_id, branch, workspace, parent_id)?;

        let mut prev: Option<&str> = None;
        for child in &children {
            let frac = crate::fractional_index::extract_fractional(&child.full_label);
            if let Some(prev_frac) = prev {
                // Equal or inverted -> mid() would reject an insert between them.
                if prev_frac >= frac {
                    return Ok(true);
                }
            }
            prev = Some(frac);
        }

        Ok(false)
    }

    /// Reassign clean, strictly-increasing fractional labels to all children of
    /// a parent, preserving their current visible order. Atomic: one revision,
    /// one write batch.
    ///
    /// The caller is expected to already hold the per-parent ordering lock.
    pub(crate) async fn rebalance_children(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        actor: Option<&str>,
    ) -> Result<()> {
        let children =
            self.collect_live_children(tenant_id, repo_id, branch, workspace, parent_id)?;

        if children.len() < 2 {
            // Nothing to rebalance (0 or 1 child can never collide).
            return Ok(());
        }

        let parent_revision = self
            .branch_repo
            .get_head(tenant_id, repo_id, branch)
            .await?;
        let revision = self.revision_repo.allocate_revision();

        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let mut batch = rocksdb::WriteBatch::default();

        // Assign first(), inc(first()), inc(...)... — the same sequence normal
        // appends use, so the labels are guaranteed prefix-free and correctly
        // ordered. A single shared HLC suffix means ordering is purely fractional.
        let mut fractional = crate::fractional_index::first();
        let mut last_full_label = String::new();
        // Nodes whose record we rewrote, for the post-commit replication capture.
        let mut relabelled_nodes = Vec::with_capacity(children.len());

        for (idx, child) in children.iter().enumerate() {
            if idx > 0 {
                fractional = crate::fractional_index::inc(&fractional)?;
            }
            let new_full_label = super::format_order_label(&fractional, &revision);

            // Tombstone the child's current entry (revision isolation - keep history).
            let tombstone_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                &child.full_label,
                &revision,
                &child.child_id,
            );
            batch.put_cf(cf_ordered, tombstone_key, TOMBSTONE);

            // Write the new entry (value = child name, as elsewhere).
            let new_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                &new_full_label,
                &revision,
                &child.child_id,
            );
            batch.put_cf(cf_ordered, new_key, &child.name);

            // Rebalancing changes every child's label, so every child's node
            // record has to be rewritten too — otherwise the stored `order_key`
            // (and the `__order` column that reads it) would still report the
            // pre-rebalance labels. Same batch, same revision.
            if let Some(mut node) = self
                .get_impl(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &child.child_id,
                    false,
                )
                .await?
            {
                node.order_key = new_full_label.clone();
                node.updated_at = Some(chrono::Utc::now());
                self.add_node_indexes_to_batch_with_parent_id(
                    &mut batch,
                    &node,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &revision,
                    Some(parent_id.to_string()),
                )?;
                relabelled_nodes.push((
                    node,
                    raisin_replication::operation::ReplicatedNodeChangeKind::Upsert,
                ));
            }

            last_full_label = new_full_label;
        }

        // The highest label is now the last child's label.
        let metadata_key =
            keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
        batch.put_cf(cf_ordered, metadata_key, last_full_label.as_bytes());

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to rebalance children: {}", e))
        })?;

        // Advance HEAD so the new entries are visible to revision-filtered reads.
        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        // Replicate the relabelled nodes. After the commit, so the capture helper
        // resolves each child's committed CF order key.
        self.capture_apply_revision_snapshot(
            tenant_id,
            repo_id,
            branch,
            workspace,
            relabelled_nodes,
            revision,
            // Matches the actor stamped on the rebalance revision metadata below.
            crate::repositories::nodes::WriteAttribution::actor(actor),
        )
        .await;

        // Record a system commit for the relabel.
        let op_meta = raisin_models::operations::OperationMeta::new_reorder(
            parent_id.to_string(),
            "rebalance".to_string(),
            "rebalance".to_string(),
            &revision,
            Some(&parent_revision),
            actor.unwrap_or("system").to_string(),
            format!("Rebalance ordering for {} children", children.len()),
        );
        let rev_meta = raisin_storage::RevisionMeta {
            revision,
            parent: Some(parent_revision),
            merge_parent: None,
            branch: branch.to_string(),
            timestamp: op_meta.timestamp,
            actor: op_meta.actor.clone(),
            message: op_meta.message.clone(),
            is_system: true,
            changed_nodes: vec![],
            changed_node_types: Vec::new(),
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation: Some(op_meta),
        };
        self.revision_repo
            .store_revision_meta(tenant_id, repo_id, rev_meta)
            .await?;

        tracing::info!(
            parent_id = %parent_id,
            branch = %branch,
            count = children.len(),
            "Rebalanced child ordering (self-heal)"
        );

        Ok(())
    }

    /// Rebalance the parent's children only if their ordering is corrupt.
    /// Returns true if a rebalance was performed.
    ///
    /// The caller is expected to already hold the per-parent ordering lock.
    pub(crate) async fn heal_ordering_if_corrupt(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        actor: Option<&str>,
    ) -> Result<bool> {
        if self.parent_ordering_is_corrupt(tenant_id, repo_id, branch, workspace, parent_id)? {
            self.rebalance_children(tenant_id, repo_id, branch, workspace, parent_id, actor)
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
