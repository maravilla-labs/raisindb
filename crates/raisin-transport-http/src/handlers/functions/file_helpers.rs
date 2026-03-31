// SPDX-License-Identifier: BSL-1.1

//! File execution helpers for `run_file` handler.
//!
//! Validation, input resolution, synthetic metadata construction, and
//! parent-function config lookup used when executing standalone files
//! (as opposed to registered `raisin:Function` nodes).

use raisin_functions::{
    ExecutionMode, FunctionLanguage, FunctionMetadata, NetworkPolicy, ResourceLimits,
};
use raisin_models::nodes::Node;

use crate::error::ApiError;
use crate::state::AppState;

use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE, TENANT_ID};

#[cfg(feature = "storage-rocksdb")]
use raisin_storage::{NodeRepository, Storage, StorageScope};

// ============================================================================
// File validation
// ============================================================================

/// Validate that the node is a runnable Asset file (JavaScript, Starlark, or SQL).
pub(super) fn validate_runnable_asset(node: &Node) -> Result<(), ApiError> {
    if node.node_type != "raisin:Asset" {
        return Err(ApiError::validation_failed(format!(
            "Node '{}' is not a raisin:Asset (found: {})",
            node.id, node.node_type
        )));
    }

    validate_runnable_asset_name(&node.name)
}

/// Validate that a file name is a runnable file (JavaScript, Starlark/Python, or SQL).
pub(super) fn validate_runnable_asset_name(name: &str) -> Result<(), ApiError> {
    let is_js = name.ends_with(".js") || name.ends_with(".ts") || name.ends_with(".mjs");
    let is_starlark = name.ends_with(".star") || name.ends_with(".py");
    let is_sql = name.ends_with(".sql");

    if !is_js && !is_starlark && !is_sql {
        return Err(ApiError::validation_failed(format!(
            "File '{}' is not a runnable file (.js, .ts, .mjs, .star, .py, or .sql)",
            name
        )));
    }

    Ok(())
}

// ============================================================================
// Input resolution
// ============================================================================

/// Resolve input from JSON or node reference.
pub(super) async fn resolve_file_input(
    state: &AppState,
    repo: &str,
    input: &Option<serde_json::Value>,
    input_node_id: &Option<String>,
    input_workspace: &Option<String>,
) -> serde_json::Value {
    // If input_node_id is provided, load that node as JSON input
    if let Some(ref node_id) = input_node_id {
        let workspace = input_workspace.as_deref().unwrap_or("content");
        let node_svc =
            state.node_service_for_context(TENANT_ID, repo, DEFAULT_BRANCH, workspace, None);

        if let Ok(Some(input_node)) = node_svc.get(node_id).await {
            return serde_json::to_value(input_node).unwrap_or(serde_json::json!({}));
        }
    }

    // Otherwise use JSON input (default to empty object)
    input.clone().unwrap_or(serde_json::json!({}))
}

// ============================================================================
// Synthetic metadata
// ============================================================================

/// Build synthetic `FunctionMetadata` from file name and handler.
pub(super) fn build_synthetic_metadata_from_name(
    file_name: &str,
    handler: &str,
) -> FunctionMetadata {
    let name = file_name
        .rsplit_once('.')
        .map(|(n, _)| n)
        .unwrap_or(file_name);

    // Detect language from file extension
    let language = if file_name.ends_with(".sql") {
        FunctionLanguage::Sql
    } else if file_name.ends_with(".star") || file_name.ends_with(".py") {
        FunctionLanguage::Starlark
    } else {
        FunctionLanguage::JavaScript
    };

    let mut metadata = FunctionMetadata::new(name.to_string(), language);

    metadata.title = format!("{} (direct execution)", name);
    metadata.description = Some(format!("Direct execution of {}", file_name));
    metadata.execution_mode = ExecutionMode::Both;
    metadata.enabled = true;
    metadata.entry_file = format!("{}:{}", file_name, handler);
    metadata.resource_limits = ResourceLimits::default();
    metadata.network_policy = NetworkPolicy::default();

    metadata
}

