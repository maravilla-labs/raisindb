//! Core tombstone functions: node data, path index, node path, ordered children

use super::{TombstoneColumnFamilies, TombstoneContext, TOMBSTONE};
use crate::keys;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// Tombstone node data (NODES CF)
pub(super) fn tombstone_node_data(
    batch: &mut WriteBatch,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) {
    let node_key = keys::node_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
        revision,
    );
    batch.put_cf(cfs.nodes, node_key, TOMBSTONE);
}

/// Tombstone path index (PATH_INDEX CF)
pub(super) fn tombstone_path_index(
    batch: &mut WriteBatch,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) {
    let path_key = keys::path_index_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.path,
        revision,
    );
    batch.put_cf(cfs.path_index, path_key, TOMBSTONE);
}

/// Tombstone node-to-path reverse index (NODE_PATH CF)
pub(super) fn tombstone_node_path(
    batch: &mut WriteBatch,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) {
    let node_path_key = keys::node_path_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
        revision,
    );
    batch.put_cf(cfs.node_path, node_path_key, TOMBSTONE);
}

/// Tombstone ordered children entry (ORDERED_CHILDREN CF)
///
/// Only tombstones if node has a parent. Empty order_key is valid.
pub(super) fn tombstone_ordered_children(
    batch: &mut WriteBatch,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) {
    if let Some(ref parent_id) = node.parent {
        // Write tombstone even for empty order_key - empty string is a valid key component
        // and we need to ensure the old entry is properly masked
        let ordered_key = keys::ordered_child_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            parent_id,
            &node.order_key,
            revision,
            &node.id,
        );
        batch.put_cf(cfs.ordered_children, ordered_key, TOMBSTONE);
    }
}
