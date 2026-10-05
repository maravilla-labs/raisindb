//! Export functions for backup (nodes, branches, workspaces, revisions, trees, nodetypes)

use crate::management::async_indexing::node_key_parse::parse_node_key;
use crate::repositories::nodes::helpers::is_tombstone;
use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_context::Branch;
use raisin_error::Result;
use raisin_models::{
    nodes::{types::NodeType, Node},
    workspace::Workspace,
};
use raisin_storage::RevisionMeta;
use serde::{Deserialize, Serialize};

/// Tree backup entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TreeBackupEntry {
    pub tree_id_hex: String,
    pub entries: Vec<raisin_models::tree::TreeEntry>,
}

/// Export every node of a repository: on each branch, the version of each
/// node visible at that branch's HEAD (a node deleted there is skipped),
/// decoded through the ONE node decoder so its path comes from the read rule.
///
/// A `StorageNode` blob — what every writer stores since Phase 10b — embeds
/// no path, and `Node.path` is `#[serde(default)]`: decoding the blob raw as
/// a `Node` exported `"path": ""` for every such node, and a restore then
/// wrote that empty path back. Every version of every node used to be
/// exported, too, and the import (which writes them all at one revision)
/// kept whichever came last — the OLDEST.
pub(super) async fn export_all_repository_nodes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<Node>> {
    let db = storage.db();
    let cf_nodes = cf_handle(db, cf::NODES)?;
    let mut nodes = Vec::new();

    for branch in export_branches(storage, tenant_id, repo_id).await? {
        let prefix = keys::branch_prefix(tenant_id, repo_id, &branch.name);
        // The node whose visible version was already decided; its older
        // versions follow it in key order (revisions descend).
        let mut decided: Option<(String, String)> = None;
        for item in crate::prefix_scan(db, cf_nodes, &prefix) {
            let (key, value) =
                item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;
            let Some((workspace, node_id, revision)) = parse_node_key(&prefix, &key) else {
                continue;
            };
            if revision > branch.head {
                continue;
            }
            if decided
                .as_ref()
                .is_some_and(|(w, i)| w == workspace && i == node_id)
            {
                continue;
            }
            decided = Some((workspace.to_string(), node_id.to_string()));
            if is_tombstone(&value) {
                continue;
            }
            match crate::mvcc_read::deserialize_node_with_path(
                db,
                &value,
                tenant_id,
                repo_id,
                &branch.name,
                workspace,
                node_id,
                &branch.head,
                &revision,
            ) {
                Ok(mut node) => {
                    node.workspace = Some(workspace.to_string());
                    nodes.push(node);
                }
                Err(e) => {
                    tracing::warn!(node_id, workspace, error = %e, "backup: failed to decode node");
                }
            }
        }
    }

    Ok(nodes)
}

/// Export all branches
pub(super) async fn export_branches(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<Branch>> {
    let cf_branches = cf_handle(storage.db(), cf::BRANCHES)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("branches")
        .build_prefix();

    let mut branches = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_branches, &prefix);

    for item in iter {
        let (_, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        if let Ok(branch) = rmp_serde::from_slice::<Branch>(&value) {
            branches.push(branch);
        }
    }

    Ok(branches)
}

/// Export all workspaces
pub(super) async fn export_workspaces(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<Workspace>> {
    let cf_workspaces = cf_handle(storage.db(), cf::WORKSPACES)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("workspaces")
        .build_prefix();

    let mut workspaces = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_workspaces, &prefix);

    for item in iter {
        let (_, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        if let Ok(workspace) = rmp_serde::from_slice::<Workspace>(&value) {
            workspaces.push(workspace);
        }
    }

    Ok(workspaces)
}

/// Export all revisions
pub(super) async fn export_revisions(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<RevisionMeta>> {
    let cf_revisions = cf_handle(storage.db(), cf::REVISIONS)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("revisions")
        .build_prefix();

    let mut revisions = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_revisions, &prefix);

    for item in iter {
        let (_, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        if let Ok(revision) = rmp_serde::from_slice::<RevisionMeta>(&value) {
            revisions.push(revision);
        }
    }

    Ok(revisions)
}

/// Export all NodeTypes
pub(super) async fn export_nodetypes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<NodeType>> {
    let cf_nodetypes = cf_handle(storage.db(), cf::NODE_TYPES)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("nodetypes")
        .build_prefix();

    let mut nodetypes = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_nodetypes, &prefix);

    for item in iter {
        let (_, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        if let Ok(nodetype) = rmp_serde::from_slice::<NodeType>(&value) {
            nodetypes.push(nodetype);
        }
    }

    Ok(nodetypes)
}

/// Export all trees
pub(super) async fn export_trees(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<([u8; 32], Vec<raisin_models::tree::TreeEntry>)>> {
    let cf_trees = cf_handle(storage.db(), cf::TREES)?;
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("trees")
        .build_prefix();

    let mut trees = Vec::new();
    let iter = crate::prefix_scan(storage.db(), cf_trees, &prefix);

    for item in iter {
        let (key, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // Extract tree ID from key
        let key_str = String::from_utf8_lossy(&key);
        if let Some(tree_id_hex) = key_str.split('\0').next_back() {
            if let Ok(tree_id_bytes) = hex::decode(tree_id_hex) {
                if tree_id_bytes.len() == 32 {
                    let mut tree_id = [0u8; 32];
                    tree_id.copy_from_slice(&tree_id_bytes);

                    if let Ok(entries) =
                        rmp_serde::from_slice::<Vec<raisin_models::tree::TreeEntry>>(&value)
                    {
                        trees.push((tree_id, entries));
                    }
                }
            }
        }
    }

    Ok(trees)
}
