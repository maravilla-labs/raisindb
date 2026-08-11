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
        attribution: crate::repositories::nodes::WriteAttribution<'_>,
    ) -> Result<()> {
        let add_start = std::time::Instant::now();

        // CRITICAL: Normalize parent field from path before saving
        node.parent = Node::extract_parent_name_from_path(&node.path);

        // CRITICAL: has_children is a computed field and should NEVER be stored
        node.has_children = None;

        // Mirrors the transaction-layer stamping in add_node/put_node (see
        // Node::ensure_write_timestamps for why every write path must do this).
        node.ensure_write_timestamps();

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

        // ========== Vault `encrypted` schema fields ==========
        //
        // AFTER validation (which must see plaintext for its constraints to
        // mean anything) and BEFORE the WriteBatch below, which serializes the
        // node blob and writes the property / unique / compound index entries.
        // An index entry keyed on a plaintext password turns
        // `properties->>'password'::String = '<guess>'` into a working oracle,
        // permanently — the entries carry the revision, so nothing rewrites
        // them later.
        //
        // No memo: two repository calls are two logical writes with two
        // revisions, so each SHOULD mint its own secret version. The
        // transaction path passes one because its writes share an HLC.
        // See `crate::vaulting`.
        let vault_actor = node
            .updated_by
            .clone()
            .unwrap_or_else(|| "anonymous".to_string());
        let minted_secrets = self
            .vault_encrypted_fields(
                crate::vaulting::VaultScope {
                    tenant_id,
                    repo_id,
                    branch,
                    actor: &vault_actor,
                },
                &mut node,
                None,
            )
            .await?;

        // Replicate the sealed bytes BEFORE the node that references them, on
        // the node's own (tenant, repo) lane — see
        // `replication/operation_capture/secret_ops.rs`.
        self.capture_secret_versions(tenant_id, repo_id, branch, &vault_actor, &minted_secrets)
            .await;

        // ========== STEP 1: Allocate revision ==========
        let step_start = std::time::Instant::now();
        let revision = self.revision_repo.allocate_revision();
        let revision_time = step_start.elapsed().as_micros();

        // ========== STEP 2: Build WriteBatch with all indexes ==========
        let step_start = std::time::Instant::now();

        let mut batch = WriteBatch::default();

        // ========== STEP 2a: Order label - OPTIMIZED (skip existence check) ==========
        //
        // Runs BEFORE the node blob is serialized: it stamps `node.order_key`, and
        // the blob must carry the same label the index entry gets.
        let order_step_start = std::time::Instant::now();

        self.add_ordered_children_to_batch_fast_path(
            &mut batch, &mut node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        let order_label_time = order_step_start.elapsed().as_micros();

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
        // Full-snapshot ApplyRevision, same shape as the transaction commit
        // path, so peers apply an identical node state (including timestamps
        // and CF order key) instead of reconstructing it from granular ops.
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
            attribution,
        )
        .await;

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
    ///
    /// Also stamps the assigned label onto `node.order_key`, so the node record
    /// and the `ORDERED_CHILDREN` entry agree. Must therefore be called BEFORE
    /// the node blob is serialized into the batch.
    async fn add_ordered_children_to_batch_fast_path(
        &self,
        batch: &mut WriteBatch,
        node: &mut Node,
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
            let order_label =
                self.next_append_label(tenant_id, repo_id, branch, workspace, parent_id, revision)?;
            let get_last_time = t.elapsed().as_micros();

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

            // Keep the node record in step with the index entry.
            node.order_key = order_label;

            let batch_prep_time = substep_start.elapsed().as_micros();

            let total_order_time = order_start.elapsed().as_micros();

            if std::env::var("RAISIN_PROFILE").is_ok() {
                eprintln!(
                    "ORDER_TIMING_FAST node={} total={}us [parent={}us, calc={}us (get_last={}us), batch={}us]",
                    node.name,
                    total_order_time,
                    parent_lookup_time,
                    label_calc_time,
                    get_last_time,
                    batch_prep_time
                );
            }
        }

        Ok(())
    }
}
