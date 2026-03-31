//! Ordered children index management for node creation
//!
//! This module handles maintaining the ORDERED_CHILDREN index using fractional indexing.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Lookup parent ID for ordering index
///
/// Determines the parent ID to use for the ORDERED_CHILDREN index.
/// Checks transaction cache first for read-your-writes semantics.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node whose parent to lookup
///
/// # Returns
///
/// Optional parent ID (None if node has no parent)
///
/// # Errors
///
/// Returns error if lookup fails
pub(super) async fn lookup_parent_id(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
) -> Result<Option<String>> {
    let parent_path = match node.parent_path() {
        Some(path) => path,
        None => return Ok(None),
    };

    // Special case: root-level nodes use "/" as parent_id
    if parent_path == "/" {
        return Ok(Some("/".to_string()));
    }

    // First check transaction's read cache for read-your-writes semantics
    let cached_parent = {
        let cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        cache
            .paths
            .get(&(workspace.to_string(), parent_path.clone()))
            .cloned()
    };

    match cached_parent {
        Some(Some(parent_id)) => {
            // Parent found in transaction cache - get the full node
            let parent_node = {
                let cache = tx
                    .read_cache
                    .lock()
                    .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
                cache
                    .nodes
                    .get(&(workspace.to_string(), parent_id.clone()))
                    .and_then(|opt| opt.clone())
            };
            Ok(parent_node.map(|n| n.id))
        }
        Some(None) => Ok(None), // Parent was deleted in this transaction
        None => {
            // Not in cache, check committed storage
            tx.node_repo
                .get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                .await
                .map(|opt| opt.map(|p| p.id))
        }
    }
}

/// Lookup old parent ID for a node
///
/// Determines the old parent ID from an existing node for tombstoning.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `existing_node` - The existing node
///
/// # Returns
///
/// Optional old parent ID
///
/// # Errors
///
/// Returns error if lookup fails
pub(super) async fn lookup_old_parent_id(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    existing_node: &Node,
) -> Result<Option<String>> {
    let old_parent_path = match existing_node.parent_path() {
        Some(path) => path,
        None => return Ok(None),
    };

    if old_parent_path == "/" {
        return Ok(Some("/".to_string()));
    }

    tx.node_repo
        .get_by_path_impl(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &old_parent_path,
            None,
        )
        .await
        .map(|opt| opt.map(|p| p.id))
}

/// Tombstone old ORDERED_CHILDREN entry
///
/// Writes a tombstone marker for the old ordering entry when a node's parent changes.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `old_parent_id` - The old parent ID
/// * `node_id` - The node ID
/// * `revision` - The HLC revision for versioning
///
/// # Errors
///
/// Returns error if lock is poisoned or lookup fails
pub(super) fn tombstone_old_ordering(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old_parent_id: &str,
    node_id: &str,
    revision: &HLC,
) -> Result<()> {
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let cf_ordered = cf_handle(&tx.db, cf::ORDERED_CHILDREN)?;

    // Get the old order label for this child under the old parent
    if let Some(old_label) = tx.node_repo.get_order_label_for_child(
        tenant_id,
        repo_id,
        branch,
        workspace,
        old_parent_id,
        node_id,
    )? {
        // Write tombstone for old ORDERED_CHILDREN entry at new revision
        let old_ordered_key = keys::ordered_child_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            old_parent_id,
            &old_label,
            revision,
            node_id,
        );
        batch.put_cf(cf_ordered, old_ordered_key, b"T"); // Tombstone marker
    }

    Ok(())
}

/// Add or update ORDERED_CHILDREN index entry
///
/// Creates or updates an entry in the ORDERED_CHILDREN index.
/// For new nodes, calculates a new fractional index by appending.
/// For existing nodes, preserves the current order.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `parent_id` - The parent node ID
/// * `node` - The node to add
/// * `revision` - The HLC revision for versioning
///
/// # Returns
///
/// Tuple of (order_label, is_new_node) indicating the fractional index label and whether this is a new entry
///
/// # Errors
///
/// Returns error if lock is poisoned or fractional index calculation fails
pub(super) fn add_ordered_child(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    node: &Node,
    revision: &HLC,
) -> Result<(String, bool)> {
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let cf_ordered = cf_handle(&tx.db, cf::ORDERED_CHILDREN)?;

    // Check if node already has an order label (for updates)
    let existing_label = tx
        .node_repo
        .get_order_label_for_child(tenant_id, repo_id, branch, workspace, parent_id, &node.id)?;

    let (order_label, is_new_node) = if let Some(existing) = existing_label {
        (existing, false) // Preserve existing order - this is an update
    } else {
        // Calculate new label by appending - this is a new node
        let last_label = tx
            .node_repo
            .get_last_order_label(tenant_id, repo_id, branch, workspace, parent_id)?;

        // Extract fractional part from last label (strip ::HLC suffix)
        let fractional_label = if let Some(ref last) = last_label {
            let last_fractional = crate::fractional_index::extract_fractional(last);
            crate::fractional_index::inc(last_fractional)?
        } else {
            crate::fractional_index::first()
        };

        // Append HLC timestamp for causal ordering and conflict resolution
        // HLC provides total ordering across cluster with wall-clock semantics
        let label = format!("{}::{:016x}", fractional_label, revision.as_u128());
        (label, true)
    };

    let ordered_key = keys::ordered_child_key_versioned(
        tenant_id,
        repo_id,
        branch,
        workspace,
        parent_id,
        &order_label,
        revision,
        &node.id,
    );
    batch.put_cf(cf_ordered, ordered_key, node.name.as_bytes());

    Ok((order_label, is_new_node))
}

/// Add ORDERED_CHILDREN entry using fast path
///
/// Fast path for new nodes - just appends to the end without checking for existing entry.
/// This is used by `add_node` which assumes the node is new.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `parent_id` - The parent node ID
/// * `node` - The node to add
/// * `revision` - The HLC revision for versioning
///
/// # Returns
///
/// The fractional index label assigned
///
/// # Errors
///
/// Returns error if lock is poisoned or fractional index calculation fails
pub(super) fn add_ordered_child_fast(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent_id: &str,
    node: &Node,
    revision: &HLC,
) -> Result<String> {
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let cf_ordered = cf_handle(&tx.db, cf::ORDERED_CHILDREN)?;

    // FAST PATH: Just append to end (no existence check)
    let last_label = tx
        .node_repo
        .get_last_order_label(tenant_id, repo_id, branch, workspace, parent_id)?;

    // Extract fractional part from last label (strip ::HLC suffix)
    let fractional_label = if let Some(ref last) = last_label {
        let last_fractional = crate::fractional_index::extract_fractional(last);
        crate::fractional_index::inc(last_fractional)?
    } else {
        crate::fractional_index::first()
    };

    // Append HLC timestamp for causal ordering and conflict resolution
    // HLC provides total ordering across cluster with wall-clock semantics
    let order_label = format!("{}::{:016x}", fractional_label, revision.as_u128());

    let ordered_key = keys::ordered_child_key_versioned(
        tenant_id,
        repo_id,
        branch,
        workspace,
        parent_id,
        &order_label,
        revision,
        &node.id,
    );
    batch.put_cf(cf_ordered, ordered_key, node.name.as_bytes());

    Ok(order_label)
}
