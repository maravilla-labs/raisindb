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

/// Import nodes from a JSON Lines file.
///
/// Each node goes through the batch funnel copy and deep create use
/// (`add_node_to_batch_with_parent_id`): the record (blob + NODE_PATH) through
/// the one record writer, plus PATH_INDEX and the property / reference /
/// relation / spatial entries — this used to write the record alone, so a
/// restored node had no PATH_INDEX entry and `get_by_path` found nothing.
///
/// A node with an empty path is refused, and the whole file with it, before
/// anything is written: asserting `""` in NODE_PATH makes the node
/// unreachable by path with no error anywhere. (A backup taken by a release
/// whose export decoded path-less blobs raw holds exactly such rows.)
pub(super) async fn import_nodes_from_jsonl(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    file: &Path,
) -> Result<()> {
    use std::io::BufRead;

    let file_handle = std::fs::File::open(file)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to open nodes file: {}", e)))?;

    let mut nodes = Vec::new();
    for line in std::io::BufReader::new(file_handle).lines() {
        let line =
            line.map_err(|e| raisin_error::Error::storage(format!("Failed to read line: {}", e)))?;
        let node: Node = serde_json::from_str(&line)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to parse node: {}", e)))?;
        nodes.push(node);
    }
    let pathless: Vec<&str> = nodes
        .iter()
        .filter(|n| n.path.is_empty())
        .map(|n| n.id.as_str())
        .collect();
    if !pathless.is_empty() {
        return Err(raisin_error::Error::Validation(format!(
            "restore refused: {} node(s) in the backup have no path (first: {}); the backup \
             was taken by a release that exported path-less records without their path",
            pathless.len(),
            pathless
                .iter()
                .take(5)
                .copied()
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    // The backup does not record the branch; the workspace is on the node.
    let branch = "main";
    let revision = raisin_hlc::HLC::new(0, 0);
    let mut batch = rocksdb::WriteBatch::default();
    let mut count = 0;
    // Parents and children share unwritten batches: give each record its
    // parent id from the backup itself rather than leaving writers to
    // resolve it from the committed PATH_INDEX.
    let ids_by_path: std::collections::HashMap<(&str, &str), &str> = nodes
        .iter()
        .map(|n| {
            (
                (n.workspace.as_deref().unwrap_or("default"), n.path.as_str()),
                n.id.as_str(),
            )
        })
        .collect();
    for node in &nodes {
        let workspace = node.workspace.as_deref().unwrap_or("default");
        let parent_path = crate::localized_name::sync::parent_path_of(&node.path);
        let parent_id = ids_by_path.get(&(workspace, parent_path.as_str())).copied();
        storage.nodes_impl().add_node_to_batch_with_parent_id(
            &mut batch,
            node,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &revision,
            None,
            parent_id,
            crate::repositories::nodes::PropertyWrite::CREATE,
        )?;
        count += 1;

        // Commit batch every 1000 nodes
        if count % 1000 == 0 {
            storage
                .db()
                .write(std::mem::take(&mut batch))
                .map_err(|e| raisin_error::Error::storage(format!("Batch write failed: {}", e)))?;
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
