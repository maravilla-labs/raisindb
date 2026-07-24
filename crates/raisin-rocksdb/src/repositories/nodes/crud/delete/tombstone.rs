//! Soft deletion via tombstone markers.
//!
//! Writes tombstone entries at a new revision for all node data and indexes,
//! preserving history for MVCC reads at older revisions.

use super::super::super::helpers::{hash_property_value, TOMBSTONE};
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::{EventBus, NodeEvent, NodeEventKind};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::RevisionRepository;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    pub(in super::super::super) async fn delete_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
    ) -> Result<bool> {
        // Check referential integrity first - prevent deletion if other nodes reference this node
        self.check_delete_safety(tenant_id, repo_id, branch, workspace, id)
            .await?;

        // Get current node to check if it exists and get its properties (internal operation)
        let node = match self
            .get_impl(tenant_id, repo_id, branch, workspace, id, false)
            .await?
        {
            Some(n) => n,
            None => return Ok(false),
        };

        // Allocate a new revision for the deletion
        let revision = self.revision_repo.allocate_revision();

        eprintln!(
            "🗑️  delete_impl: node_id={}, allocated revision={}",
            id, revision
        );

        // Prepare WriteBatch for atomic multi-operation delete
        let mut batch = WriteBatch::default();

        // All per-CF tombstones come from the shared tombstones module — the
        // SINGLE SOURCE OF TRUTH also used by the cascade and transaction
        // delete paths. (This path used to maintain a parallel hand-built
        // list, which drifted: it never tombstoned NODE_PATH, COMPOUND_INDEX,
        // or SPATIAL_INDEX entries.)
        let ctx = crate::tombstones::TombstoneContext::new(tenant_id, repo_id, branch, workspace);
        let cfs = crate::tombstones::TombstoneColumnFamilies::from_db(&self.db)?;
        crate::tombstones::add_node_tombstones(&mut batch, &self.db, &ctx, &cfs, &node, &revision)?;

        // Unique index tombstones (async — needs NodeType lookup to find the
        // unique properties), not covered by the shared module.
        self.add_unique_tombstones_to_batch(
            &mut batch, &node, tenant_id, repo_id, branch, workspace, &revision,
        )
        .await?;

        // Ordered-children insurance: the shared module tombstones the entry
        // under `node.order_key`, but the STORED label can differ (e.g. after
        // rebalance/copy). Tombstone the looked-up label too when it differs.
        if let Some(ref parent_id) = node.parent {
            if let Some(label) = self
                .get_order_label_for_child(tenant_id, repo_id, branch, workspace, parent_id, id)?
            {
                if label != node.order_key {
                    let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
                    let ordered_key = keys::ordered_child_key_versioned(
                        tenant_id, repo_id, branch, workspace, parent_id, &label, &revision, id,
                    );
                    batch.put_cf(cf_ordered, ordered_key, TOMBSTONE);
                }
            }
        }

        // Add revision indexing to batch (ATOMIC)
        self.revision_repo
            .index_node_change_to_batch(&mut batch, tenant_id, repo_id, &revision, id)?;

        // Add branch HEAD update to the same atomic batch
        let updated_branch = self
            .branch_repo
            .update_head_to_batch(&mut batch, tenant_id, repo_id, branch, revision)
            .await?;

        // Atomic commit - all operations succeed or fail together
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic delete failed: {}", e)))?;

        // Capture replication events (after atomic write)
        self.branch_repo
            .capture_head_update_for_replication(
                tenant_id,
                repo_id,
                branch,
                &updated_branch,
                revision,
            )
            .await;

        // Capture DeleteNode operation for replication (non-transaction path)
        if self.operation_capture.is_enabled() {
            let op_type = raisin_replication::OpType::DeleteNode {
                node_id: id.to_string(),
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

        // Emit node deletion event to trigger background cleanup job
        let node_event = NodeEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo_id.to_string(),
            workspace_id: workspace.to_string(),
            branch: branch.to_string(),
            revision,
            node_id: id.to_string(),
            node_type: Some(node.node_type.clone()),
            kind: NodeEventKind::Deleted,
            path: Some(node.path.clone()),
            metadata: None,
        };

        self.event_bus
            .publish(raisin_events::Event::Node(node_event));

        Ok(true)
    }

    /// Add tombstone entries for common node field indexes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn add_field_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        cf_property: &rocksdb::ColumnFamily,
        node: &raisin_models::nodes::Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        revision: &raisin_hlc::HLC,
        is_published: bool,
    ) {
        if !node.name.is_empty() {
            let name_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__name",
                &node.name,
                revision,
                id,
                is_published,
            );
            batch.put_cf(cf_property, name_key, TOMBSTONE);
        }
        if let Some(ref archetype) = node.archetype {
            if !archetype.is_empty() {
                let archetype_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__archetype",
                    archetype,
                    revision,
                    id,
                    is_published,
                );
                batch.put_cf(cf_property, archetype_key, TOMBSTONE);
            }
        }
        if let Some(ref created_by) = node.created_by {
            if !created_by.is_empty() {
                let created_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__created_by",
                    created_by,
                    revision,
                    id,
                    is_published,
                );
                batch.put_cf(cf_property, created_by_key, TOMBSTONE);
            }
        }
        if let Some(ref updated_by) = node.updated_by {
            if !updated_by.is_empty() {
                let updated_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__updated_by",
                    updated_by,
                    revision,
                    id,
                    is_published,
                );
                batch.put_cf(cf_property, updated_by_key, TOMBSTONE);
            }
        }
        if let Some(created_at) = node.created_at {
            let created_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__created_at",
                created_at.timestamp_micros(),
                revision,
                id,
                is_published,
            );
            batch.put_cf(cf_property, created_at_key, TOMBSTONE);
        }
        if let Some(updated_at) = node.updated_at {
            let updated_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__updated_at",
                updated_at.timestamp_micros(),
                revision,
                id,
                is_published,
            );
            batch.put_cf(cf_property, updated_at_key, TOMBSTONE);
        }
    }

    /// Add tombstone entries for reference indexes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn add_reference_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        cf_reference: &rocksdb::ColumnFamily,
        node: &raisin_models::nodes::Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        revision: &raisin_hlc::HLC,
        is_published: bool,
    ) {
        for (prop_path, prop_value) in &node.properties {
            if let PropertyValue::Reference(ref_data) = prop_value {
                // Tombstone forward reference
                let fwd_key = keys::reference_forward_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    id,
                    prop_path,
                    revision,
                    is_published,
                );
                batch.put_cf(cf_reference, fwd_key, TOMBSTONE);

                // Tombstone reverse reference
                let rev_key = keys::reference_reverse_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &ref_data.workspace,
                    &ref_data.id,
                    id,
                    prop_path,
                    revision,
                    is_published,
                );
                batch.put_cf(cf_reference, rev_key, TOMBSTONE);
            }
        }
    }

    /// Add tombstone entries for outgoing and incoming relations.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn add_relation_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        cf_relation: &rocksdb::ColumnFamily,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        // Tombstone ALL outgoing relations
        let outgoing_relations =
            self.get_outgoing_relations(tenant_id, repo_id, branch, workspace, id)?;

        for relation in outgoing_relations {
            let fwd_key = keys::relation_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                id,
                &relation.relation_type,
                revision,
                &relation.target,
            );
            batch.put_cf(cf_relation, fwd_key, TOMBSTONE);

            let rev_key = keys::relation_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                &relation.workspace,
                &relation.target,
                &relation.relation_type,
                revision,
                id,
            );
            batch.put_cf(cf_relation, rev_key, TOMBSTONE);
        }

        // Tombstone ALL incoming relations
        let incoming_relations =
            self.get_incoming_relations(tenant_id, repo_id, branch, workspace, id)?;

        for (source_node_id, relation_type, source_workspace) in &incoming_relations {
            let fwd_key = keys::relation_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                source_workspace,
                source_node_id,
                relation_type,
                revision,
                id,
            );
            batch.put_cf(cf_relation, fwd_key, TOMBSTONE);

            let rev_key = keys::relation_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                id,
                relation_type,
                revision,
                source_node_id,
            );
            batch.put_cf(cf_relation, rev_key, TOMBSTONE);
        }

        // ALSO clear the PACKED adjacency lists (see
        // relations::helpers::packed_adjacency_cleanup_puts) — same batch, so
        // the packed rewrite stays atomic with the tombstones above.
        let puts = crate::repositories::packed_adjacency_cleanup_puts(
            &self.db,
            revision,
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            incoming_relations
                .iter()
                .map(|(src_id, _, src_ws)| (src_ws.clone(), src_id.clone())),
        )?;
        for (key, value) in puts {
            batch.put_cf(cf_relation, key, value);
        }

        Ok(())
    }

    /// Add tombstone entries for translations.
    pub(in crate::repositories::nodes) fn add_translation_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        revision: &raisin_hlc::HLC,
    ) -> Result<()> {
        let cf_translation_data = cf_handle(&self.db, cf::TRANSLATION_DATA)?;
        let translation_locales =
            self.list_translation_locales(tenant_id, repo_id, branch, workspace, id)?;

        for locale in translation_locales {
            let mut translation_key = format!(
                "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
                tenant_id, repo_id, branch, workspace, id, locale
            )
            .into_bytes();
            translation_key.extend_from_slice(&keys::encode_descending_revision(revision));

            batch.put_cf(cf_translation_data, translation_key, TOMBSTONE);
        }

        Ok(())
    }
}
