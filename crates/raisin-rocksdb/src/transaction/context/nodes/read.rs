//! Node read operations
//!
//! This module contains the implementation of node read operations for transactions:
//! - `get_node`: Get a node by ID with read-your-writes semantics
//! - `get_node_by_path`: Get a node by path with read-your-writes semantics
//!
//! # Key Features
//!
//! ## Read-Your-Writes Semantics
//!
//! All read operations check the in-memory cache first, ensuring that uncommitted
//! changes made earlier in the transaction are visible to later operations.
//!
//! ## MVCC Read
//!
//! Reads the latest version of the node at or before the branch HEAD.
//! Skips tombstone markers to respect deletions.
//!
//! ## StorageNode Compatibility
//!
//! Supports both old (Node with path) and new (StorageNode without path) formats.
//! Path is materialized from NODE_PATH index when needed.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchRepository;
use rocksdb::DB;
use std::sync::Arc;

use crate::transaction::types::is_tombstone;
use crate::transaction::RocksDBTransaction;
use crate::StorageNode;
use crate::{cf, cf_handle, keys};

/// Materialize path from NODE_PATH index
///
/// Used when reading nodes stored as StorageNode (without path).
fn materialize_path(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    target_revision: &HLC,
) -> Result<String> {
    let prefix = keys::node_path_key_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let cf = cf_handle(db, cf::NODE_PATH)?;

    let iter = db.prefix_iterator_cf(cf, prefix.clone());

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        if !key.starts_with(&prefix) {
            break;
        }

        let revision = match keys::extract_revision_from_key(&key) {
            Ok(rev) => rev,
            Err(_) => continue,
        };

        if &revision > target_revision {
            continue;
        }

        // Check for tombstone - node was deleted at this revision
        if is_tombstone(&value) {
            return Err(raisin_error::Error::storage(format!(
                "Node {} was deleted (tombstone in NODE_PATH)",
                node_id
            )));
        }

        let path = String::from_utf8(value.to_vec())
            .map_err(|e| raisin_error::Error::storage(format!("Invalid path encoding: {}", e)))?;

        return Ok(path);
    }

    Err(raisin_error::Error::storage(format!(
        "Path not found for node_id={} at revision={}",
        node_id, target_revision
    )))
}

