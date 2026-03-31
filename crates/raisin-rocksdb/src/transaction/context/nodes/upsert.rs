//! Node upsert operations (create-or-update by PATH)
//!
//! This module implements true UPSERT semantics for transactions:
//! - If a node exists at the given PATH → UPDATE that node (uses existing ID)
//! - If no node exists at the PATH → CREATE new node
//!
//! This differs from `put_node` which does create-or-update by ID.

use raisin_error::Result;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;

/// Upsert a node in the transaction (create or update by PATH)
///
/// Unlike `put_node` which checks by ID, this checks by PATH for true UPSERT semantics.
///
/// # Semantics
///
/// - If a node exists at `node.path` → UPDATE that node (uses existing node's ID)
/// - If no node exists at `node.path` → CREATE new node (uses provided ID)
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node` - The node to upsert (path is the key for existence check)
///
/// # Returns
///
/// Ok(()) on success, Error on validation or storage failure
pub async fn upsert_node(tx: &RocksDBTransaction, workspace: &str, node: &Node) -> Result<()> {
    // 1. Check if node exists at PATH (not by ID) - uses read-your-writes cache
    let existing = super::read::get_node_by_path(tx, workspace, &node.path).await?;

    tracing::info!(
        "UPSERT_NODE: workspace={}, path={}, input_id={}, existing_id={}",
        workspace,
        node.path,
        node.id,
        existing.as_ref().map(|n| n.id.as_str()).unwrap_or("NONE")
    );

    if let Some(existing_node) = existing {
        // UPDATE: Use existing node's ID, preserve identity
        let mut updated_node = node.clone();
        updated_node.id = existing_node.id.clone();

        tracing::info!(
            "UPSERT_NODE: Updating existing node at path '{}', preserving node_id={}",
            node.path,
            existing_node.id
        );

        // Use put_node which handles updates properly (validates, indexes, tracks)
        super::create::put_node(tx, workspace, &updated_node).await
    } else {
        // CREATE: Use provided ID (build_node_from_columns generates UUID)
        // DIAGNOSTIC: Log when creating new node to help track duplicate creation issues
        tracing::info!(
            "UPSERT_NODE: Creating new node at path '{}' with node_id={} (no existing node found)",
            node.path,
            node.id
        );

        // Use add_node which handles creates properly (validates, indexes, tracks)
        // Note: add_node will also check for path conflicts via read cache
        super::create::add_node(tx, workspace, node).await
    }
}
