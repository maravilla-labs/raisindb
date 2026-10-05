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
            let workspace = super::node_workspace(&change.node);
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
        // `Node.order_key == ORDERED_CHILDREN label`, on the replica too: the
        // record carries the label the entry below is written under.
        if !cf_key_to_use.is_empty() {
            normalized_node.order_key = cf_key_to_use.clone();
        }

        let mut batch = WriteBatch::default();
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
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

        // Diff against the version stored just BELOW this one and tombstone
        // stale index entries the snapshot supersedes. Without this a
        // replicated update/move leaves the peer's old path and old property
        // values live forever (reads by old path/value still match).
        //
        // Below `revision`, not the newest: ops arrive out of order, and when a
        // newer version is already stored the diff against IT would tombstone
        // its values at an older revision (a no-op) and leave this version's
        // own predecessor's values live. The newer version is handled after
        // the write, by `tombstone_superseded_by_newer`. AT `revision` counts
        // as below: an in-place (`versionable=false`) write replaces the version
        // stored at its own revision, and diffing past it left that version's
        // values (its old `__updated_at`, a changed title) live beside the new.
        let replaced = self.load_node_replaced_by(
            tenant_id,
            repo_id,
            branch,
            super::WorkspaceHint::of(&normalized_node),
            &normalized_node.id,
            revision,
        )?;
        if let Some((at, stored)) = &replaced {
            if super::newer_version::stale_in_place(at, stored, revision, &normalized_node) {
                tracing::debug!(
                    node_id = %normalized_node.id,
                    revision = %revision,
                    "replicated in-place write is older than the one already stored at its \
                     revision; skipped"
                );
                return Ok(());
            }
        }
        // An in-place write (a version is already stored AT this revision)
        // writes under the in-place guard, like the local in-place writers.
        let in_place = replaced.as_ref().is_some_and(|(at, _)| at == revision);
        // Its stale PROPERTY_INDEX values are tombstoned by the one writer
        // below (`write_all_node_indexes`, a full put against it).
        let replaced_node = replaced.as_ref().map(|(_, node)| node.clone());
        // Versions ABOVE this one: the op arrived older than what is stored.
        // A successor may be a LOCAL write made with skip-unchanged (this node
        // is an origin too), keeping unchanged entries BELOW `revision` that
        // the tombstones written here would mask — the property writer
        // re-asserts every successor at its own revision (`OutOfOrder`).
        let successors = if in_place {
            Vec::new()
        } else {
            crate::mvcc_read::node_versions_above(
                &self.db,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &normalized_node.id,
                revision,
            )?
        };
        if let Some((old_rev, old_node)) = replaced {
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
            crate::indexing::tombstone_superseded_spatial_indexes(
                &mut batch,
                &spatial_targets,
                &index_ctx,
                &old_node,
                Some(&normalized_node),
                revision,
                &spatial_policies,
            )?;

            if old_node.path != normalized_node.path
                && self.path_owned_at(&index_ctx, &old_node.path, &old_node.id, revision)?
            {
                let old_path_key = keys::path_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &old_node.path,
                    revision,
                );
                batch.put_cf(cf_path, old_path_key, TOMBSTONE);
            }

            // Old ordered-children entry (old parent / old label) must go too
            // — on a move, or the old parent still lists this node as a child,
            // and on a REORDER (same path, new label), or the old label stays
            // live beside the new one: `ORDER BY __order DESC` met it first and
            // listed the node at its old position.
            //
            // The old label is READ from ORDERED_CHILDREN as of `revision`,
            // never taken from the stored blob: a node written through the
            // transaction path before `order_key` was stamped carries "" there
            // (so a reorder of it tombstoned nothing), and a legacy copy carries
            // its SOURCE's label (so the tombstone landed on a label this child
            // never had). Only a label the ORIGIN supplied can relabel: an empty
            // `cf_order_key` means capture found no entry, and the locally
            // allocated fallback must not tombstone the entry the node has.
            //
            // The old PARENT is resolved as of the old version's own revision:
            // by path at HEAD it vanished whenever the parent had since moved
            // or been renamed (an op this replica applied first, out of
            // order), and the old entry was then never tombstoned.
            let supplied = !cf_order_key.is_empty();
            if old_node.path != normalized_node.path || supplied {
                if let Ok(Some(old_parent_id)) = crate::repositories::nodes::parent_index_id(
                    &self.db,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &old_node.path,
                    Some(&old_rev),
                ) {
                    let old_label = crate::repositories::nodes::stored_order_label_at(
                        &self.db,
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &old_parent_id,
                        &old_node.id,
                        Some(revision),
                    )?;
                    if let Some(old_label) = old_label {
                        if Some(old_parent_id.as_str()) != parent_id || old_label != cf_key_to_use {
                            let old_ordered_key = keys::ordered_child_key_versioned(
                                tenant_id,
                                repo_id,
                                branch,
                                workspace,
                                &old_parent_id,
                                &old_label,
                                revision,
                                &old_node.id,
                            );
                            batch.put_cf(cf_ordered, old_ordered_key, TOMBSTONE);
                        }
                    }
                }
            }
        }

        // The record — `StorageNode` blob and NODE_PATH — through the one
        // record writer (replicas used to store the full `Node`).
        crate::repositories::nodes::write_node_record(
            &self.db,
            &mut batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node,
            crate::repositories::nodes::parent_id_of(parent_id),
            revision,
        )?;

        let path_key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.path,
            revision,
        );
        batch.put_cf(cf_path, path_key, normalized_node.id.as_bytes());

        // Use index writer helpers to write all indexes — property, reference,
        // relation AND spatial. Spatial rides in this same batch, so a replica can
        // never hold the record without its index entries.
        write_all_node_indexes(
            &mut batch,
            &self.db,
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
            if successors.is_empty() {
                crate::indexing::Baseline::Full(replaced_node.as_ref())
            } else {
                crate::indexing::Baseline::OutOfOrder {
                    prior: replaced_node.as_ref(),
                    successors: &successors,
                }
            },
            in_place,
        )?;

        // Create the local index-state record when this replica is seeing a
        // geometry property for the first time, so the query planner reports
        // `Ready` rather than falling back to a scan on a fully-indexed replica.
        //
        // Walked, not flat: a replica that recorded state only for top-level
        // geometry reported `NotBuilt` for every nested path and answered nested
        // proximity queries by full scan forever, even though the entries were
        // right there in the same batch.
        for property_name in &crate::indexing::indexed_geometry_paths(&normalized_node.properties) {
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
                // The shared entry writer: the entry plus the parent's
                // last-child metadata when this label sorts last.
                crate::repositories::nodes::put_ordered_child(
                    &mut batch,
                    &self.db,
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    pid,
                    &cf_key_to_use,
                    revision,
                    &normalized_node.id,
                    &normalized_node.name,
                )?;
            }
        }

        // Out of order: a newer version is already stored. Everything written
        // above at `revision` that it does not carry ends at ITS revision.
        self.tombstone_superseded_by_newer(
            &mut batch,
            &index_ctx,
            &spatial_targets,
            &spatial_policies,
            &normalized_node,
            parent_id.filter(|_| !cf_key_to_use.is_empty()).map(|pid| {
                super::newer_version::OrderedPlacement {
                    parent_id: pid,
                    label: &cf_key_to_use,
                }
            }),
            revision,
        )?;

        // COMPOUND and UNIQUE entries from the cached definitions, or — cold —
        // the mark that fails the workspace's compound indexes closed plus a
        // local build request (see `compound_marker`).
        {
            let _in_place = in_place.then(|| {
                crate::repositories::nodes::in_place_write_guard(tenant_id, repo_id, branch)
            });
            let baseline = if successors.is_empty() {
                crate::indexing::Baseline::Full(replaced_node.as_ref())
            } else {
                crate::indexing::Baseline::OutOfOrder {
                    prior: replaced_node.as_ref(),
                    successors: &successors,
                }
            };
            self.commit_with_schema_indexes(
                batch,
                &index_ctx,
                baseline,
                &normalized_node,
                revision,
                in_place,
            )?;
        }

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
        let workspace = super::node_workspace(node);

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
}
