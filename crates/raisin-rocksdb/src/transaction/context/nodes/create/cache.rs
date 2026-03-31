//! Read cache management for node creation
//!
//! This module handles updating the transaction's read cache to support
//! read-your-writes semantics within a transaction.

use raisin_error::Result;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;

/// Update read cache for read-your-writes semantics
///
/// Caches the node by both ID and path for fast lookups within the transaction.
/// If an old path is provided (for updates), marks it as deleted in the cache.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node` - The node to cache
/// * `old_path` - Optional old path to mark as deleted (for moves)
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(super) fn update_read_cache(
    tx: &RocksDBTransaction,
    workspace: &str,
    node: &Node,
    old_path: Option<&str>,
) -> Result<()> {
    let mut cache = tx
        .read_cache
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    cache
        .nodes
        .insert((workspace.to_string(), node.id.clone()), Some(node.clone()));
    cache.paths.insert(
        (workspace.to_string(), node.path.clone()),
        Some(node.id.clone()),
    );

    // Mark old path as deleted if changed
    if let Some(old_p) = old_path {
        cache
            .paths
            .insert((workspace.to_string(), old_p.to_string()), None);
    }

    Ok(())
}
