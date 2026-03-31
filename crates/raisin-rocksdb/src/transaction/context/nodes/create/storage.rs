//! Storage operations for node creation
//!
//! This module handles writing node data and path indexes to the transaction batch.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Write node to batch with versioned key
///
/// Serializes the node and writes it to the transaction batch.
/// Returns the node key for conflict tracking.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node to write
/// * `revision` - The HLC revision for versioning
///
/// # Returns
///
/// The node key bytes for conflict tracking
///
/// # Errors
///
/// Returns error if:
/// - Lock is poisoned
/// - Serialization fails
pub(super) fn write_node_to_batch(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<Vec<u8>> {
    let cf_nodes = cf_handle(&tx.db, cf::NODES)?;

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let node_key =
        keys::node_key_versioned(tenant_id, repo_id, branch, workspace, &node.id, revision);
    let node_value = rmp_serde::to_vec_named(node)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    batch.put_cf(cf_nodes, node_key.clone(), node_value);

    Ok(node_key)
}

/// Write path index to batch
///
/// Creates a path -> node_id mapping in the index.
/// Optionally tombstones an old path if the node was moved.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `path` - The new path to index
/// * `node_id` - The node ID
/// * `revision` - The HLC revision for versioning
/// * `tombstone_old_path` - Optional old path to tombstone
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(super) fn write_path_index(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    node_id: &str,
    revision: &HLC,
    tombstone_old_path: Option<&str>,
) -> Result<()> {
    let cf_path = cf_handle(&tx.db, cf::PATH_INDEX)?;

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    // Tombstone old path if needed
    if let Some(old_path) = tombstone_old_path {
        let old_path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, old_path, revision,
        );
        batch.put_cf(cf_path, old_path_key, b"T");
    }

    // Write new path index
    let path_key =
        keys::path_index_key_versioned(tenant_id, repo_id, branch, workspace, path, revision);
    batch.put_cf(cf_path, path_key, node_id.as_bytes());

    Ok(())
}
