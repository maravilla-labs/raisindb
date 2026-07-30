//! CRDT operations: replicated revision, upsert/delete snapshots

use crate::{cf, cf_handle, fractional_index, keys, repositories::hash_property_value};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    Operation,
};
use raisin_storage::BranchRepository;
use rocksdb::WriteBatch;
use std::collections::HashMap;

use super::super::index_writers::write_all_node_indexes;
use super::super::node_operations::EventAttribution;
use super::{is_tombstone, OperationApplicator, TOMBSTONE};

impl OperationApplicator {
    /// Apply a replicated revision (batch of node changes with branch head update)
    pub(in crate::replication::application) async fn apply_replicated_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        branch_head: &HLC,
        node_changes: &[ReplicatedNodeChange],
        op: &Operation,
    ) -> Result<()> {
        let revision = Self::op_revision(op)?;
        // The originating node's attribution, replayed verbatim onto every event
        // this revision produces here.
        let attribution = EventAttribution::from_op(op);

        for change in node_changes {
            let workspace = change.node.workspace.as_deref().unwrap_or("default");
            match change.kind {
                ReplicatedNodeChangeKind::Upsert => self.apply_replicated_upsert(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &change.node,
                    change.parent_id.as_deref(),
                    &revision,
                    &change.cf_order_key,
                    attribution,
                )?,
                ReplicatedNodeChangeKind::Delete => self.apply_replicated_delete(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &change.node,
                    change.parent_id.as_deref(),
                    &revision,
                    attribution,
                )?,
            }
        }

        // A fresh node may receive revisions before (or without) an
        // UpdateBranch op for this branch - create it on demand instead of
        // failing the whole revision (mirrors git fetch creating refs).
        if self
            .branch_repo
            .get_branch(tenant_id, repo_id, branch)
            .await?
            .is_none()
        {
            tracing::info!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                branch = %branch,
                "Branch missing on replica - creating it from replicated revision"
            );
            self.branch_repo
                .create_branch(
                    tenant_id,
                    repo_id,
                    branch,
                    "replication",
                    None,
                    None,
                    false,
                    false,
                )
                .await?;
        }

        self.branch_repo
            .update_head(tenant_id, repo_id, branch, *branch_head)
            .await?;

        Ok(())
    }

    /// Apply a single replicated node upsert, forcing an `Updated` event.
    ///
    /// The `ApplyRevision` path has always reported `Updated` regardless of whether
    /// the node is new on this replica; preserved verbatim so subscribers and
    /// triggers see no change.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) fn apply_replicated_upsert(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        cf_order_key: &str,
        attribution: EventAttribution<'_>,
    ) -> Result<()> {
        self.apply_replicated_upsert_with_event(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            parent_id,
            revision,
            cf_order_key,
            attribution,
            Some(raisin_events::NodeEventKind::Updated),
        )
    }

    /// Apply a single replicated node upsert.
    ///
    /// `event_kind` of `None` derives `Created` vs `Updated` from the SOURCE node's
    /// timestamps (`created_at == updated_at` means the originating write was a
    /// create), which is what the snapshot path has always done. This parameter
    /// exists solely so the snapshot and revision paths can share one body without
    /// either one's event semantics changing — the drift between them was in the
    /// *indexing*, and that is what has been unified.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) fn apply_replicated_upsert_with_event(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        cf_order_key: &str,
        attribution: EventAttribution<'_>,
        event_kind: Option<raisin_events::NodeEventKind>,
    ) -> Result<()> {
        let mut normalized_node = node.clone();
        normalized_node.has_children = None;

        // Determine CF order key to use
        let cf_key_to_use = if !cf_order_key.is_empty() {
            cf_order_key.to_string()
        } else if let Some(pid) = parent_id {
            tracing::warn!(
                node_id = %normalized_node.id,
                "⚠️ REPLICATION BUG: cf_order_key is empty - falling back to local generation"
            );
            self.allocate_order_label(tenant_id, repo_id, branch, workspace, pid)?
        } else {
            String::new()
        };

        tracing::debug!(
            node_id = %normalized_node.id,
            cf_key = %cf_key_to_use,
            "📥 Applying CF order key from replication"
        );

        let mut batch = WriteBatch::default();
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let cf_spatial = cf_handle(&self.db, cf::SPATIAL_INDEX)?;

        // Resolve the spatial policy from the LOCAL index-state records before
        // touching the batch. Policy resolution reads schema, which is async; this
        // path is sync, so the state record acts as the local cache of the resolved
        // policy (see `crate::spatial_state`). A property with no record yet falls
        // back to the default precision set and the write creates the record —
        // which is what keeps automatic, opt-in-free indexing working on a replica
        // that has never seen a local write for this property.
        let spatial_targets = crate::indexing::SpatialIndexTargets {
            spatial_index: cf_spatial,
        };
        let index_ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace);
        let spatial_state = crate::spatial_state::SpatialStateStore::new(self.db.clone());
        let spatial_policies = crate::indexing::NodeSpatialPolicies::from_local_state(
            &spatial_state,
            &index_ctx,
            &normalized_node,
        );

        // Diff against the locally-stored previous version and tombstone
        // stale index entries the snapshot supersedes. Without this a
        // replicated update/move leaves the peer's old path and old property
        // values live forever (reads by old path/value still match).
        if let Some(old_node) =
            self.load_latest_node(tenant_id, repo_id, branch, &normalized_node.id)?
        {
            crate::repositories::add_stale_property_tombstones(
                &mut batch,
                cf_property,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &old_node,
                &normalized_node,
                revision,
            );

            crate::repositories::add_stale_reference_tombstones(
                &mut batch,
                cf_reference,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &old_node,
                &normalized_node,
                revision,
            );

            // Stale SPATIAL entries. Derived from `old_node`'s geometry, never by a
            // prefix scan: a scan here would be O(all geometries in the workspace)
            // for a single replicated upsert. Without this a replicated MOVE leaves
            // the peer matching at BOTH the old and the new location.
            crate::indexing::spatial::tombstone_superseded_spatial_indexes(
                &mut batch,
                &spatial_targets,
                &index_ctx,
                &old_node,
                Some(&normalized_node),
                revision,
                &spatial_policies,
            )?;

            if old_node.path != normalized_node.path {
                let old_path_key = keys::path_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &old_node.path,
                    revision,
                );
                batch.put_cf(cf_path, old_path_key, TOMBSTONE);

                // Old ordered-children entry (old parent / old label) must go
                // too, or the old parent still lists this node as a child.
                if let Ok(Some(old_parent_id)) = self.resolve_parent_id_for_snapshot(
                    tenant_id, repo_id, branch, workspace, &old_node,
                ) {
                    let old_ordered_key = keys::ordered_child_key_versioned(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &old_parent_id,
                        &old_node.order_key,
                        revision,
                        &old_node.id,
                    );
                    batch.put_cf(cf_ordered, old_ordered_key, TOMBSTONE);
                }
            }
        }

        let node_value = rmp_serde::to_vec_named(&normalized_node)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let node_key = keys::node_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.id,
            revision,
        );
        batch.put_cf(cf_nodes, node_key, node_value);

        let path_key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.path,
            revision,
        );
        batch.put_cf(cf_path, path_key, normalized_node.id.as_bytes());

        let node_path_key = keys::node_path_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.id,
            revision,
        );
        batch.put_cf(cf_node_path, node_path_key, normalized_node.path.as_bytes());

        // Use index writer helpers to write all indexes — property, reference,
        // relation AND spatial. Spatial rides in this same batch, so a replica can
        // never hold the record without its index entries.
        write_all_node_indexes(
            &mut batch,
            &super::super::index_writers::ReplicationIndexCfs {
                property: cf_property,
                reference: cf_reference,
                relation: cf_relation,
                spatial: cf_spatial,
            },
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node,
            revision,
            &spatial_policies,
        )?;

        // Create the local index-state record when this replica is seeing a
        // geometry property for the first time, so the query planner reports
        // `Ready` rather than falling back to a scan on a fully-indexed replica.
        for (property_name, value) in &normalized_node.properties {
            if !matches!(value, PropertyValue::Geometry(_)) {
                continue;
            }
            let policy = spatial_policies.for_property(property_name);
            if let Err(e) = spatial_state.ensure_for_write(
                &mut batch,
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                policy,
                *revision,
            ) {
                tracing::warn!(
                    property = %property_name,
                    error = %e,
                    "Could not record spatial index state for replicated node"
                );
            }
        }

        if let Some(pid) = parent_id {
            if cf_key_to_use.is_empty() {
                tracing::warn!(
                    node_id = %normalized_node.id,
                    parent_id = %pid,
                    "⚠️ Skipping ORDERED_CHILDREN update due to missing cf_order_key"
                );
            } else {
                let ordered_key = keys::ordered_child_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    pid,
                    &cf_key_to_use,
                    revision,
                    &normalized_node.id,
                );
                batch.put_cf(cf_ordered, ordered_key, normalized_node.name.as_bytes());

                let metadata_key =
                    keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, pid);
                let should_update = match self.db.get_cf(cf_ordered, &metadata_key) {
                    Ok(Some(existing)) => {
                        let existing_label = String::from_utf8_lossy(&existing);
                        cf_key_to_use.as_str() > existing_label.as_ref()
                    }
                    _ => true,
                };

                if should_update {
                    batch.put_cf(cf_ordered, metadata_key, cf_key_to_use.as_bytes());
                }
            }
        }

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply replicated upsert: {}", e))
        })?;

        // Event kind: forced by the caller, or derived from the SOURCE node's
        // timestamps. Deriving locally (e.g. "did load_latest_node find anything")
        // would be wrong on a replica that is catching up out of order.
        let resolved_kind = event_kind.unwrap_or_else(|| {
            match (normalized_node.created_at, normalized_node.updated_at) {
                (Some(created), Some(updated)) if created == updated => {
                    raisin_events::NodeEventKind::Created
                }
                (Some(_), Some(_)) => raisin_events::NodeEventKind::Updated,
                (Some(_), None) => raisin_events::NodeEventKind::Created,
                // No timestamps: report an update, which is the conservative
                // choice (a spurious Created can fire create-only side effects).
                _ => raisin_events::NodeEventKind::Updated,
            }
        });

        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.id,
            Some(normalized_node.node_type.clone()),
            Some(normalized_node.path.clone()),
            revision,
            resolved_kind,
            "replication",
            attribution,
        );

        Ok(())
    }

    /// Apply a single replicated node delete
    ///
    /// Delegates to the shared `crate::tombstones` module (single source of
    /// truth for deletion tombstones), so replicated deletes clean up the same
    /// families as local deletes — including packed adjacency lists,
    /// compound/spatial indexes, and NODE_PATH.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) fn apply_replicated_delete(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        attribution: EventAttribution<'_>,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();

        // The shared tombstone path derives the ORDERED_CHILDREN key from
        // node.parent; replicated changes carry the parent separately.
        let mut node_for_tombstones = node.clone();
        if node_for_tombstones.parent.is_none() {
            node_for_tombstones.parent = parent_id.map(|p| p.to_string());
        }

        let ctx = crate::tombstones::TombstoneContext::new(tenant_id, repo_id, branch, workspace);
        let cfs = crate::tombstones::TombstoneColumnFamilies::from_arc_db(&self.db)?;
        crate::tombstones::add_node_tombstones(
            &mut batch,
            self.db.as_ref(),
            &ctx,
            &cfs,
            &node_for_tombstones,
            revision,
        )?;

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply replicated delete: {}", e))
        })?;

        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            Some(node.node_type.clone()),
            Some(node.path.clone()),
            revision,
            raisin_events::NodeEventKind::Deleted,
            "replication",
            attribution,
        );

        Ok(())
    }

    /// Apply a node snapshot upsert (decomposed from ApplyRevision for CRDT commutativity)
    ///
    /// Uses Last-Write-Wins (LWW) semantics via the revision HLC.
    pub(in crate::replication::application) async fn apply_upsert_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        cf_order_key: &str,
        op: &Operation,
    ) -> Result<()> {
        let workspace = node.workspace.as_deref().unwrap_or("default");

        self.apply_replicated_upsert(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            parent_id,
            revision,
            cf_order_key,
            EventAttribution::from_op(op),
        )?;

        tracing::debug!(
            node_id = %node.id,
            revision = ?revision,
            "Applied UpsertNodeSnapshot with LWW semantics"
        );

        Ok(())
    }

    /// Apply a node snapshot delete (decomposed from ApplyRevision for CRDT commutativity)
    ///
    /// Uses Delete-Wins semantics - deletions always take precedence.
    pub(in crate::replication::application) async fn apply_delete_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
        revision: &HLC,
        op: &Operation,
    ) -> Result<()> {
        let node = match self.load_latest_node(tenant_id, repo_id, branch, node_id)? {
            Some(n) => n,
            None => {
                tracing::debug!(
                    node_id = %node_id,
                    revision = ?revision,
                    "Node not found for DeleteNodeSnapshot - treating as already deleted"
                );
                return Ok(());
            }
        };

        let workspace = node.workspace.as_deref().unwrap_or("default");

        // We use None for parent_id - delete logic handles this gracefully
        let _parent_id: Option<&str> = None;

        self.apply_replicated_delete(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node,
            None,
            revision,
            EventAttribution::from_op(op),
        )?;

        tracing::debug!(
            node_id = %node_id,
            revision = ?revision,
            "Applied DeleteNodeSnapshot with Delete-Wins semantics"
        );

        Ok(())
    }
}
