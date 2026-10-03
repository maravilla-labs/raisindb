//! Core tombstone functions: node data, path index, node path, ordered children

use super::{TombstoneColumnFamilies, TombstoneContext, TOMBSTONE};
use crate::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

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
/// The entry lives under the parent's ID (`/` for a root child) and the label
/// actually STORED for this child, which can differ from `node.order_key`
/// (legacy drift, merge verbatim copies). This used to key the tombstone by
/// `node.parent` — the parent's NAME — so it landed under a parent no entry
/// lives under, and every deleted child stayed listed.
///
/// `parent_index_id` is the caller's parent id when it has one (the replicated
/// delete carries it); otherwise it is resolved from PATH_INDEX by the node's
/// parent path. With no resolvable parent nothing is written.
pub(super) fn tombstone_ordered_children(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
    parent_index_id: Option<&str>,
) -> Result<()> {
    let parent_id = match parent_index_id {
        Some(id) => Some(id.to_string()),
        None => crate::repositories::nodes::parent_index_id(
            db,
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &node.path,
            None,
        )?,
    };
    let Some(parent_id) = parent_id else {
        tracing::debug!(
            node_id = %node.id,
            path = %node.path,
            "delete: no parent id resolvable for the ORDERED_CHILDREN tombstone"
        );
        return Ok(());
    };
    // Empty order_key is a valid key component, so the fallback still masks
    // an entry written under it.
    let label = crate::repositories::nodes::stored_order_label(
        db,
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &parent_id,
        &node.id,
    )?
    .unwrap_or_else(|| node.order_key.clone());
    let ordered_key = keys::ordered_child_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &parent_id,
        &label,
        revision,
        &node.id,
    );
    batch.put_cf(cfs.ordered_children, ordered_key, TOMBSTONE);
    Ok(())
}
