//! Node list operations
//!
//! This module contains the implementation of node list operations for transactions:
//! - `list_children`: List ordered children of a parent node
//! - `scan_nodes`: Scan all nodes in a workspace (for management operations)
//!
//! # Key Features
//!
//! ## Delegation to NodeRepository
//!
//! The list operations delegate to NodeRepository's corresponding methods which use
//! the appropriate indexes for efficient querying.

use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{ListOptions, NodeRepository, StorageScope};

use crate::transaction::RocksDBTransaction;

/// List ordered children of a parent node
///
/// Delegates to NodeRepository's list_children which uses the ORDERED_CHILDREN index.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the nodes
/// * `parent_path` - The path of the parent node
///
/// # Returns
///
/// Ok(Vec<Node>) with children in fractional index order
pub async fn list_children(
    tx: &RocksDBTransaction,
    workspace: &str,
    parent_path: &str,
) -> Result<Vec<Node>> {
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

    // Delegate to node repository's list_children trait method
    // This uses the ORDERED_CHILDREN index to get children in fractional index order
    let node_repo = tx.node_repo.as_ref();
    node_repo
        .list_children(
            StorageScope::new(&tenant_id, &repo_id, &branch, workspace),
            parent_path,
            raisin_storage::ListOptions::default(),
        )
        .await
}

/// Scan all nodes in a workspace (collects all into memory)
///
/// This is used for management operations like re-indexing and integrity checks
/// that need to iterate over all nodes in a workspace.
///
/// For bulk UPDATE/DELETE operations with complex WHERE clauses, use the SQL
/// execution engine which leverages optimized SELECT queries to find matching
/// nodes efficiently (via property indexes, full-text search, etc.) before updating.
///
/// # Warning
///
/// This loads ALL nodes into memory at once. For large datasets (100K+ nodes),
/// this can cause high memory usage.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace to scan
///
/// # Returns
///
/// Ok(Vec<Node>) with all nodes in the workspace
pub async fn scan_nodes(tx: &RocksDBTransaction, workspace: &str) -> Result<Vec<Node>> {
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

    // Delegate to node repository's list_all trait method
    let node_repo = tx.node_repo.as_ref();
    node_repo
        .list_all(
            StorageScope::new(&tenant_id, &repo_id, &branch, workspace),
            ListOptions::default(),
        )
        .await
}
