//! Shared reorder logic for child ordering operations
//!
//! This module contains the core reorder implementation that is used by all
//! three ordering operations: reorder_child, move_child_before, and move_child_after.

use super::super::helpers::TOMBSTONE;
use super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_storage::{BranchRepository, RevisionRepository};

impl NodeRepositoryImpl {
    /// Shared reorder logic that handles the atomic write operation
    ///
    /// This function contains all the boilerplate code that was duplicated across
    /// the three ordering operations. It accepts a pre-calculated new_label and
    /// performs the atomic reorder operation.
    ///
    /// # Parameters
    /// - `new_label`: The pre-calculated order label for the child's new position
    /// - Other parameters identify the child and provide revision metadata
    pub(in crate::repositories::nodes) async fn reorder_child_with_label(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        target_child_id: &str,
        child_name: &str,
        new_label: String,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<()> {
        // 1. Check for fractional index exhaustion
        if crate::fractional_index::is_approaching_exhaustion(&new_label) {
            tracing::warn!(
                label = %new_label,
                length = new_label.len(),
                parent_id = %parent_id,
                tenant = %tenant_id,
                repo = %repo_id,
                threshold = crate::fractional_index::WARNING_LENGTH,
                "Order label approaching exhaustion. Consider rebalancing parent's children."
            );
        }

        // 2. Get old label for tombstone (revision isolation)
        let old_label = self.get_order_label_for_child(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            target_child_id,
        )?;
        let old_label_for_meta = old_label.clone();

        // 3. Get parent revision for operation metadata
        let parent_revision = self
            .branch_repo
            .get_head(tenant_id, repo_id, branch)
            .await?;

        // 4. Allocate new revision
        let revision = self.revision_repo.allocate_revision();

        // 4b. Append HLC timestamp to new_label for causal ordering
        // This ensures deterministic ordering across cluster when multiple nodes
        // generate the same fractional part concurrently
        let final_label = super::format_order_label(&new_label, &revision);

        // 5. ATOMIC WRITE using WriteBatch
        let mut batch = rocksdb::WriteBatch::default();
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // Write tombstone for old position (revision isolation - never delete old entries!)
        if let Some(old_label) = old_label {
            let tombstone_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                &old_label,
                &revision,
                target_child_id,
            );
            batch.put_cf(cf_ordered, tombstone_key, TOMBSTONE);
        }

        // Write new ordered index entry (store child name in value for efficient lookups)
        let new_key = keys::ordered_child_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            &final_label,
            &revision,
            target_child_id,
        );
        batch.put_cf(cf_ordered, new_key, child_name.as_bytes());

        // Smart metadata cache update: only update if final_label is lexicographically last
        let metadata_key =
            keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);

        let is_last = self.is_lexicographically_last_label(
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            &final_label,
            Some(target_child_id),
        )?;

        if is_last {
            // final_label is the lexicographically last - update cache
            batch.put_cf(cf_ordered, metadata_key, final_label.as_bytes());
        } else {
            // final_label is not the last - invalidate cache to force rescan on next append
            batch.delete_cf(cf_ordered, metadata_key);
        }

        // 5b. Persist the new label onto the node record, in the SAME batch and
        // at the SAME revision.
        //
        // `Node.order_key` is the value every read surface reports (JSON, the
        // `__order` SQL column, replication's fallback order key). Before this,
        // reorder only wrote ORDERED_CHILDREN, so the stored `order_key` silently
        // went stale the moment a node was moved — the CF and the node record
        // disagreed, and the delete path had to compensate (see
        // `crud/delete/tombstone.rs`).
        //
        // This writes NODES / PATH_INDEX / NODE_PATH / property indexes at the
        // already-allocated revision. It deliberately does NOT go through
        // `update_impl`, which would allocate a second revision, bump HEAD again,
        // and append another ORDERED_CHILDREN entry.
        let reordered_node = match self
            .get_impl(
                tenant_id,
                repo_id,
                branch,
                workspace,
                target_child_id,
                false,
            )
            .await?
        {
            Some(mut node) => {
                node.order_key = final_label.clone();
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
                Some(node)
            }
            None => {
                // The order index can outlive the node (concurrent delete). The
                // ordering write above is still correct and idempotent, so don't
                // fail the reorder over a missing node record.
                tracing::warn!(
                    node_id = %target_child_id,
                    parent_id = %parent_id,
                    "Reorder: node record not found; ordering index updated without it"
                );
                None
            }
        };

        // Commit batch atomically
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to reorder child: {}", e)))?;

        // 6. Update branch HEAD
        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        // 6b. Replicate the reordered node.
        //
        // Must run AFTER the commit: the capture helper re-reads the committed
        // ORDERED_CHILDREN entry to resolve `cf_order_key`, so peers reproduce
        // the exact index entry rather than inferring one.
        if let Some(node) = reordered_node {
            self.capture_apply_revision_snapshot(
                tenant_id,
                repo_id,
                branch,
                workspace,
                vec![(
                    node,
                    raisin_replication::operation::ReplicatedNodeChangeKind::Upsert,
                )],
                revision,
                // Same actor the revision metadata below records, so the peer's
                // replay attributes the reorder to the user who made it instead
                // of to "system".
                crate::repositories::nodes::WriteAttribution::actor(actor),
            )
            .await;
        }

        // 7. Store revision metadata with commit message
        if message.is_some() || actor.is_some() {
            let op_meta = raisin_models::operations::OperationMeta::new_reorder(
                target_child_id.to_string(),
                old_label_for_meta.unwrap_or_else(|| "unknown".to_string()),
                final_label.clone(),
                &revision,
                Some(&parent_revision),
                actor.unwrap_or("system").to_string(),
                message.unwrap_or("Reorder operation").to_string(),
            );

            // Record the reordered child so it surfaces in branch diffs (e.g. the
            // admin-console merge "Reordered" group). Conflict detection skips
            // Reordered entries, so this does not create spurious merge conflicts.
            let changed_nodes = vec![raisin_storage::NodeChangeInfo {
                node_id: target_child_id.to_string(),
                workspace: workspace.to_string(),
                operation: raisin_models::tree::ChangeOperation::Reordered,
                translation_locale: None,
            }];

            let rev_meta = raisin_storage::RevisionMeta {
                revision,
                parent: Some(parent_revision),
                merge_parent: None,
                branch: branch.to_string(),
                timestamp: op_meta.timestamp,
                actor: op_meta.actor.clone(),
                message: op_meta.message.clone(),
                is_system: op_meta.is_system,
                changed_nodes,
                changed_node_types: Vec::new(),
                changed_archetypes: Vec::new(),
                changed_element_types: Vec::new(),
                operation: Some(op_meta),
            };

            self.revision_repo
                .store_revision_meta(tenant_id, repo_id, rev_meta)
                .await?;
        }

        Ok(())
    }
}
