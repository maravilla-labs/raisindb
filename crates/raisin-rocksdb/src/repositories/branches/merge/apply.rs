//! Writing a resolved conflict into the target branch.
//!
//! A merge in this engine is a KEY-LEVEL operation: `copy_branch_indexes`
//! replays the source branch's entries into the target branch keeping their
//! original revisions, and the merge commit then advances the target HEAD past
//! all of them. That is why applying a conflict resolution is not "call the
//! node repository" — the node repository needs a `BranchRepositoryImpl` to
//! exist, which is what we are inside of — but a write at the freshly
//! allocated merge revision M, which is newer than either branch's HEAD and
//! therefore shadows both sides no matter which one the copy brought over.
//!
//! Merge apply used to be its own mirrored write funnel: no ORDERED_CHILDREN
//! entry or tombstone, no NODE_PATH tombstone on delete, a `\0` path tombstone
//! no reader recognised, and stale-value tombstones only for the target's old
//! version. A merge-created node was missing from CHILD_OF, a merge-deleted one
//! stayed listed, and `keep-ours` kept matching the source's values. Now:
//!
//! - a deletion goes through `tombstones::add_node_tombstones_with_parent`,
//!   the one delete tombstoner, once per superseded version (with the parent
//!   that version's own branch had), plus its UNIQUE entries;
//! - an upsert tombstones the UNION of what base, target head and source head
//!   indexed — properties, references, geometries, UNIQUE values, old path and
//!   old ORDERED_CHILDREN entry — then does a full put at M: blob, PATH_INDEX,
//!   NODE_PATH, every index through `write_all_node_indexes`, UNIQUE through
//!   the local write path's sync writer, and ORDERED_CHILDREN through
//!   `put_ordered_child`, which also advances the parent's last-child label.
//!
//! Not written at M: translations (Phase 11) and COMPOUND entries, which the
//! workspace's stale marker covers until a rebuild.

use super::superseded::{order_entry, Superseded};
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

pub(super) use super::superseded::{load_node_at, superseded_versions};

pub(super) use super::deletion::write_resolved_deletion;
pub(super) use super::unique_props::UniqueProperties;

