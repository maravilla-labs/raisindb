//! Change tracking for node creation
//!
//! This module handles tracking changes for revision snapshots, WebSocket notifications,
//! and CRDT replication.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::tree::ChangeOperation;

use crate::transaction::change_types::NodeChange;
use crate::transaction::RocksDBTransaction;

/// Track a new node creation
///
/// Records the node creation in both the changed_nodes set and the change_tracker.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace name
/// * `node` - The created node
/// * `revision` - The HLC revision
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(super) fn track_create(
    tx: &RocksDBTransaction,
    workspace: &str,
    node: &Node,
    revision: HLC,
) -> Result<()> {
    // Track in changed_nodes for revision snapshot
    {
        let mut changed = tx
            .changed_nodes
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        changed.insert(
            node.id.clone(),
            NodeChange {
                workspace: workspace.to_string(),
                revision,
                operation: ChangeOperation::Added,
                path: Some(node.path.clone()),
                node_type: Some(node.node_type.clone()),
            },
        );
    }

    // Track in change_tracker for CRDT replication
    {
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        tracker.track_create(workspace.to_string(), revision, node.clone());
        tracing::info!(
            node_id = %node.id,
            workspace = workspace,
            revision = %revision,
            "TRANSACTION: ChangeTracker recorded CREATE operation"
        );
    }

    Ok(())
}

/// Track a node update with property changes
///
/// Compares old and new nodes to track property changes.
/// Records changes in both changed_nodes and change_tracker.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace name
/// * `old_node` - The old node state
/// * `new_node` - The new node state
/// * `revision` - The HLC revision
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(super) fn track_update(
    tx: &RocksDBTransaction,
    workspace: &str,
    old_node: &Node,
    new_node: &Node,
    revision: HLC,
) -> Result<()> {
    // Track in changed_nodes
    {
        let mut changed = tx
            .changed_nodes
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        changed.insert(
            new_node.id.clone(),
            NodeChange {
                workspace: workspace.to_string(),
                revision,
                operation: ChangeOperation::Modified,
                path: Some(new_node.path.clone()),
                node_type: Some(new_node.node_type.clone()),
            },
        );
    }

    // Track property changes in change_tracker
    {
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        // Compare properties to track changes
        for (prop_name, new_value) in &new_node.properties {
            let old_value = old_node.properties.get(prop_name);
            if old_value != Some(new_value) {
                let old_val_json = old_value.and_then(|v| serde_json::to_value(v).ok());
                let new_val_json = serde_json::to_value(new_value).ok();
                tracker.track_property_change(
                    new_node.id.clone(),
                    workspace.to_string(),
                    revision,
                    prop_name.clone(),
                    old_val_json,
                    new_val_json,
                    Some(new_node.path.clone()),
                    Some(new_node.node_type.clone()),
                );
            }
        }

        // Track removed properties
        for (prop_name, old_value) in &old_node.properties {
            if !new_node.properties.contains_key(prop_name) {
                let old_val_json = serde_json::to_value(old_value).ok();
                tracker.track_property_change(
                    new_node.id.clone(),
                    workspace.to_string(),
                    revision,
                    prop_name.clone(),
                    old_val_json,
                    None,
                    Some(new_node.path.clone()),
                    Some(new_node.node_type.clone()),
                );
            }
        }
    }

    Ok(())
}

/// Track a node move operation
///
/// Records a move operation in the change_tracker.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace name
/// * `node_id` - The node ID
/// * `old_parent` - The old parent value
/// * `new_parent` - The new parent value
/// * `order_label` - Optional order label
/// * `revision` - The HLC revision
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(super) fn track_move(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    old_parent: Option<String>,
    new_parent: Option<String>,
    order_label: Option<String>,
    revision: HLC,
) -> Result<()> {
    let mut tracker = tx
        .change_tracker
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    tracker.track_move(
        node_id.to_string(),
        workspace.to_string(),
        revision,
        old_parent,
        new_parent,
        order_label,
    );

    Ok(())
}

/// Track a node reorder operation
///
/// Records a reorder operation where a node's order among siblings changed.
/// This is tracked separately from moves (which change parent) and updates (which change properties).
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace name
/// * `node_id` - The node ID
/// * `node_path` - The path of the node
/// * `node_type` - The node type
/// * `old_order_label` - The old order label
/// * `new_order_label` - The new order label
/// * `revision` - The HLC revision
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(crate) fn track_reorder(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    node_path: &str,
    node_type: &str,
    old_order_label: String,
    new_order_label: String,
    revision: HLC,
) -> Result<()> {
    // Track in changed_nodes for revision snapshot
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
                operation: ChangeOperation::Reordered,
                path: Some(node_path.to_string()),
                node_type: Some(node_type.to_string()),
            },
        );
    }

    // Track in change_tracker for CRDT replication
    {
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        tracker.track_order_key_change(
            node_id.to_string(),
            workspace.to_string(),
            revision,
            old_order_label,
            new_order_label,
        );
        tracing::debug!(
            node_id = %node_id,
            workspace = workspace,
            revision = %revision,
            "TRANSACTION: ChangeTracker recorded REORDER operation"
        );
    }

    Ok(())
}

/// Track changes for orphaned nodes
///
/// Tracks nodes without a parent (shouldn't normally happen).
/// Records as Modified operation with property change tracking.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace name
/// * `old_node` - Optional old node state
/// * `new_node` - The new node state
/// * `revision` - The HLC revision
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(super) fn track_orphaned(
    tx: &RocksDBTransaction,
    workspace: &str,
    old_node: Option<&Node>,
    new_node: &Node,
    revision: HLC,
) -> Result<()> {
    // Track in changed_nodes
    {
        let mut changed = tx
            .changed_nodes
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        changed.insert(
            new_node.id.clone(),
            NodeChange {
                workspace: workspace.to_string(),
                revision,
                operation: ChangeOperation::Modified,
                path: Some(new_node.path.clone()),
                node_type: Some(new_node.node_type.clone()),
            },
        );
    }

    // Track property changes if we have an old node
    if let Some(old) = old_node {
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        for (prop_name, new_value) in &new_node.properties {
            let old_value = old.properties.get(prop_name);
            if old_value != Some(new_value) {
                let old_val_json = old_value.and_then(|v| serde_json::to_value(v).ok());
                let new_val_json = serde_json::to_value(new_value).ok();
                tracker.track_property_change(
                    new_node.id.clone(),
                    workspace.to_string(),
                    revision,
                    prop_name.clone(),
                    old_val_json,
                    new_val_json,
                    Some(new_node.path.clone()),
                    Some(new_node.node_type.clone()),
                );
            }
        }
    }

    Ok(())
}
