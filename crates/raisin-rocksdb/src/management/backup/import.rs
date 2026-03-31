//! Import functions for restore (nodes, branches, workspaces, revisions, trees, nodetypes)

use super::export::TreeBackupEntry;
use crate::{cf, cf_handle, keys, RocksDBStorage};
use raisin_context::Branch;
use raisin_error::Result;
use raisin_models::{
    nodes::{types::NodeType, Node},
    workspace::Workspace,
};
use raisin_storage::RevisionMeta;
use std::path::Path;

/// Import nodes from JSON Lines file
pub(super) async fn import_nodes_from_jsonl(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    file: &Path,
) -> Result<()> {
    use std::io::BufRead;

    let file_handle = std::fs::File::open(file)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to open nodes file: {}", e)))?;

    let reader = std::io::BufReader::new(file_handle);
    let cf_nodes = cf_handle(storage.db(), cf::NODES)?;
    let mut batch = rocksdb::WriteBatch::default();
    let mut count = 0;

    for line in reader.lines() {
        let line =
            line.map_err(|e| raisin_error::Error::storage(format!("Failed to read line: {}", e)))?;

        let node: Node = serde_json::from_str(&line)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to parse node: {}", e)))?;

        // We need to extract branch and workspace from the node's context
        // For now, we'll use default values - in a real implementation,
        // this information should be in the backup metadata
        let branch = "main"; // Default branch
        let workspace = "default"; // Default workspace
        let revision = raisin_hlc::HLC::new(0, 0); // Default revision

        let key =
            keys::node_key_versioned(tenant_id, repo_id, branch, workspace, &node.id, &revision);

        let value = rmp_serde::to_vec_named(&node).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize node: {}", e))
        })?;

        batch.put_cf(cf_nodes, key, value);
        count += 1;

        // Commit batch every 1000 nodes
        if count % 1000 == 0 {
            storage
                .db()
                .write(batch)
                .map_err(|e| raisin_error::Error::storage(format!("Batch write failed: {}", e)))?;
            batch = rocksdb::WriteBatch::default();
            tracing::debug!("Imported {} nodes", count);
        }
    }

    // Commit remaining nodes
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch write failed: {}", e))
        })?;
    }

    tracing::info!("Imported {} nodes", count);
    Ok(())
}

/// Import branches from JSON file
pub(super) async fn import_branches(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    file: &Path,
) -> Result<()> {
    let branches: Vec<Branch> = rmp_serde::from_slice(&std::fs::read(file).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to read branches file: {}", e))
    })?)
    .map_err(|e| raisin_error::Error::storage(format!("Failed to parse branches: {}", e)))?;

    let cf_branches = cf_handle(storage.db(), cf::BRANCHES)?;
    let mut batch = rocksdb::WriteBatch::default();

    for branch in branches {
        let key = keys::branch_key(tenant_id, repo_id, &branch.name);
        let value = rmp_serde::to_vec(&branch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize branch: {}", e))
        })?;

        batch.put_cf(cf_branches, key, value);
    }

    storage
        .db()
        .write(batch)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to write branches: {}", e)))?;

    Ok(())
}

/// Import workspaces from JSON file
pub(super) async fn import_workspaces(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    file: &Path,
) -> Result<()> {
    let workspaces: Vec<Workspace> = rmp_serde::from_slice(&std::fs::read(file).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to read workspaces file: {}", e))
    })?)
    .map_err(|e| raisin_error::Error::storage(format!("Failed to parse workspaces: {}", e)))?;

    let cf_workspaces = cf_handle(storage.db(), cf::WORKSPACES)?;
    let mut batch = rocksdb::WriteBatch::default();

    for workspace in workspaces {
        let key = keys::workspace_key(tenant_id, repo_id, &workspace.name);
        let value = rmp_serde::to_vec_named(&workspace).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize workspace: {}", e))
        })?;

        batch.put_cf(cf_workspaces, key, value);
    }

    storage
        .db()
        .write(batch)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to write workspaces: {}", e)))?;

    Ok(())
}

/// Import revisions from JSON file
pub(super) async fn import_revisions(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    file: &Path,
) -> Result<()> {
    let revisions: Vec<RevisionMeta> =
        rmp_serde::from_slice(&std::fs::read(file).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to read revisions file: {}", e))
        })?)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to parse revisions: {}", e)))?;

    let cf_revisions = cf_handle(storage.db(), cf::REVISIONS)?;
    let mut batch = rocksdb::WriteBatch::default();

    for revision in revisions {
        let key = keys::revision_meta_key(tenant_id, repo_id, &revision.revision);
        let value = rmp_serde::to_vec(&revision).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize revision: {}", e))
        })?;

        batch.put_cf(cf_revisions, key, value);
    }

    storage
        .db()
        .write(batch)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to write revisions: {}", e)))?;

    Ok(())
}

/// Import NodeTypes from JSON file
pub(super) async fn import_nodetypes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    file: &Path,
) -> Result<()> {
    let nodetypes: Vec<NodeType> = rmp_serde::from_slice(&std::fs::read(file).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to read nodetypes file: {}", e))
    })?)
    .map_err(|e| raisin_error::Error::storage(format!("Failed to parse nodetypes: {}", e)))?;

    let cf_nodetypes = cf_handle(storage.db(), cf::NODE_TYPES)?;
    let mut batch = rocksdb::WriteBatch::default();

    for nodetype in nodetypes {
        let key = keys::nodetype_key(tenant_id, repo_id, &nodetype.name);
        let value = rmp_serde::to_vec(&nodetype).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize nodetype: {}", e))
        })?;

        batch.put_cf(cf_nodetypes, key, value);
    }

    storage
        .db()
        .write(batch)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to write nodetypes: {}", e)))?;

    Ok(())
}

/// Import trees from JSON Lines file
pub(super) async fn import_trees(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    file: &Path,
) -> Result<()> {
    use std::io::BufRead;

    let file_handle = std::fs::File::open(file)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to open trees file: {}", e)))?;

    let reader = std::io::BufReader::new(file_handle);
    let cf_trees = cf_handle(storage.db(), cf::TREES)?;
    let mut batch = rocksdb::WriteBatch::default();
    let mut count = 0;

    for line in reader.lines() {
        let line =
            line.map_err(|e| raisin_error::Error::storage(format!("Failed to read line: {}", e)))?;

        let tree_entry: TreeBackupEntry = serde_json::from_str(&line)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to parse tree: {}", e)))?;

        let tree_id_bytes = hex::decode(&tree_entry.tree_id_hex).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to decode tree ID: {}", e))
        })?;

        if tree_id_bytes.len() != 32 {
            return Err(raisin_error::Error::storage(
                "Invalid tree ID length".to_string(),
            ));
        }

        let mut tree_id = [0u8; 32];
        tree_id.copy_from_slice(&tree_id_bytes);

        let key = keys::tree_key(tenant_id, repo_id, &tree_id);
        let value = rmp_serde::to_vec(&tree_entry.entries).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize tree entries: {}", e))
        })?;

        batch.put_cf(cf_trees, key, value);
        count += 1;

        // Commit batch every 1000 trees
        if count % 1000 == 0 {
            storage
                .db()
                .write(batch)
                .map_err(|e| raisin_error::Error::storage(format!("Batch write failed: {}", e)))?;
            batch = rocksdb::WriteBatch::default();
        }
    }

    // Commit remaining trees
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch write failed: {}", e))
        })?;
    }

    tracing::info!("Imported {} trees", count);
    Ok(())
}