/// Write `node` into `target_branch` at `revision`, shadowing every version
/// in `superseded`.
///
/// `origin` is the branch (and revision) `node` was taken from: its parent
/// key and stored order label are read there, because a node the source
/// created has no entry on the target until the copy runs.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_resolved_node(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    target_branch: &str,
    workspace: &str,
    node: &Node,
    origin: (&str, &HLC),
    revision: &HLC,
    superseded: &[Superseded],
    unique: &UniqueProperties,
) -> Result<()> {
    let mut normalized = node.clone();
    normalized.has_children = None;

    let placement = super::superseded::placement(
        db,
        tenant_id,
        repo_id,
        target_branch,
        workspace,
        &normalized,
        origin,
        revision,
    )?;
    // `Node.order_key` must equal the stored label.
    if let Some((_, ref label)) = placement {
        normalized.order_key = label.clone();
    }

    let mut batch = WriteBatch::default();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;
    let cf_node_path = cf_handle(db, cf::NODE_PATH)?;
    let cf_property = cf_handle(db, cf::PROPERTY_INDEX)?;
    let cf_reference = cf_handle(db, cf::REFERENCE_INDEX)?;
    let cf_relation = cf_handle(db, cf::RELATION_INDEX)?;
    let cf_spatial = cf_handle(db, cf::SPATIAL_INDEX)?;
    let cf_ordered = cf_handle(db, cf::ORDERED_CHILDREN)?;

    // Policy resolution reads schema records, which is async, and this path is
    // sync — exactly the constraint the replication apply path has. The local
    // spatial-state record is that path's cache of the resolved policy, so it
    // is the cache here too, and a property with no record yet falls back to
    // the default precision set.
    let index_ctx = crate::indexing::IndexCtx::new(tenant_id, repo_id, target_branch, workspace);
    let spatial_state = crate::spatial_state::SpatialStateStore::new(Arc::clone(db));
    let spatial_policies = crate::indexing::NodeSpatialPolicies::from_local_state(
        &spatial_state,
        &index_ctx,
        &normalized,
    );

    // 1. Tombstone, at M, everything any superseded version indexed that the
    //    resolved node does not. Written BEFORE the puts below, so a key both
    //    tombstoned and re-put ends up live (a batch applies in order).
    let spatial_targets = crate::indexing::SpatialIndexTargets {
        spatial_index: cf_spatial,
    };
    for old in superseded {
        crate::repositories::add_stale_property_tombstones(
            &mut batch,
            cf_property,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &old.node,
            &normalized,
            revision,
        );
        crate::repositories::add_stale_reference_tombstones(
            &mut batch,
            cf_reference,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &old.node,
            &normalized,
            revision,
        );
        // Geometries: a superseded version's old position must stop matching
        // ST_DWITHIN, exactly as the replicated upsert retires it.
        crate::indexing::tombstone_superseded_spatial_indexes(
            &mut batch,
            &spatial_targets,
            &index_ctx,
            &old.node,
            Some(&normalized),
            revision,
            &spatial_policies,
        )?;
        crate::repositories::nodes::tombstone_unique_entries(
            &mut batch,
            db,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &old.node,
            unique.of(&old.node.node_type),
            revision,
        )?;
        if old.node.path != normalized.path && !old.node.path.is_empty() {
            let old_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old.node.path,
                revision,
            );
            batch.put_cf(cf_path, old_path_key, keys::TOMBSTONE_VALUE);
        }
        if let Some((parent, label)) = order_entry(
            db,
            tenant_id,
            repo_id,
            &old.branch,
            workspace,
            &old.node,
            Some(&old.at),
        )? {
            if placement.as_ref() != Some(&(parent.clone(), label.clone())) {
                let key = keys::ordered_child_key_versioned(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &parent,
                    &label,
                    revision,
                    &normalized.id,
                );
                batch.put_cf(cf_ordered, key, keys::TOMBSTONE_VALUE);
            }
        }
    }

    // 2. The full put at M.
    let node_value = rmp_serde::to_vec_named(&normalized)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {e}")))?;
    batch.put_cf(
        cf_nodes,
        keys::node_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &normalized.id,
            revision,
        ),
        node_value,
    );
    batch.put_cf(
        cf_path,
        keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &normalized.path,
            revision,
        ),
        normalized.id.as_bytes(),
    );
    batch.put_cf(
        cf_node_path,
        keys::node_path_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &normalized.id,
            revision,
        ),
        normalized.path.as_bytes(),
    );
    if let Some((parent, label)) = &placement {
        crate::repositories::nodes::put_ordered_child(
            &mut batch,
            db,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            parent,
            label,
            revision,
            &normalized.id,
            &normalized.name,
        )?;
    }
    crate::repositories::nodes::write_unique_entries(
        &mut batch,
        db,
        tenant_id,
        repo_id,
        target_branch,
        workspace,
        &normalized,
        unique.of(&normalized.node_type),
        revision,
    )?;

    crate::replication::application::index_writers::write_all_node_indexes(
        &mut batch,
        &crate::replication::application::index_writers::ReplicationIndexCfs {
            property: cf_property,
            reference: cf_reference,
            relation: cf_relation,
            spatial: cf_spatial,
        },
        tenant_id,
        repo_id,
        target_branch,
        workspace,
        &normalized,
        revision,
        &spatial_policies,
    )?;

    // The compound NotBuilt mark rides in the same batch, under the compound
    // transitions lock: merge apply writes no COMPOUND entries for an upsert.
    crate::compound_state::CompoundStateStore::new(Arc::clone(db)).write_marking_stale(
        batch,
        tenant_id,
        repo_id,
        target_branch,
        workspace,
    )?;

    Ok(())
}
