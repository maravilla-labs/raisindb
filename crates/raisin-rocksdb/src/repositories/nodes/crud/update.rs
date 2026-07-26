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

        // Stamp modification time at this write level too — mirrors the
        // transaction-layer put_node stamping so non-transactional updates
        // (e.g. property updates by path) also record when they happened.
        // Never let an incoming None wipe the original creation metadata.
        // (updated_by cannot be resolved here: the repository layer has no
        // actor; put_node handles that where an auth context exists.)
        node.updated_at = Some(chrono::Utc::now());
        if node.created_at.is_none() {
            node.created_at = old_node.created_at;
        }
        if node.created_by.is_none() {
            node.created_by = old_node.created_by.clone();
        }

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

        // ORDERED_CHILDREN must be maintained BEFORE the node blob is
        // serialized: it stamps `node.order_key` (preserving the existing label
        // on update, appending a new one otherwise), and the blob has to carry
        // the same label the index entry does.
        let order_step_start = std::time::Instant::now();
        self.add_ordered_children_to_batch(
            &mut batch, &mut node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;
        let order_label_time = order_step_start.elapsed().as_micros();

        // Property index: tombstone stale OLD-value entries (value changed /
        // property removed / published tag flipped) BEFORE writing the new
        // entries. Without this, equality scans and index COUNTs on the old
        // value keep matching this node forever (orphaned entries even survive
        // restarts).
        {
            let cf_property = crate::cf_handle(&self.db, crate::cf::PROPERTY_INDEX)?;
            super::indexing::property_indexes::add_stale_property_tombstones(
                &mut batch,
                cf_property,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &old_node,
                &node,
                &revision,
            );
        }

        // Reference index: tombstone stale OLD entries (reference removed /
        // retargeted) — otherwise REFERENCES()/backlinks keep matching this
        // node against the old target forever.
        self.add_stale_reference_tombstones_to_batch(
            &mut batch, &old_node, &node, tenant_id, repo_id, branch, workspace, &revision,
        )?;

        // Use shared indexing helper (DRY)
        self.add_node_indexes_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )?;

        // Compound indexes: tombstone the OLD value entries first, then write the
        // new ones. Without the tombstone, a column value change (e.g. status
        // held -> confirmed) would leave the stale old-value entry live and a scan
        // keyed on the old value would still return this node.
        self.add_compound_tombstones_to_batch(
            &mut batch, &old_node, tenant_id, repo_id, branch, workspace,
        )?;
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
        // Full-snapshot ApplyRevision (like the transaction commit path).
        // Per-property SetProperty ops cannot express removed properties or
        // path/name/type changes, so peers would drift.
        self.capture_apply_revision_snapshot(
            tenant_id,
            repo_id,
            branch,
            workspace,
            vec![(
                node.clone(),
                raisin_replication::operation::ReplicatedNodeChangeKind::Upsert,
            )],
            revision,
        )
        .await;

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
