//! Tree and cascade deletion operations
//!
//! This module contains functions for deleting entire node trees efficiently.
//! It uses optimized batch operations to delete a root node and all its
//! descendants in a single atomic WriteBatch.

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::RevisionRepository;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Delete a node and all its descendants recursively
    ///
    /// This performs an efficient tree deletion using a single WriteBatch
    /// and a single revision for the entire operation.
    ///
    /// # Algorithm
    /// 1. Check if node exists and verify referential integrity
    /// 2. Allocate SINGLE revision for entire tree deletion
    /// 3. Delete root AND all descendants in ONE WriteBatch (optimal!)
    /// 4. Index all node changes with SAME revision
    /// 5. Update branch HEAD to the new revision
    ///
    /// # Performance
    /// - For N total descendants: O(N) deletions
    /// - ONE WriteBatch for entire tree (atomic)
    /// - ONE db.write() call for all tombstones
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `node_id` - The ID of the root node to delete
    ///
    /// # Returns
    /// * `Ok(true)` if node and descendants were deleted
    /// * `Ok(false)` if node didn't exist
    /// * `Err` if deletion failed (nothing was written)
    ///
    /// # Atomicity
    /// Tombstones, unique claims, the revision index and the HEAD advance are
    /// ONE batch, written as one node commit step (plan Phase 7b).
    pub(in super::super::super) async fn delete_with_cascade(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        attribution: crate::repositories::nodes::WriteAttribution<'_>,
    ) -> Result<bool> {
        // Check if node exists
        let node = self
            .get_impl(tenant_id, repo_id, branch, workspace, node_id, false)
            .await?;

        // Early return if node doesn't exist
        let node = match node {
            Some(n) => n,
            None => return Ok(false),
        };

        // Check referential integrity - prevent deletion if other nodes reference this node
        self.check_delete_safety(tenant_id, repo_id, branch, workspace, node_id)
            .await?;

        // STEP 1: Allocate SINGLE revision for entire tree deletion
        let revision = self.revision_repo.allocate_revision();

        // STEP 2: Tombstones for the root AND all descendants, their unique
        // claims and the revision index, in ONE WriteBatch — written once,
        // with the HEAD advance, as one commit step (plan Phase 7b): every
        // deleted node locked, and its tombstones re-derived against what is
        // stored at that moment (the descendants were read by a scan, so
        // there is no per-node "before the read" to record — `always`).
        let (mut batch, deleted_descendants) =
            self.stage_tree_delete(tenant_id, repo_id, branch, workspace, &node, &revision)?;

        // Unique claims (async — needs the NodeType's unique properties).
        self.add_unique_tombstones_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;
        for deleted_node in &deleted_descendants {
            self.add_unique_tombstones_to_batch(
                &mut batch,
                deleted_node,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &revision,
            )
            .await?;
        }

        // STEP 3: Index every node change with the same revision.
        for deleted_node in &deleted_descendants {
            self.revision_repo.index_node_change_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                &revision,
                &deleted_node.id,
            )?;
        }
        self.revision_repo
            .index_node_change_to_batch(&mut batch, tenant_id, repo_id, &revision, node_id)?;

        let ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
        let mut commit = crate::indexing::NodeCommit::new(tenant_id, repo_id, branch);
        for deleted in std::iter::once(&node).chain(deleted_descendants.iter()) {
            commit.check(
                crate::indexing::StagedDeltaCheck::always(&ctx, &deleted.id, &revision),
                None,
            );
        }
        let updated_branch = self
            .branch_repo
            .write_nodes_with_head(batch, tenant_id, repo_id, branch, revision, &commit)
            .await?;

        // STEP 3.5: Capture replication events (after atomic write)
        self.branch_repo
            .capture_head_update_for_replication(
                tenant_id,
                repo_id,
                branch,
                &updated_branch,
                revision,
            )
            .await;

        // Capture one ApplyRevision snapshot covering the whole deleted tree
        // (root + descendants), so peers tombstone every index family from the
        // full pre-delete node state instead of a bare node id.
        let mut changes = Vec::with_capacity(1 + deleted_descendants.len());
        changes.push((
            node.clone(),
            raisin_replication::operation::ReplicatedNodeChangeKind::Delete,
        ));
        for deleted_node in &deleted_descendants {
            changes.push((
                deleted_node.clone(),
                raisin_replication::operation::ReplicatedNodeChangeKind::Delete,
            ));
        }
        self.capture_apply_revision_snapshot(
            tenant_id,
            repo_id,
            branch,
            workspace,
            changes,
            revision,
            attribution,
        )
        .await;

        // Emit `node:deleted` for the root and every descendant, through the
        // same emitter as the non-cascade path (see `publish_deleted_event`).
        self.publish_deleted_event(
            tenant_id,
            repo_id,
            branch,
            workspace,
            revision,
            &node,
            attribution,
        );
        for deleted_node in &deleted_descendants {
            self.publish_deleted_event(
                tenant_id,
                repo_id,
                branch,
                workspace,
                revision,
                deleted_node,
                attribution,
            );
        }

        Ok(true)
    }

    /// Stage the tombstones of `root_node` AND all its descendants into ONE
    /// WriteBatch, at one revision. Nothing is written: the caller commits it
    /// through the node commit step (plan Phase 7b).
    ///
    /// # Returns
    /// The batch and every deleted descendant (NOT including the root).
    pub(in super::super::super) fn stage_tree_delete(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        root_node: &Node,
        revision: &HLC,
    ) -> Result<(WriteBatch, Vec<Node>)> {
        // STEP 1: Scan all descendants (not including root)
        let descendants = self.scan_descendants_ordered_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &root_node.id,
            None,
        )?;

        // STEP 2: Create SINGLE WriteBatch for entire tree
        let mut batch = WriteBatch::default();

        // Get column family handles once
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_compound = cf_handle(&self.db, cf::COMPOUND_INDEX)?;
        let cf_spatial = cf_handle(&self.db, cf::SPATIAL_INDEX)?;

        // STEP 3: Add root node tombstones to batch FIRST
        self.add_node_tombstones_to_batch(
            &mut batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            root_node,
            revision,
            cf_nodes,
            cf_path,
            cf_property,
            cf_relation,
            cf_ordered,
            cf_node_path,
            cf_compound,
            cf_spatial,
        )?;

        // STEP 4: Add all descendant tombstones to SAME batch
        let mut deleted_nodes = Vec::new();
        for (node, _depth) in descendants.into_iter() {
            // Skip the root itself (already added above)
            if node.id == root_node.id {
                continue;
            }

            self.add_node_tombstones_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node,
                revision,
                cf_nodes,
                cf_path,
                cf_property,
                cf_relation,
                cf_ordered,
                cf_node_path,
                cf_compound,
                cf_spatial,
            )?;

            deleted_nodes.push(node);
        }

        Ok((batch, deleted_nodes))
    }
}
