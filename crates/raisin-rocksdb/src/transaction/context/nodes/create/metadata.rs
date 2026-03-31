//! Metadata extraction and normalization for node creation
//!
//! This module provides helper functions for extracting transaction metadata
//! and normalizing node parent fields.

use raisin_error::Result;
use raisin_models::nodes::Node;
use std::sync::Arc;

use crate::transaction::RocksDBTransaction;

/// Normalize the parent field from the path
///
/// Parent should NEVER be null:
/// - Root-level nodes have parent = "/"
/// - Other nodes have parent = parent's name
///
/// # Arguments
///
/// * `node` - The node to normalize
///
/// # Returns
///
/// A new node with normalized parent field
pub(super) fn normalize_parent(node: &Node) -> Node {
    let mut normalized = node.clone();
    normalized.parent = raisin_models::nodes::Node::extract_parent_name_from_path(&node.path);
    normalized
}

/// Extract transaction metadata (tenant, repo, branch)
///
/// Returns Arc-wrapped strings for cheap cloning. Use `as_ref()` or `&**` to get `&str`.
///
/// # Arguments
///
/// * `tx` - The transaction instance
///
/// # Returns
///
/// A tuple of (tenant_id, repo_id, branch) wrapped in Arc for cheap cloning
///
/// # Errors
///
/// Returns error if:
/// - Lock is poisoned
/// - Branch is not set in transaction
pub(super) fn extract_metadata(
    tx: &RocksDBTransaction,
) -> Result<(Arc<String>, Arc<String>, Arc<String>)> {
    let meta = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let branch = meta
        .branch
        .clone()
        .ok_or_else(|| raisin_error::Error::Validation("Branch not set in transaction".into()))?;

    Ok((meta.tenant_id.clone(), meta.repo_id.clone(), branch))
}
