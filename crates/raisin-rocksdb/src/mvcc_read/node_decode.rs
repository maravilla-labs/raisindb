//! Decoding one `NODES` blob into a `Node`.
//!
//! The ONE decoder for a raw node blob read outside the repository (the
//! transaction read path and the replication apply path's baseline). Two blob
//! formats live in `NODES`: a `StorageNode`, which carries no path — the path
//! is materialized from `NODE_PATH` at the version's revision — and the legacy
//! full `Node`, which carries its own. A reader that decoded every blob as a
//! `Node` got `path == ""` for every repository-written node.

use crate::StorageNode;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;

/// Decode `bytes`, the `NODES` value of `node_id`'s version at
/// `target_revision`, materializing a `StorageNode`'s path from `NODE_PATH`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn deserialize_node_with_path(
    db: &DB,
    bytes: &[u8],
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    target_revision: &HLC,
) -> Result<Node> {
    tracing::trace!(
        node_id = %node_id,
        bytes_len = bytes.len(),
        "Attempting to deserialize node"
    );

    // Try StorageNode first with path materialization
    if let Ok(storage_node) = rmp_serde::from_slice::<StorageNode>(bytes) {
        if let Ok(path) = super::materialize_path(
            db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            target_revision,
        ) {
            return Ok(storage_node.into_node(path));
        }
        // Path materialization failed - try Node format
    }

    // Fallback: try Node format
    let node: Node = rmp_serde::from_slice(bytes).map_err(|e| {
        // Error: log detailed info when deserialization fails. The byte dump
        // is built only here — it used to be allocated on every read.
        let first_bytes: Vec<u8> = bytes.iter().take(20).copied().collect();
        let as_string = String::from_utf8_lossy(&bytes[..std::cmp::min(100, bytes.len())]);
        tracing::error!(
            node_id = %node_id,
            workspace = %workspace,
            bytes_len = bytes.len(),
            first_bytes = ?first_bytes,
            as_string = %as_string,
            error = %e,
            "Failed to deserialize node - raw bytes shown"
        );
        raisin_error::Error::storage(format!("Deserialization error: {}", e))
    })?;

    Ok(node)
}
