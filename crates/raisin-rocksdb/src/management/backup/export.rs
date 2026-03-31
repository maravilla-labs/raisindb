//! Export functions for backup (nodes, branches, workspaces, revisions, trees, nodetypes)

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

/// Export all nodes from a repository
pub(super) async fn export_all_repository_nodes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Vec<Node>> {
    let cf_nodes = cf_handle(storage.db(), cf::NODES)?;
    let prefix = keys::repo_prefix(tenant_id, repo_id);

    let mut nodes = Vec::new();
    let iter = storage.db().prefix_iterator_cf(cf_nodes, &prefix);

    for item in iter {
        let (key, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        let key_str = String::from_utf8_lossy(&key);
        if !key_str.contains("\0nodes\0") {
            continue;
        }

        if !value.is_empty() {
            match rmp_serde::from_slice::<Node>(&value) {
                Ok(node) => nodes.push(node),
                Err(e) => {
                    tracing::warn!("Failed to deserialize node: {}", e);
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
    let iter = storage.db().prefix_iterator_cf(cf_branches, &prefix);

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
    let iter = storage.db().prefix_iterator_cf(cf_workspaces, &prefix);

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
    let iter = storage.db().prefix_iterator_cf(cf_revisions, &prefix);

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
    let iter = storage.db().prefix_iterator_cf(cf_nodetypes, &prefix);

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
    let iter = storage.db().prefix_iterator_cf(cf_trees, &prefix);

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
