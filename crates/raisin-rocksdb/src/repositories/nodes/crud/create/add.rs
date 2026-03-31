//! Optimized node addition for brand new nodes.
//!
//! Skips existence checks for maximum throughput. Only use when you know
//! the node does not already exist.

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::{BranchScope, NodeRepository, RevisionRepository, StorageScope};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add a brand new node (optimized - no existence check)
    ///
    /// This is an optimized create function that ASSUMES the node is new.
    /// Use this when you know for certain the node doesn't exist yet.
    ///
    /// **IMPORTANT**: Do NOT use this for updates! Only for brand new nodes.
    pub(in super::super::super) async fn add_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        mut node: Node,
    ) -> Result<()> {
        let add_start = std::time::Instant::now();

        // CRITICAL: Normalize parent field from path before saving
        node.parent = Node::extract_parent_name_from_path(&node.path);

        // CRITICAL: has_children is a computed field and should NEVER be stored
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
        if let Some(parent_path) = node.parent_path() {
            if parent_path != "/" {
                if let Some(parent) = self
                    .get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                    .await?
                {
                    self.validate_parent_allows_child(
                        BranchScope::new(tenant_id, repo_id, branch),
                        &parent.node_type,
                        &node.node_type,
                    )
                    .await?;
                }
            }
        }

        // VALIDATION 3: Check unique property constraints (O(1) lookup using UNIQUE_INDEX CF)
        self.check_unique_constraints(&node, tenant_id, repo_id, branch, workspace)
            .await?;

        // ========== STEP 1: Allocate revision ==========
        let step_start = std::time::Instant::now();
        let revision = self.revision_repo.allocate_revision();
        let revision_time = step_start.elapsed().as_micros();

        // ========== STEP 2: Build WriteBatch with all indexes ==========
        let step_start = std::time::Instant::now();

        let mut batch = WriteBatch::default();

        // Use shared indexing helper (DRY - eliminates 200+ lines of duplication)
        self.add_node_indexes_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )?;

        // Add compound indexes if NodeType defines them
        self.add_compound_indexes_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        // Add unique indexes for properties marked as unique: true
        self.add_unique_indexes_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        let index_prep_time = step_start.elapsed().as_micros();

        // ========== STEP 3: Order label - OPTIMIZED (skip existence check) ==========
        let step_start = std::time::Instant::now();

        self.add_ordered_children_to_batch_fast_path(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        let order_label_time = step_start.elapsed().as_micros();

        // ========== STEP 4: Add revision indexing to batch (ATOMIC) ==========
        let step_start = std::time::Instant::now();

        self.revision_repo
            .index_node_change_to_batch(&mut batch, tenant_id, repo_id, &revision, &node.id)?;

        let updated_branch = self
            .branch_repo
            .update_head_to_batch(&mut batch, tenant_id, repo_id, branch, revision)
            .await?;

        let revision_index_time = step_start.elapsed().as_micros();

        // ========== STEP 5: RocksDB write batch (single atomic operation) ==========
        let step_start = std::time::Instant::now();

        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic write failed: {}", e)))?;

        let rocksdb_write_time = step_start.elapsed().as_micros();

        // ========== STEP 6: Capture replication events (after atomic write) ==========
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
        if self.operation_capture.is_enabled() {
            let op_type = raisin_replication::OpType::CreateNode {
                node_id: node.id.clone(),
                name: node.name.clone(),
                node_type: node.node_type.clone(),
                archetype: node.archetype.clone(),
                parent_id: node.parent.clone(),
                order_key: node.order_key.clone(),
                properties: node.properties.clone(),
                owner_id: node.owner_id.clone(),
                workspace: Some(workspace.to_string()),
                path: node.path.clone(),
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

        let total_time = add_start.elapsed().as_micros();

        if std::env::var("RAISIN_PROFILE").is_ok() {
            eprintln!(
                "ADD_TIMING node={} total={}us [rev={}us, idx={}us, ord={}us, write={}us, rev_idx={}us]",
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

    /// Fast path for adding ordered children (assumes node is new)
    async fn add_ordered_children_to_batch_fast_path(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let order_start = std::time::Instant::now();

        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // ========== SUBSTEP 1: Parent lookup ==========
        let substep_start = std::time::Instant::now();

        let parent_id_for_index = if let Some(parent_path) = node.parent_path() {
            if parent_path == "/" {
                Some("/".to_string())
            } else {
                self.get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                    .await?
                    .map(|p| p.id)
            }
        } else {
            None
        };

        let parent_lookup_time = substep_start.elapsed().as_micros();

        if let Some(ref parent_id) = parent_id_for_index {
            // ========== SUBSTEP 2: Order label calculation (FAST PATH) ==========
            let substep_start = std::time::Instant::now();

            let t = std::time::Instant::now();
            let last_label =
                self.get_last_order_label(tenant_id, repo_id, branch, workspace, parent_id)?;
            let get_last_time = t.elapsed().as_micros();

            let t = std::time::Instant::now();
            let order_label = if let Some(ref last) = last_label {
                match crate::fractional_index::inc(last) {
                    Ok(label) => label,
                    Err(e) => {
                        tracing::warn!(
                            parent_id = %parent_id,
                            last_label = %last,
                            error = %e,
                            "Corrupt order label detected, falling back to first()"
                        );
                        crate::fractional_index::first()
                    }
                }
            } else {
                crate::fractional_index::first()
            };
            let inc_time = t.elapsed().as_micros();

            let label_calc_time = substep_start.elapsed().as_micros();

            // ========== SUBSTEP 3: Batch preparation ==========
            let substep_start = std::time::Instant::now();

            let ordered_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                &order_label,
                revision,
                &node.id,
            );
            batch.put_cf(cf_ordered, ordered_key, node.name.as_bytes());

            let metadata_key =
                keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
            batch.put_cf(cf_ordered, metadata_key, order_label.as_bytes());

            let batch_prep_time = substep_start.elapsed().as_micros();

            let total_order_time = order_start.elapsed().as_micros();

            if std::env::var("RAISIN_PROFILE").is_ok() {
                eprintln!(
                    "ORDER_TIMING_FAST node={} total={}us [parent={}us, calc={}us (get_last={}us, inc={}us), batch={}us]",
                    node.name,
                    total_order_time,
                    parent_lookup_time,
                    label_calc_time,
                    get_last_time,
                    inc_time,
                    batch_prep_time
                );
            }
        }

        Ok(())
    }
}
