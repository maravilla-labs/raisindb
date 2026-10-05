//! Soft deletion via tombstone markers.
//!
//! Writes tombstone entries at a new revision for all node data and indexes,
//! preserving history for MVCC reads at older revisions.

use super::super::super::helpers::{hash_property_value, TOMBSTONE};
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::{EventBus, NodeEvent, NodeEventKind};
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
        attribution: crate::repositories::nodes::WriteAttribution<'_>,
    ) -> Result<bool> {
        // Check referential integrity first - prevent deletion if other nodes reference this node
        self.check_delete_safety(tenant_id, repo_id, branch, workspace, id)
            .await?;

        // The stored versions BEFORE the version this delete ends is read:
        // re-checked under the node's commit lock (plan Phase 7b).
        let pending_check = crate::indexing::StagedDeltaCheck::before_read(
            &self.db,
            &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
            id,
        )?;

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

        // (No separate ordered-children "insurance" here any more: the shared
        // tombstoner resolves the parent's ID and tombstones the STORED label.
        // The old insurance keyed its lookup by `node.parent` — the parent's
        // NAME — so it never found the entry it meant to cover.)

        // Add revision indexing to batch (ATOMIC)
        self.revision_repo
            .index_node_change_to_batch(&mut batch, tenant_id, repo_id, &revision, id)?;

        // Add branch HEAD update to the batch and write it atomically
        let mut commit = crate::indexing::NodeCommit::new(tenant_id, repo_id, branch);
        commit.check(pending_check.at(&revision), None);
        let updated_branch = self
            .branch_repo
            .write_nodes_with_head(batch, tenant_id, repo_id, branch, revision, &commit)
            .await?;

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

        // Capture ApplyRevision delete snapshot for replication
        // (non-transaction path). The full pre-delete node lets peers
        // tombstone every index family without a local lookup.
        self.capture_apply_revision_snapshot(
            tenant_id,
            repo_id,
            branch,
            workspace,
            vec![(
                node.clone(),
                raisin_replication::operation::ReplicatedNodeChangeKind::Delete,
            )],
            revision,
            attribution,
        )
        .await;

        self.publish_deleted_event(
            tenant_id,
            repo_id,
            branch,
            workspace,
            revision,
            &node,
            attribution,
        );

        Ok(true)
    }

    /// Publish the `node:deleted` event for one node.
    ///
    /// ONE emitter for both delete paths (single-node `delete_impl` and the
    /// cascade in `crud/cascade/tree.rs`): the cascade path used to emit
    /// nothing, so a WebSocket subscriber never saw `node:deleted` for the
    /// default (cascade = true) delete, and only the job system — which reads
    /// tombstones directly — noticed the deletion.
    ///
    /// The metadata carries the same `actor` / `agent` attribution the
    /// replication capture carries (keys read by `raisin-core::audit_events`),
    /// plus `node_data`, the full pre-delete node: the node is tombstoned by
    /// the time a subscriber asks for it, so `include_node` and RLS on the WS
    /// side can only be served from the event itself.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) fn publish_deleted_event(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: raisin_hlc::HLC,
        node: &raisin_models::nodes::Node,
        attribution: crate::repositories::nodes::WriteAttribution<'_>,
    ) {
        let mut m = std::collections::HashMap::new();
        if let Some(actor) = attribution.actor.as_deref() {
            m.insert(
                "actor".to_string(),
                serde_json::Value::String(actor.to_string()),
            );
        }
        if let Some(agent) = attribution.agent.as_deref() {
            m.insert(
                "agent".to_string(),
                serde_json::Value::String(agent.to_string()),
            );
        }
        if let Ok(node_json) = serde_json::to_value(node) {
            m.insert("node_data".to_string(), node_json);
        }

        let node_event = NodeEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo_id.to_string(),
            workspace_id: workspace.to_string(),
            branch: branch.to_string(),
            revision,
            node_id: node.id.clone(),
            node_type: Some(node.node_type.clone()),
            kind: NodeEventKind::Deleted,
            path: Some(node.path.clone()),
            metadata: Some(m),
        };

        self.event_bus
            .publish(raisin_events::Event::Node(node_event));
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
    ///
    /// Through the ONE reference tombstoner (`add_stale_reference_tombstones`,
    /// diffed against no references at all), so nested references — arrays,
    /// objects, element and composite content — are found by the same walker
    /// and keyed by the same dot path the writers used. A top-level-only loop
    /// here left every nested reference of a pruned node live forever. Both
    /// published variants are tombstoned, so `is_published` no longer selects.
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
        _is_published: bool,
    ) {
        debug_assert_eq!(id, node.id, "reference tombstones keyed by another id");
        crate::repositories::add_stale_reference_tombstones(
            batch,
            cf_reference,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            &raisin_models::nodes::Node::default(),
            revision,
        );
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
}
