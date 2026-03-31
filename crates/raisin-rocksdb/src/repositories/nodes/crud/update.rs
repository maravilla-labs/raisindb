//! Node update operations
//!
//! This module contains node update operations - modifying existing nodes.
//! For creating new nodes, see create.rs

use super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{BranchScope, NodeRepository, RevisionRepository, StorageScope};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Update an existing node
    ///
    /// This function is specifically for updating nodes that already exist.
    /// It will fail if the node doesn't exist (unlike put_impl which handled both create and update).
    ///
    /// **IMPORTANT**: Only use this for updates. For creating new nodes, use add_impl.
    pub(in super::super) async fn update_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        mut node: Node,
    ) -> Result<()> {
        let update_start = std::time::Instant::now();

        // CRITICAL: Normalize parent field from path before saving
        // Parent should NEVER be null - it's either "/" for root-level nodes or the parent's name
        node.parent = Node::extract_parent_name_from_path(&node.path);

        // CRITICAL: has_children is a computed field and should NEVER be stored
        // It's only populated at the service layer for API responses
        node.has_children = None;

        // VALIDATION 1: Check workspace allowed_node_types
        let is_root_node = node.parent_path().map(|p| p == "/").unwrap_or(false);
        self.validate_workspace_allows_node_type(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            &node.node_type,
            is_root_node,
        )
        .await?;

        // VALIDATION 2: Check NodeType.allowed_children if node has a parent
        // This is opportunistic: only validate if parent exists, allowing flexible node creation order
        if let Some(parent_path) = node.parent_path() {
            if parent_path != "/" {
                // Try to get parent node - if it exists, validate allowed_children
                if let Some(parent) = self
                    .get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                    .await?
                {
                    // Validate that parent's NodeType allows this child's NodeType
                    self.validate_parent_allows_child(
                        BranchScope::new(tenant_id, repo_id, branch),
                        &parent.node_type,
                        &node.node_type,
                    )
                    .await?;
                }
                // If parent doesn't exist yet, skip validation - allows flexible creation order
            }
        }

        // VALIDATION 3: Verify node exists and get old node for unique constraint handling
        // For updates, we need the old node to:
        // - Ensure we're not accidentally creating a new node
        // - Write tombstones for changed unique property values
        let old_node = match self
            .get_impl(tenant_id, repo_id, branch, workspace, &node.id, false)
            .await?
        {
            Some(n) => n,
            None => {
                return Err(raisin_error::Error::NotFound(format!(
                    "Cannot update node '{}' - node does not exist. Use create/add instead.",
                    node.id
                )));
            }
        };

        // VALIDATION 4: Check unique property constraints (O(1) lookup using UNIQUE_INDEX CF)
        // This allows the same node to keep its unique values (no conflict with itself)
        self.check_unique_constraints(&node, tenant_id, repo_id, branch, workspace)
            .await?;

        // ========== STEP 1: Allocate revision ==========
        let step_start = std::time::Instant::now();
        let revision = self.revision_repo.allocate_revision();
        let revision_time = step_start.elapsed().as_micros();

        // ========== STEP 2: Build WriteBatch with all indexes ==========
        let step_start = std::time::Instant::now();

        let mut batch = WriteBatch::default();

        // Use shared indexing helper (DRY)
        self.add_node_indexes_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )?;

        // Add compound indexes if NodeType defines them
        self.add_compound_indexes_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        // Handle unique index updates:
        // 1. Write tombstones for unique values that have changed (old values)
        // 2. Write new unique index entries for current values
        // Note: We compare old vs new to only tombstone changed values, but for simplicity
        // we write tombstones for ALL old unique values and write new entries for ALL new values.
        // The tombstone mechanism ensures this is correct even if values haven't changed.
        self.add_unique_tombstones_to_batch(
            &mut batch, &old_node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;
        self.add_unique_indexes_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        let index_prep_time = step_start.elapsed().as_micros();

        // ========== STEP 3: Order label calculation ==========
        let step_start = std::time::Instant::now();

        // Add ORDERED_CHILDREN index entry using shared helper
        // This will preserve the existing order label for updates
        self.add_ordered_children_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        let order_label_time = step_start.elapsed().as_micros();

        // ========== STEP 4: Add revision indexing to batch (ATOMIC) ==========
        let step_start = std::time::Instant::now();

        // Add revision index to the same atomic batch
        self.revision_repo
            .index_node_change_to_batch(&mut batch, tenant_id, repo_id, &revision, &node.id)?;

        // Add branch HEAD update to the same atomic batch
        let updated_branch = self
            .branch_repo
            .update_head_to_batch(&mut batch, tenant_id, repo_id, branch, revision)
            .await?;

        let revision_index_time = step_start.elapsed().as_micros();

        // ========== STEP 5: RocksDB write batch (single atomic operation) ==========
        let step_start = std::time::Instant::now();

        // Atomic commit - all operations succeed or fail together
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic write failed: {}", e)))?;

        let rocksdb_write_time = step_start.elapsed().as_micros();

        // ========== STEP 6: Capture replication events (after atomic write) ==========
        // Capture branch HEAD update for replication
        self.branch_repo
            .capture_head_update_for_replication(
                tenant_id,
                repo_id,
                branch,
                &updated_branch,
                revision,
            )
            .await;

        // ========== STEP 7: Capture operation for replication ==========
        // update_impl is always an update (SetProperty operations for each changed property)
        if self.operation_capture.is_enabled() {
            // Capture property changes as SetProperty operations
            for (prop_name, prop_value) in &node.properties {
                let op_type = raisin_replication::OpType::SetProperty {
                    node_id: node.id.clone(),
                    property_name: prop_name.clone(),
                    value: prop_value.clone(),
                };

                let _ = self
                    .operation_capture
                    .capture_operation_with_revision(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch.to_string(),
                        op_type,
                        "system".to_string(),
                        None,
                        true,
                        Some(revision),
                    )
                    .await;
            }
        }

        let total_time = update_start.elapsed().as_micros();

        // Log detailed timing breakdown
        tracing::debug!(
            "UPDATE_TIMING node={} total={}μs [rev={}μs, idx={}μs, ord={}μs, write={}μs, rev_idx={}μs]",
            node.name,
            total_time,
            revision_time,
            index_prep_time,
            order_label_time,
            rocksdb_write_time,
            revision_index_time
        );

        if std::env::var("RAISIN_PROFILE").is_ok() {
            eprintln!(
                "UPDATE_TIMING node={} total={}μs [rev={}μs, idx={}μs, ord={}μs, write={}μs, rev_idx={}μs]",
                node.name,
                total_time,
                revision_time,
                index_prep_time,
                order_label_time,
                rocksdb_write_time,
                revision_index_time
            );
        }

        Ok(())
    }
}
