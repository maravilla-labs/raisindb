//! The versions a merge resolution supersedes, and where each sat in the tree.
//!
//! A resolution is written at the merge revision M, and then
//! `copy_branch_indexes` replays the SOURCE side's index entries into the
//! target at their original revisions. Anything any side ever indexed for the
//! node — the common ancestor's version, the target head's, the source head's —
//! is therefore live in the target keyspace below M unless the resolution
//! tombstones it at M. Tombstoning only the target's old values (what the
//! resolution used to do) left the source side's values live: `keep-ours`
//! still matched a value only the source ever had.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;

/// One version of the node as some side of the merge saw it.
pub(super) struct Superseded {
    pub node: Node,
    /// The branch it was read from.
    pub branch: String,
    /// The revision it was read at.
    pub at: HLC,
}

/// Load a node from a branch as of `max_revision` (inclusive): the newest
/// version at or below it, decoded by the shared decoder (a repository-written
/// `StorageNode` carries no path; it is materialized from `NODE_PATH`).
///
/// `Ok(None)` for "absent or deleted at that revision".
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
    let Some((_, bytes)) =
        crate::mvcc_read::newest_at_or_before(db, cf_nodes, &prefix, Some(max_revision))?
    else {
        return Ok(None);
    };
    if keys::is_tombstone_value(&bytes) {
        return Ok(None);
    }
    crate::mvcc_read::deserialize_node_with_path(
        db,
        &bytes,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        max_revision,
    )
    .map(Some)
}

/// Every version the resolution of `node_id` supersedes: the common
/// ancestor's (read on the target, where the fork put it), the target head's
/// and the source head's. Absent and deleted versions are left out.
#[allow(clippy::too_many_arguments)]
pub(super) fn superseded_versions(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    workspace: &str,
    node_id: &str,
    target: (&str, &HLC),
    source: (&str, &HLC),
    common_ancestor: &HLC,
) -> Result<Vec<Superseded>> {
    let mut out = Vec::new();
    for (branch, at) in [(target.0, common_ancestor), target, source] {
        if let Some(node) = load_node_at(db, tenant_id, repo_id, branch, workspace, node_id, at)? {
            out.push(Superseded {
                node,
                branch: branch.to_string(),
                at: *at,
            });
        }
    }
    Ok(out)
}

/// The ORDERED_CHILDREN parent key of `node` on `branch` as of `at` — the
/// one resolver the delete tombstoner uses too.
pub(super) fn parent_index_id(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    at: Option<&HLC>,
) -> Result<Option<String>> {
    crate::repositories::nodes::parent_index_id(
        db, tenant_id, repo_id, branch, workspace, &node.path, at,
    )
}

/// Where `node` sat in ORDERED_CHILDREN on `branch` as of `at`: its parent
/// key and the label actually STORED there (which can differ from
/// `node.order_key`), falling back to `order_key`. `None` when the parent
/// cannot be resolved.
pub(super) fn order_entry(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    at: Option<&HLC>,
) -> Result<Option<(String, String)>> {
    let Some(parent_id) = parent_index_id(db, tenant_id, repo_id, branch, workspace, node, at)?
    else {
        return Ok(None);
    };
    let label = crate::repositories::nodes::stored_order_label(
        db, tenant_id, repo_id, branch, workspace, &parent_id, &node.id,
    )?
    .or_else(|| (!node.order_key.is_empty()).then(|| node.order_key.clone()));
    Ok(label.map(|label| (parent_id, label)))
}

/// Where a resolved node goes in its parent's editorial order: the origin's
/// stored entry, else its parent on the target with its own `order_key`,
/// else a fresh first label minted at `revision`.
#[allow(clippy::too_many_arguments)]
pub(super) fn placement(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    target_branch: &str,
    workspace: &str,
    node: &Node,
    origin: (&str, &HLC),
    revision: &HLC,
) -> Result<Option<(String, String)>> {
    Ok(
        match order_entry(
            db,
            tenant_id,
            repo_id,
            origin.0,
            workspace,
            node,
            Some(origin.1),
        )? {
            Some(entry) => Some(entry),
            None => parent_index_id(db, tenant_id, repo_id, target_branch, workspace, node, None)?
                .map(|parent| {
                    let label = if node.order_key.is_empty() {
                        crate::fractional_index::format_label(
                            &crate::fractional_index::first(),
                            revision,
                        )
                    } else {
                        node.order_key.clone()
                    };
                    (parent, label)
                }),
        },
    )
}
