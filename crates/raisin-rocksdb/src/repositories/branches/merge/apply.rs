//! Writing a resolved conflict into the target branch.
//!
//! A merge in this engine is a KEY-LEVEL operation: `copy_branch_indexes`
//! replays the source branch's entries into the target branch keeping their
//! original revisions, and the merge commit then advances the target HEAD past
//! all of them. That is why applying a conflict resolution is not "call the
//! node repository" — the node repository needs a `BranchRepositoryImpl` to
//! exist, which is what we are inside of — but a write at the freshly
//! allocated merge revision, which is newer than either branch's HEAD and
//! therefore shadows both sides no matter which one the copy brought over.
//!
//! **The blob alone is not enough.** Writing only `cf::NODES` leaves the
//! chosen value invisible to `properties->>'x' = ...`, to `REFERENCES(...)`
//! and to `ST_DWITHIN`, because the copy has already put the *losing* side's
//! index entries in at a revision the reader would find first. So this goes
//! through the SAME `write_all_node_indexes` the replication apply path uses,
//! preceded by the same stale-entry tombstones — one writer, one key format.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

/// Marker byte a deleted revision carries in `cf::NODES`.
const NODE_TOMBSTONE: &[u8] = b"T";
/// Marker an index entry carries when it shadows a superseded one.
const INDEX_TOMBSTONE: &[u8] = b"\x00";

/// Load a node from a branch as of `max_revision` (inclusive).
///
/// Returns `Ok(None)` for "absent or deleted at that revision" — the same
/// answer a HEAD-bounded read gives, so a caller cannot accidentally resurrect
/// a tombstoned node by resolving a conflict over it.
pub(super) fn load_node_at(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    max_revision: &HLC,
) -> Result<Option<Node>> {
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, workspace, node_id);

    for item in db.prefix_iterator_cf(cf_nodes, &prefix) {
        let (key, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {e}")))?;

        // A prefix iterator seeks to the prefix and then keeps going, so an id
        // that does not exist would otherwise deserialize the NEXT node in the
        // keyspace. Matching keys are contiguous; stop at the first that isn't.
        if !key.starts_with(&prefix) {
            break;
        }
        if key.len() < 16 {
            continue;
        }

        let revision = keys::decode_descending_revision(&key[key.len() - 16..])
            .map_err(|e| raisin_error::Error::storage(format!("Revision decode error: {e}")))?;
        if revision > *max_revision {
            continue;
        }

        // Revisions are stored descending, so the first one at or below the
        // bound is the newest one — including a tombstone, which means deleted.
        if value.as_ref() == NODE_TOMBSTONE {
            return Ok(None);
        }

        let node: Node = rmp_serde::from_slice(&value).map_err(|e| {
            raisin_error::Error::storage(format!("Node deserialization error: {e}"))
        })?;
        return Ok(Some(node));
    }

    Ok(None)
}

/// Write `node` into `target_branch` at `revision`, shadowing whatever either
/// side of the merge left behind.
///
/// Mirrors the replication apply path: node blob, path index, node-path index,
/// then property / reference / relation / spatial indexes through the one
/// shared writer, with stale entries from the target's previous revision
/// tombstoned first.
pub(super) fn write_resolved_node(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    target_branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let mut normalized = node.clone();
    normalized.has_children = None;

    let mut batch = WriteBatch::default();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;
    let cf_node_path = cf_handle(db, cf::NODE_PATH)?;
    let cf_property = cf_handle(db, cf::PROPERTY_INDEX)?;
    let cf_reference = cf_handle(db, cf::REFERENCE_INDEX)?;
    let cf_relation = cf_handle(db, cf::RELATION_INDEX)?;
    let cf_spatial = cf_handle(db, cf::SPATIAL_INDEX)?;

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

    // Tombstone what the target branch's previous revision indexed, or a query
    // on the OLD value keeps matching this node after the merge resolved it away.
    if let Some(old_node) = load_node_at(
        db,
        tenant_id,
        repo_id,
        target_branch,
        workspace,
        &normalized.id,
        revision,
    )? {
        crate::repositories::add_stale_property_tombstones(
            &mut batch,
            cf_property,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &old_node,
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
            &old_node,
            &normalized,
            revision,
        );
        if old_node.path != normalized.path {
            let old_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_node.path,
                revision,
            );
            batch.put_cf(cf_path, old_path_key, INDEX_TOMBSTONE);
        }
    }

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

    db.write(batch)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    Ok(())
}

/// Write a delete tombstone for `node_id` in `target_branch` at `revision`.
///
/// Used when a resolution says the node should not survive the merge. The
/// node blob tombstone is what every read consults; the index entries the
/// losing side copied in are shadowed by the stale-entry tombstones.
pub(super) fn write_resolved_deletion(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    target_branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Result<()> {
    let mut batch = WriteBatch::default();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let cf_path = cf_handle(db, cf::PATH_INDEX)?;

    if let Some(old_node) = load_node_at(
        db,
        tenant_id,
        repo_id,
        target_branch,
        workspace,
        node_id,
        revision,
    )? {
        let cf_property = cf_handle(db, cf::PROPERTY_INDEX)?;
        let cf_reference = cf_handle(db, cf::REFERENCE_INDEX)?;
        let empty = Node {
            properties: Default::default(),
            relations: Vec::new(),
            ..old_node.clone()
        };
        crate::repositories::add_stale_property_tombstones(
            &mut batch,
            cf_property,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &old_node,
            &empty,
            revision,
        );
        crate::repositories::add_stale_reference_tombstones(
            &mut batch,
            cf_reference,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &old_node,
            &empty,
            revision,
        );
        batch.put_cf(
            cf_path,
            keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_node.path,
                revision,
            ),
            INDEX_TOMBSTONE,
        );
    }

    batch.put_cf(
        cf_nodes,
        keys::node_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            node_id,
            revision,
        ),
        NODE_TOMBSTONE,
    );

    db.write(batch)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    Ok(())
}
