//! Deep node operations (auto-create parent folders)
//!
//! This module provides deep versions of node creation/upsert operations
//! that automatically create missing parent folders as `raisin:Folder` nodes.
//!
//! # Functions
//!
//! - `add_deep_node`: Create a node, auto-creating parent folders
//! - `upsert_deep_node`: Upsert a node by PATH, auto-creating parent folders

use raisin_error::{Error, Result};
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Check if a workspace exists in the database.
///
/// Returns an error if the workspace doesn't exist.
fn validate_workspace_exists(tx: &RocksDBTransaction, workspace: &str) -> Result<()> {
    // Get tenant_id and repo_id from transaction metadata
    let (tenant_id, repo_id) = {
        let metadata = tx.metadata.lock().unwrap();
        (metadata.tenant_id.clone(), metadata.repo_id.clone())
    };

    // Build workspace key and check existence
    let key = keys::workspace_key(&tenant_id, &repo_id, workspace);
    let cf = cf_handle(&tx.db, cf::WORKSPACES)?;

    match tx.db.get_cf(cf, &key) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(Error::NotFound(format!(
            "Workspace '{}' does not exist. Cannot create nodes in non-existent workspace.",
            workspace
        ))),
        Err(e) => Err(Error::storage(format!(
            "Failed to check workspace existence: {}",
            e
        ))),
    }
}

/// Ensure all parent folders exist for a given node path.
///
/// Creates `raisin:Folder` (or specified type) nodes for any missing
/// intermediate paths. For example, for path `/lib/raisin/handler/mynode`,
/// this creates `/lib`, `/lib/raisin`, `/lib/raisin/handler` if they don't exist.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `path` - The full node path (e.g., "/lib/raisin/handler/mynode")
/// * `parent_node_type` - Node type for auto-created folders (e.g., "raisin:Folder")
async fn ensure_parent_folders(
    tx: &RocksDBTransaction,
    workspace: &str,
    path: &str,
    parent_node_type: &str,
) -> Result<()> {
    // Split path into segments: "/lib/raisin/handler/mynode" -> ["lib", "raisin", "handler", "mynode"]
    let segments: Vec<&str> = path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    // If 0 or 1 segments, no parent folders needed
    if segments.len() <= 1 {
        return Ok(());
    }

    let mut current_path = String::new();

    // Iterate through parent segments (all except the last one)
    for segment in &segments[..segments.len() - 1] {
        current_path = format!("{}/{}", current_path, segment);

        // Check if folder already exists at this path
        let existing = super::read::get_node_by_path(tx, workspace, &current_path).await?;

        tracing::debug!(
            workspace = %workspace,
            path = %current_path,
            exists = existing.is_some(),
            existing_id = existing.as_ref().map(|n| n.id.as_str()).unwrap_or("NONE"),
            "ENSURE_PARENT_FOLDERS: checking if parent folder exists"
        );

        if existing.is_none() {
            // Folder doesn't exist, create it
            let folder = Node {
                id: nanoid::nanoid!(),
                node_type: parent_node_type.to_string(),
                name: (*segment).to_string(),
                path: current_path.clone(),
                workspace: Some(workspace.to_string()),
                ..Default::default()
            };

            tracing::debug!(
                workspace = %workspace,
                path = %current_path,
                parent_node_type = %parent_node_type,
                "Creating parent folder for deep node operation"
            );

            super::upsert::upsert_node(tx, workspace, &folder).await?;
        }
    }

    Ok(())
}

/// Add a node, creating any missing parent folders first.
///
/// This is the deep version of `add_node()`. It ensures all intermediate
/// folders in the node's path exist before creating the node.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node` - The node to create (must have a valid path)
/// * `parent_node_type` - Node type for auto-created parent folders (e.g., "raisin:Folder")
///
/// # Errors
///
/// Returns an error if:
/// - The node already exists at the path
/// - Any parent folder creation fails
/// - The final node creation fails
pub async fn add_deep_node(
    tx: &RocksDBTransaction,
    workspace: &str,
    node: &Node,
    parent_node_type: &str,
) -> Result<()> {
    // Validate workspace exists before doing anything
    validate_workspace_exists(tx, workspace)?;

    // Ensure all parent folders exist
    ensure_parent_folders(tx, workspace, &node.path, parent_node_type).await?;

    // Then add the actual node
    super::create::add_node(tx, workspace, node).await
}

/// Upsert a node by PATH, creating any missing parent folders first.
///
/// This is the deep version of `upsert_node()`. It ensures all intermediate
/// folders in the node's path exist before upserting the node.
///
/// # Semantics
///
/// - Missing parent folders are always CREATED (never updated)
/// - If a node exists at the given PATH → UPDATE that node (preserves existing ID)
/// - If no node exists at the PATH → CREATE new node (uses provided ID)
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace containing the node
/// * `node` - The node to create or update
/// * `parent_node_type` - Node type for auto-created parent folders (e.g., "raisin:Folder")
pub async fn upsert_deep_node(
    tx: &RocksDBTransaction,
    workspace: &str,
    node: &Node,
    parent_node_type: &str,
) -> Result<()> {
    // Validate workspace exists before doing anything
    validate_workspace_exists(tx, workspace)?;

    // Ensure all parent folders exist
    ensure_parent_folders(tx, workspace, &node.path, parent_node_type).await?;

    // Then upsert the actual node
    super::upsert::upsert_node(tx, workspace, node).await
}
