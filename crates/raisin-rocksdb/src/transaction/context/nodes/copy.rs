//! Node copy operations
//!
//! This module contains the implementation of node copy operations for transactions:
//! - `copy_node_tree`: Copy an entire node tree
//!
//! # Key Features
//!
//! ## Delegation to NodeRepository
//!
//! The copy operation delegates to NodeRepository's copy_node_tree which handles:
//! - Recursive copying of all descendants
//! - ID mapping and reference rewriting
//! - Fractional index preservation
//! - Atomic transaction handling

use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, StorageScope};

use crate::transaction::RocksDBTransaction;

/// Copy an entire node tree
///
/// Delegates to NodeRepository's copy_node_tree which handles:
/// - Recursive copying of all descendants
/// - ID mapping and reference rewriting
/// - Fractional index preservation
/// - Atomic transaction handling
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the nodes
/// * `source_path` - The path of the source node to copy
/// * `target_parent` - The path of the target parent
/// * `new_name` - Optional new name for the copied root node
/// * `_actor` - The actor performing the operation (unused in transaction)
///
/// # Returns
///
/// Ok(Node) with the copied root node
pub async fn copy_node_tree(
    tx: &RocksDBTransaction,
    workspace: &str,
    source_path: &str,
    target_parent: &str,
    new_name: Option<&str>,
    _actor: &str,
) -> Result<Node> {
    // Get transaction metadata
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

    // Delegate to storage layer's optimized copy_node_tree implementation
    // This handles all recursion, ID mapping, fractional index preservation, and atomicity
    // Pass None for operation_meta - the storage layer will handle revision allocation and metadata
    let node_repo = tx.node_repo.as_ref();
    node_repo
        .copy_node_tree(
            StorageScope::new(&tenant_id, &repo_id, &branch, workspace),
            source_path,
            target_parent,
            new_name,
            None, // Let storage layer handle revision allocation and operation metadata
        )
        .await
}
