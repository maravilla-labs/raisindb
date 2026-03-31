//! Node deletion operations
//!
//! This module contains the implementation of node deletion operations for transactions:
//! - `delete_node`: Delete a node with tombstone marker
//! - `delete_path_index`: Delete a path index entry with tombstone marker
//!
//! # Key Features
//!
//! ## Tombstoning
//!
//! Uses MVCC tombstone marker (b"T") instead of deleting keys.
//! This preserves time-travel semantics for historical queries.
//!
//! ## Single Source of Truth
//!
//! This module delegates to `crate::tombstones::add_node_tombstones()` for all
//! tombstone writing. This ensures that both transaction and repository delete
//! paths write identical tombstones to all column families.
//!
//! ## WARNING
//!
//! `delete_node` does NOT check for children or cascade delete.
//! Caller is responsible for ensuring node has no children, or for calling
//! delete_descendants() first. This is intentional for transactions because:
//! 1. Bulk operations (imports, migrations) need fine control over deletion order
//! 2. Tree deletions should delete children explicitly (makes operation visible in logs)
//! 3. Transactions can't easily cascade across multiple nodes atomically
//!
//! For safe single-node deletes with cascade, use NodeRepository::delete() instead.

use raisin_error::Result;

use crate::repositories::hash_property_value;
use crate::tombstones::{
    add_node_tombstones, TombstoneColumnFamilies, TombstoneContext, TOMBSTONE,
};
use crate::transaction::change_types::NodeChange;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Delete a node from the transaction
///
/// # WARNING
///
/// This method does NOT check for children or cascade delete.
/// Caller is responsible for ensuring node has no children, or for calling
/// delete_descendants() first. This is intentional for transactions because:
/// 1. Bulk operations (imports, migrations) need fine control over deletion order
/// 2. Tree deletions should delete children explicitly (makes operation visible in logs)
/// 3. Transactions can't easily cascade across multiple nodes atomically
///
/// For safe single-node deletes with cascade, use NodeRepository::delete() instead.
///
/// # Tombstoning
///
/// Uses MVCC tombstone marker (b"T") instead of deleting keys.
/// This preserves time-travel semantics for historical queries.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node_id` - The ID of the node to delete
///
/// # Returns
///
/// Ok(()) on success, Error if node not found or storage failure
pub async fn delete_node(tx: &RocksDBTransaction, workspace: &str, node_id: &str) -> Result<()> {
    // WARNING: This method does NOT check for children or cascade delete.
    // Caller is responsible for ensuring node has no children, or for calling
    // delete_descendants() first. This is intentional for transactions because:
    // 1. Bulk operations (imports, migrations) need fine control over deletion order
    // 2. Tree deletions should delete children explicitly (makes operation visible in logs)
    // 3. Transactions can't easily cascade across multiple nodes atomically
    //
    // For safe single-node deletes with cascade, use NodeRepository::delete() instead.

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

    // 2. Get current node to know what indexes to clean up
    let node = super::read::get_node(tx, workspace, node_id)
        .await?
        .ok_or_else(|| raisin_error::Error::NotFound(format!("Node {} not found", node_id)))?;

    // 2a. Check DELETE permission - SECURITY: deny-by-default if no auth context
    {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        match &meta.auth_context {
            Some(auth) => {
                use raisin_core::services::rls_filter;
                use raisin_models::permissions::{Operation, PermissionScope};

                // Create permission scope from transaction context
                let branch_str = meta.branch.as_ref().map(|s| s.as_str()).unwrap_or("main");
                let scope = PermissionScope::new(workspace, branch_str);

                if !rls_filter::can_perform(&node, Operation::Delete, auth, &scope) {
                    return Err(raisin_error::Error::PermissionDenied(format!(
                        "Cannot delete node at path '{}'",
                        node.path
                    )));
                }
            }
            None => {
                // SECURITY: Deny operation if no auth context set on transaction
                tracing::warn!(
                    node_id = %node_id,
                    workspace = %workspace,
                    path = %node.path,
                    "Transaction has no auth context - denying delete_node operation"
                );
                return Err(raisin_error::Error::PermissionDenied(
                    "Transaction requires auth context for node operations".to_string(),
                ));
            }
        }
    }

    // 3. Get or allocate the single transaction HLC (all nodes in tx share same revision)
    let revision = tx.get_or_allocate_transaction_revision()?;

    // Update read cache to mark deletion for read-your-writes
    {
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        cache
            .nodes
            .insert((workspace.to_string(), node_id.to_string()), None);
        cache
            .paths
            .insert((workspace.to_string(), node.path.clone()), None);
    }

    // 4. Get unique property info BEFORE locking batch (async operation)
    // This avoids holding MutexGuard across await points
    use crate::repositories::UniqueIndexManager;
    use raisin_storage::NodeTypeRepository;

    let unique_properties = {
        let node_type = tx
            .node_repo
            .node_type_repo
            .get(
                raisin_storage::BranchScope::new(&tenant_id, &repo_id, &branch),
                &node.node_type,
                None,
            )
            .await?;

        match node_type {
            Some(nt) => match nt.properties {
                Some(ref props) => props
                    .iter()
                    .filter_map(|p| {
                        if p.unique.unwrap_or(false) {
                            p.name.clone()
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>(),
                None => Vec::new(),
            },
            None => Vec::new(),
        }
    };

    // 5. Lock batch and write all tombstones (synchronous operations only)
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    // 5a. Tombstone unique index entries (release unique values for reuse)
    if !unique_properties.is_empty() {
        let unique_manager = UniqueIndexManager::new(tx.db.clone());

        for prop_name in unique_properties {
            if let Some(prop_value) = node.properties.get(&prop_name) {
                let value_hash = hash_property_value(prop_value);

                unique_manager.add_unique_tombstone_to_batch(
                    &mut batch,
                    &tenant_id,
                    &repo_id,
                    &branch,
                    workspace,
                    &node.node_type,
                    &prop_name,
                    &value_hash,
                    &revision,
                )?;
            }
        }
    }

    // Use shared tombstone function - SINGLE SOURCE OF TRUTH for all deletion tombstones
    let ctx = TombstoneContext::new(&tenant_id, &repo_id, &branch, workspace);
    let cfs = TombstoneColumnFamilies::from_arc_db(&tx.db)?;

    add_node_tombstones(&mut batch, &tx.db, &ctx, &cfs, &node, &revision)?;

    // Track changed node for revision snapshot creation during commit (always Deleted for delete_impl)
    // IMPORTANT: Store path and node_type BEFORE deletion so WebSocket subscriptions can match
    {
        let mut changed = tx
            .changed_nodes
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        changed.insert(
            node_id.to_string(),
            NodeChange {
                workspace: workspace.to_string(),
                revision,
                operation: raisin_models::tree::ChangeOperation::Deleted,
                path: Some(node.path.clone()), // Store path before deletion for event matching
                node_type: Some(node.node_type.clone()), // Store node_type for subscription filtering
            },
        );

        // Track detailed deletion for replication
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        tracker.track_delete(
            node_id.to_string(),
            workspace.to_string(),
            revision,
            Some(node.path.clone()),
            Some(node.node_type.clone()),
            Some(node.clone()),
        );
    }

    Ok(())
}

/// Delete a path index entry with tombstone marker
///
/// # MVCC Semantics
///
/// Writes a tombstone marker (b"T") instead of deleting the key.
/// This preserves time-travel semantics for historical queries.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the path
/// * `path` - The path to delete
///
/// # Returns
///
/// Ok(()) on success
pub async fn delete_path_index(tx: &RocksDBTransaction, workspace: &str, path: &str) -> Result<()> {
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

    // 2. Get or allocate the single transaction HLC (all operations in tx share same revision)
    let revision = tx.get_or_allocate_transaction_revision()?;

    tracing::info!(
        "TXN delete_path_index: workspace={}, path={}, revision={}",
        workspace,
        path,
        revision
    );

    // Update read cache to mark path as deleted
    {
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        cache
            .paths
            .insert((workspace.to_string(), path.to_string()), None);
    }

    // 3. CRITICAL FIX: Write TOMBSTONE marker instead of deleting
    // This preserves MVCC time-travel semantics
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let cf_path = cf_handle(&tx.db, cf::PATH_INDEX)?;
    let path_key =
        keys::path_index_key_versioned(&tenant_id, &repo_id, &branch, workspace, path, &revision);
    // Write tombstone marker instead of deleting the key
    batch.put_cf(cf_path, path_key, TOMBSTONE);

    Ok(())
}