// ============================================================================
// Parent function config lookup
// ============================================================================

/// Find the `raisin:Function` node and extract its `network_policy` and `resource_limits`.
///
/// Tries two lookup strategies:
/// 1. Check if the path itself is a `raisin:Function` node
/// 2. If not, try the parent path (for asset paths like `/lib/func/index.js`)
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn find_parent_function_config(
    state: &AppState,
    repo: &str,
    path: &str,
) -> Option<(NetworkPolicy, ResourceLimits)> {
    tracing::trace!(path = path, "find_parent_function_config - looking up");
    let storage = &state.storage;

    // Strategy 1: Check if the path itself is a raisin:Function
    if let Ok(Some(node)) = storage
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE),
            path,
            None,
        )
        .await
    {
        tracing::trace!(
            path = %node.path,
            node_type = %node.node_type,
            "find_parent_function_config - found node at path"
        );
        if node.node_type == "raisin:Function" {
            let result = extract_function_config(&node);
            if let Some((ref policy, _)) = result {
                tracing::trace!(
                    http_enabled = policy.http_enabled,
                    allowed_urls = ?policy.allowed_urls,
                    "find_parent_function_config - extracted policy"
                );
            }
            return result;
        }
    }

    // Strategy 2: Try the parent path (for asset paths)
    let parent_path = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    tracing::trace!(
        parent_path = parent_path,
        "find_parent_function_config - trying parent"
    );
    if parent_path.is_empty() {
        tracing::trace!("find_parent_function_config - parent path is empty, returning None");
        return None;
    }

    if let Ok(Some(node)) = storage
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE),
            parent_path,
            None,
        )
        .await
    {
        tracing::trace!(
            path = %node.path,
            node_type = %node.node_type,
            "find_parent_function_config - found parent node"
        );
        if node.node_type == "raisin:Function" {
            let result = extract_function_config(&node);
            if let Some((ref policy, _)) = result {
                tracing::trace!(
                    http_enabled = policy.http_enabled,
                    allowed_urls = ?policy.allowed_urls,
                    "find_parent_function_config - extracted policy from parent"
                );
            }
            return result;
        }
    }

    tracing::trace!("find_parent_function_config - no raisin:Function node found");
    None
}

/// Extract `network_policy` and `resource_limits` from a Function node.
#[cfg(feature = "storage-rocksdb")]
fn extract_function_config(node: &Node) -> Option<(NetworkPolicy, ResourceLimits)> {
    let network_policy = node
        .properties
        .get("network_policy")
        .and_then(|v| serde_json::to_value(v).ok())
        .and_then(|v| serde_json::from_value::<NetworkPolicy>(v).ok())
        .unwrap_or_default();

    let resource_limits = node
        .properties
        .get("resource_limits")
        .and_then(|v| serde_json::to_value(v).ok())
        .and_then(|v| serde_json::from_value::<ResourceLimits>(v).ok())
        .unwrap_or_default();

    Some((network_policy, resource_limits))
}

// ============================================================================
// Encryption key helper
// ============================================================================

/// Get the master encryption key from environment variable.
///
/// The key must be set in `RAISIN_MASTER_KEY` as a 64-character hex string
/// representing 32 bytes.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn get_master_encryption_key() -> Result<[u8; 32], raisin_error::Error> {
    let key_hex = std::env::var("RAISIN_MASTER_KEY").map_err(|_| {
        raisin_error::Error::Validation(
            "RAISIN_MASTER_KEY environment variable not set".to_string(),
        )
    })?;

    let key_bytes = hex::decode(&key_hex).map_err(|e| {
        raisin_error::Error::Validation(format!("Invalid RAISIN_MASTER_KEY: not valid hex: {}", e))
    })?;

    if key_bytes.len() != 32 {
        return Err(raisin_error::Error::Validation(format!(
            "Invalid RAISIN_MASTER_KEY: expected 32 bytes, got {}",
            key_bytes.len()
        )));
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&key_bytes);

    Ok(key)
}
