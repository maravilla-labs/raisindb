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
    /// * `Err` if deletion failed (note: may leave partial deletions on error)
    ///
    /// # Atomicity Note
    /// Each individual node deletion is atomic (via WriteBatch), but the entire
    /// tree deletion is NOT atomic. If an error occurs midway, some descendants
    /// may already be deleted. This is acceptable because:
    /// 1. Partial deletions don't violate referential integrity (children deleted first)
    /// 2. Retry will complete the operation (idempotent)
    /// 3. Time-travel can recover nodes from before the operation
    pub(in super::super::super) async fn delete_with_cascade(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
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

        // STEP 2: Delete root AND all descendants in ONE WriteBatch (optimal!)
        // This batch includes tombstones for nodes, path indexes, property indexes, etc.
        let deleted_descendants = self.delete_tree_with_single_batch(
            tenant_id, repo_id, branch, workspace, &node, &revision,
        )?;

        // STEP 2.5: Write unique index tombstones for root node and all descendants
        // This is separate from the main batch because add_unique_tombstones_to_batch is async
        // (requires NodeType lookup to find unique properties)
        let mut unique_batch = WriteBatch::default();

        // Root node unique tombstones
        self.add_unique_tombstones_to_batch(
            &mut unique_batch,
            &node,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &revision,
        )
        .await?;

        // Descendant unique tombstones
        for deleted_node in &deleted_descendants {
            self.add_unique_tombstones_to_batch(
                &mut unique_batch,
                deleted_node,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &revision,
            )
            .await?;
        }

        // Write unique tombstones
        if !unique_batch.is_empty() {
            self.db.write(unique_batch).map_err(|e| {
                raisin_error::Error::storage(format!("Unique tombstone write failed: {}", e))
            })?;
        }

        // STEP 3: Index all node changes AND update branch HEAD in a SECOND atomic batch
        // This ensures revision tracking is atomic even if separate from tombstones
        let mut revision_batch = rocksdb::WriteBatch::default();

        // Index all descendant node deletions
        for deleted_node in &deleted_descendants {
            self.revision_repo.index_node_change_to_batch(
                &mut revision_batch,
                tenant_id,
                repo_id,
                &revision,
                &deleted_node.id,
            )?;
        }
        // Index the root node deletion
        self.revision_repo.index_node_change_to_batch(
            &mut revision_batch,
            tenant_id,
            repo_id,
            &revision,
            node_id,
        )?;

        // Add branch HEAD update to the same atomic batch
        let updated_branch = self
            .branch_repo
            .update_head_to_batch(&mut revision_batch, tenant_id, repo_id, branch, revision)
            .await?;

        // Write revision indexes + branch HEAD atomically
        self.db.write(revision_batch).map_err(|e| {
            raisin_error::Error::storage(format!("Atomic revision index failed: {}", e))
        })?;

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

        // Capture DeleteNode operations for replication
        if self.operation_capture.is_enabled() {
            let actor = "system".to_string(); // Cascade deletes are system operations

            // Capture delete for root node
            let _ = self
                .operation_capture
                .capture_delete_node(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    branch.to_string(),
                    node_id.to_string(),
                    actor.clone(),
                )
                .await;

            // Capture delete for each descendant
            for deleted_node in &deleted_descendants {
                let _ = self
                    .operation_capture
                    .capture_delete_node(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch.to_string(),
                        deleted_node.id.clone(),
                        actor.clone(),
                    )
                    .await;
            }
        }

        Ok(true)
    }

    /// Delete all descendants of a node using iterative prefix scan with single revision
    ///
    /// This is a helper for cascade delete that uses RocksDB prefix iteration
    /// instead of recursion. It scans all descendants and deletes them using a
    /// SINGLE WriteBatch for optimal performance (one atomic commit for entire tree).
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `parent_id` - The ID of the parent whose children should be deleted
    /// * `revision` - The single revision to use for all deletions
    ///
    /// # Returns
    /// * `Ok(Vec<Node>)` - All deleted nodes
    /// * `Err` if deletion failed
    ///
    /// # Performance
    /// - O(N) where N = total descendants
    /// - ONE WriteBatch for entire tree (atomic)
    /// - ONE db.write() call for all tombstones
    pub(in super::super::super) fn delete_descendants_with_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
        revision: &HLC,
    ) -> Result<Vec<Node>> {
        // Use prefix scan to collect all descendants (NO RECURSION!)
        let descendants = self.scan_descendants_ordered_impl(
            tenant_id, repo_id, branch, workspace, parent_id, None,
        )?;

        // CRITICAL: Single WriteBatch for ALL descendants (one atomic commit!)
        let mut batch = WriteBatch::default();
        let mut deleted_nodes = Vec::new();

        // Get column family handles once
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_compound = cf_handle(&self.db, cf::COMPOUND_INDEX)?;
        let cf_spatial = cf_handle(&self.db, cf::SPATIAL_INDEX)?;

        // Process all descendants (excluding root itself)
        // ALL tombstones added to SAME batch!
        for (node, _depth) in descendants.into_iter() {
            // Skip the root node itself (we only delete descendants)
            if node.id == parent_id {
                continue;
            }

            // Add all tombstones to the batch using shared logic
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

        // Atomic commit of all deletions with SAME revision
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(deleted_nodes)
    }

    /// Delete root node AND all descendants in SINGLE WriteBatch (optimal!)
    ///
    /// This combines the root node deletion and all descendant deletions into
    /// ONE atomic WriteBatch commit for maximum performance.
    ///
    /// # Arguments
    /// * `tenant_id`, `repo_id`, `branch`, `workspace` - Context for the operation
    /// * `root_node` - The root node to delete (already fetched)
    /// * `revision` - The single revision to use for all deletions
    ///
    /// # Returns
    /// * `Ok(Vec<Node>)` - All deleted descendants (NOT including root)
    /// * `Err` if deletion failed
    ///
    /// # Performance
    /// - O(N) where N = total descendants + root
    /// - ONE WriteBatch for entire tree (fully atomic)
    /// - ONE db.write() call for all tombstones
    pub(in super::super::super) fn delete_tree_with_single_batch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        root_node: &Node,
        revision: &HLC,
    ) -> Result<Vec<Node>> {
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

        // STEP 5: ONE atomic commit for entire tree!
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(deleted_nodes)
    }
}