/// Deserialize node with path materialization support
///
/// Handles both old (Node) and new (StorageNode) formats.
fn deserialize_node_with_path(
    db: &Arc<DB>,
    bytes: &[u8],
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    target_revision: &HLC,
) -> Result<Node> {
    // Debug: log the raw bytes being deserialized
    let first_bytes: Vec<u8> = bytes.iter().take(20).copied().collect();
    tracing::debug!(
        node_id = %node_id,
        bytes_len = bytes.len(),
        first_bytes = ?first_bytes,
        "Attempting to deserialize node"
    );

    // Try StorageNode first with path materialization
    if let Ok(storage_node) = rmp_serde::from_slice::<StorageNode>(bytes) {
        match materialize_path(
            db,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            target_revision,
        ) {
            Ok(path) => {
                return Ok(storage_node.into_node(path));
            }
            Err(_) => {
                // Path materialization failed - try Node format
            }
        }
    }

    // Fallback: try Node format
    let node: Node = rmp_serde::from_slice(bytes).map_err(|e| {
        // Error: log detailed info when deserialization fails
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

/// Get a node by ID with read-your-writes semantics
///
/// Checks the read cache first to ensure uncommitted changes are visible.
///
/// # MVCC Read
///
/// Reads the latest version of the node at or before the branch HEAD.
/// Skips tombstone markers to respect deletions.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node_id` - The ID of the node to read
///
/// # Returns
///
/// Ok(Some(node)) if found, Ok(None) if not found or deleted
pub async fn get_node(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
) -> Result<Option<Node>> {
    // Check read cache first for read-your-writes semantics
    {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let cache_key = (workspace.to_string(), node_id.to_string());
        if let Some(cached) = cache.nodes.get(&cache_key) {
            return Ok(cached.clone());
        }
    }

    // 1. Get metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // 2. Get HEAD revision
    let head_revision = tx
        .branch_repo
        .get_branch(&tenant_id, &repo_id, &branch)
        .await?
        .ok_or_else(|| raisin_error::Error::NotFound(format!("Branch {} not found", branch)))?
        .head;

    // 3. Build key prefix for this node across all revisions
    let cf_nodes = cf_handle(&tx.db, cf::NODES)?;
    let prefix = keys::node_key_prefix(&tenant_id, &repo_id, &branch, workspace, node_id);

    // 4. Iterate to find latest version <= HEAD
    let iter = tx.db.prefix_iterator_cf(cf_nodes, &prefix);

    for item in iter {
        let (key, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // Debug: log key/value info for each entry
        tracing::debug!(
            node_id = %node_id,
            key_len = key.len(),
            value_len = value.len(),
            key_as_string = %String::from_utf8_lossy(&key),
            "Processing NODES CF entry"
        );

        // Decode revision from key (last 16 bytes for HLC)
        if key.len() < 16 {
            tracing::warn!(
                node_id = %node_id,
                key_len = key.len(),
                key_as_string = %String::from_utf8_lossy(&key),
                "Skipping NODES entry with key shorter than 16 bytes"
            );
            continue;
        }
        let rev_bytes = &key[key.len() - 16..];
        let revision = keys::decode_descending_revision(rev_bytes)
            .map_err(|e| raisin_error::Error::storage(format!("Revision decode error: {}", e)))?;

        // Only consider revisions at or before HEAD
        if revision > head_revision {
            continue;
        }

        // Check if it's a tombstone (deleted node) - single byte 'T'
        if value.as_ref() == b"T" {
            // Tombstone - node is deleted
            return Ok(None);
        }

        // Deserialize with StorageNode/Node compatibility
        let node = deserialize_node_with_path(
            &tx.db, &value, &tenant_id, &repo_id, &branch, workspace, node_id, &revision,
        )?;

        // RLS check - SECURITY: deny-by-default if no auth context
        {
            let meta = tx
                .metadata
                .lock()
                .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
            match &meta.auth_context {
                Some(auth) => {
                    use raisin_core::services::rls_filter;
                    use raisin_models::permissions::{Operation, PermissionScope};

                    let branch_str = meta.branch.as_ref().map(|s| s.as_str()).unwrap_or("main");
                    let scope = PermissionScope::new(workspace, branch_str);

                    if !rls_filter::can_perform(&node, Operation::Read, auth, &scope) {
                        tracing::debug!(
                            node_id = %node_id,
                            workspace = %workspace,
                            "RLS: denying read access to node"
                        );
                        return Ok(None);
                    }
                }
                None => {
                    // SECURITY: Deny read if no auth context set on transaction
                    tracing::warn!(
                        node_id = %node_id,
                        workspace = %workspace,
                        "Transaction has no auth context - denying get_node read"
                    );
                    return Ok(None);
                }
            }
        }

        return Ok(Some(node));
    }

    Ok(None)
}

/// Get a node by path with read-your-writes semantics
///
/// Checks the read cache first to ensure uncommitted changes are visible.
///
/// # Path Resolution
///
/// 1. Queries PATH_INDEX to get node_id
/// 2. Calls get_node to read the node data
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `path` - The path of the node to read
///
/// # Returns
///
/// Ok(Some(node)) if found, Ok(None) if not found or deleted
pub async fn get_node_by_path(
    tx: &RocksDBTransaction,
    workspace: &str,
    path: &str,
) -> Result<Option<Node>> {
    // Check read cache first for read-your-writes semantics
    let cached_node_id = {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        let cache_key = (workspace.to_string(), path.to_string());
        cache.paths.get(&cache_key).cloned()
    }; // Drop lock here before async call

    if let Some(node_id_opt) = cached_node_id {
        if let Some(node_id) = node_id_opt {
            // Path found, now get the node (which will also check cache)
            return get_node(tx, workspace, &node_id).await;
        } else {
            // Path was explicitly deleted in this transaction
            return Ok(None);
        }
    }

    // 1. Get metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // 2. Get HEAD revision for filtering PATH_INDEX entries
    // CRITICAL: This ensures we only see nodes that are visible at the current HEAD
    let head_revision = tx
        .branch_repo
        .get_branch(&tenant_id, &repo_id, &branch)
        .await?
        .ok_or_else(|| raisin_error::Error::NotFound(format!("Branch {} not found", branch)))?
        .head;

    // 3. Query path index to get node_id
    let cf_path = cf_handle(&tx.db, cf::PATH_INDEX)?;
    let prefix = keys::path_index_key_prefix(&tenant_id, &repo_id, &branch, workspace, path);

    let iter = tx.db.prefix_iterator_cf(cf_path, &prefix);

    tracing::debug!(
        "TX get_node_by_path: workspace={}, path={}, prefix_len={}, head_revision={}",
        workspace,
        path,
        prefix.len(),
        head_revision
    );

    // Find the first (newest) NON-tombstone entry AT OR BEFORE HEAD revision
    // Keys are sorted in descending revision order (newest first)
    for item in iter {
        let (key, value) =
            item.map_err(|e| raisin_error::Error::storage(format!("Iterator error: {}", e)))?;

        // CRITICAL: Verify key actually starts with our prefix
        // This prevents false matches when paths share common prefixes
        if !key.starts_with(&prefix) {
            tracing::debug!("TX get_node_by_path: key doesn't match prefix, stopping iteration");
            break;
        }

        // CRITICAL: Extract revision from key and filter by HEAD
        // PATH_INDEX keys have descending revision as the last component
        let revision = match keys::extract_revision_from_key(&key) {
            Ok(rev) => rev,
            Err(e) => {
                tracing::warn!(
                    "TX get_node_by_path: failed to extract revision from key: {}",
                    e
                );
                continue;
            }
        };

        // Skip entries with revision > HEAD (not yet visible)
        if revision > head_revision {
            tracing::debug!(
                "TX get_node_by_path: skipping entry with revision {} > head {}",
                revision,
                head_revision
            );
            continue;
        }

        tracing::debug!(
            "TX get_node_by_path: found key_len={}, value_len={}, revision={}, is_tombstone={}",
            key.len(),
            value.len(),
            revision,
            is_tombstone(&value)
        );

        // Check for tombstone - path was deleted, return None
        if is_tombstone(&value) {
            tracing::debug!(
                "TX get_node_by_path: tombstone found for path={}, node is deleted",
                path
            );
            return Ok(None);
        }

        // Found the node ID
        let node_id = String::from_utf8(value.to_vec())
            .map_err(|e| raisin_error::Error::storage(format!("Invalid node ID: {}", e)))?;

        tracing::debug!(
            "TX get_node_by_path: found node_id={} at revision={}",
            node_id,
            revision
        );

        // Now get the actual node
        return get_node(tx, workspace, &node_id).await;
    }

    tracing::debug!("TX get_node_by_path: no node found for path={}", path);

    Ok(None)
}
