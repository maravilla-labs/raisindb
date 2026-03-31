//! Node reorder operations
//!
//! This module contains the implementation of node reorder operations for transactions:
//! - `reorder_child_before`: Move a child node to appear before another sibling
//! - `reorder_child_after`: Move a child node to appear after another sibling
//!
//! # Key Features
//!
//! ## Delegation to NodeRepository
//!
//! The reorder operations delegate to NodeRepository's corresponding methods which handle:
//! - Fractional index calculation for O(1) reordering
//! - MVCC tombstones for old positions
//! - Per-parent locking to prevent concurrent modification races
//! - Revision allocation and branch HEAD updates
//!
//! ## Change Tracking
//!
//! After reordering, the operation is tracked in the changeset as `ChangeOperation::Reordered`.

use raisin_error::Result;
use raisin_storage::{NodeRepository, StorageScope};

use super::create::track_reorder;
use crate::transaction::RocksDBTransaction;

/// Reorder a child node to appear before another sibling
///
/// Delegates to NodeRepository's move_child_before which uses fractional indexing.
/// Tracks the reorder operation in the changeset.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the nodes
/// * `parent_path` - The path of the parent node
/// * `child_name` - The name of the child to move
/// * `before_child_name` - The name of the sibling to position before
///
/// # Returns
///
/// Ok(()) on success
pub async fn reorder_child_before(
    tx: &RocksDBTransaction,
    workspace: &str,
    parent_path: &str,
    child_name: &str,
    before_child_name: &str,
) -> Result<()> {
    let (tenant_id, repo_id, branch, actor, message) = {
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
            meta.actor.clone(),
            meta.message.clone(),
        )
    };

    // Get the child node info before reordering (for change tracking)
    let child_path = if parent_path == "/" {
        format!("/{}", child_name)
    } else {
        format!("{}/{}", parent_path, child_name)
    };

    let node_repo = tx.node_repo.as_ref();

    let scope = StorageScope::new(&tenant_id, &repo_id, &branch, workspace);

    // Get node info for tracking
    let node_info = node_repo.get_by_path(scope, &child_path, None).await?;

    // Delegate to node repository's move_child_before trait method
    // This handles fractional indexing, MVCC, and atomic writes
    node_repo
        .move_child_before(
            scope,
            parent_path,
            child_name,
            before_child_name,
            message.as_ref().map(|s| s.as_str()),
            actor.as_ref().map(|s| s.as_str()),
        )
        .await?;

    // Track the reorder operation in the changeset
    if let Some(node) = node_info {
        // Use the transaction's unified revision for consistency with other operations
        let revision = tx.get_or_allocate_transaction_revision()?;
        track_reorder(
            tx,
            workspace,
            &node.id,
            &child_path,
            &node.node_type,
            "".to_string(), // Old order label not easily available
            "".to_string(), // New order label not easily available
            revision,
        )?;
    }

    Ok(())
}

/// Reorder a child node to appear after another sibling
///
/// Delegates to NodeRepository's move_child_after which uses fractional indexing.
/// Tracks the reorder operation in the changeset.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the nodes
/// * `parent_path` - The path of the parent node
/// * `child_name` - The name of the child to move
/// * `after_child_name` - The name of the sibling to position after
///
/// # Returns
///
/// Ok(()) on success
pub async fn reorder_child_after(
    tx: &RocksDBTransaction,
    workspace: &str,
    parent_path: &str,
    child_name: &str,
    after_child_name: &str,
) -> Result<()> {
    let (tenant_id, repo_id, branch, actor, message) = {
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
            meta.actor.clone(),
            meta.message.clone(),
        )
    };

    // Get the child node info before reordering (for change tracking)
    let child_path = if parent_path == "/" {
        format!("/{}", child_name)
    } else {
        format!("{}/{}", parent_path, child_name)
    };

    let node_repo = tx.node_repo.as_ref();

    let scope = StorageScope::new(&tenant_id, &repo_id, &branch, workspace);

    // Get node info for tracking
    let node_info = node_repo.get_by_path(scope, &child_path, None).await?;

    // Delegate to node repository's move_child_after trait method
    // This handles fractional indexing, MVCC, and atomic writes
    node_repo
        .move_child_after(
            scope,
            parent_path,
            child_name,
            after_child_name,
            message.as_ref().map(|s| s.as_str()),
            actor.as_ref().map(|s| s.as_str()),
        )
        .await?;

    // Track the reorder operation in the changeset
    if let Some(node) = node_info {
        // Use the transaction's unified revision for consistency with other operations
        let revision = tx.get_or_allocate_transaction_revision()?;
        track_reorder(
            tx,
            workspace,
            &node.id,
            &child_path,
            &node.node_type,
            "".to_string(), // Old order label not easily available
            "".to_string(), // New order label not easily available
            revision,
        )?;
    }

    Ok(())
}
