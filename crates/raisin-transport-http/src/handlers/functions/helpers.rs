// SPDX-License-Identifier: BSL-1.1

//! Shared helper functions for function handlers.
//!
//! Provides node lookup, metadata extraction, code loading, and property
//! parsing utilities used by invoke, list, and webhook execution paths.

use std::sync::Arc;

use chrono::Utc;
use raisin_binary::BinaryStorage;
use raisin_functions::{
    ExecutionMode, FunctionLanguage, FunctionMetadata, LoadedFunction, NetworkPolicy,
    ResourceLimits,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::{value::Resource, PropertyValue};
use raisin_models::nodes::Node;
use raisin_storage::NodeRepository;

use crate::error::ApiError;
use crate::state::AppState;

use super::types::FunctionDetails;
use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE, TENANT_ID};

// ============================================================================
// Node lookup helpers
// ============================================================================

/// Find a function node by name in the functions workspace.
pub(crate) async fn find_function_node(
    state: &AppState,
    repo: &str,
    name: &str,
) -> Result<Node, ApiError> {
    let node_svc =
        state.node_service_for_context(TENANT_ID, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE, None);
    let nodes = node_svc
        .list_by_type("raisin:Function")
        .await
        .map_err(map_storage_error)?;

    nodes
        .into_iter()
        .find(|n| {
            property_as_string(n.properties.get("name"))
                .map(|p| p == name)
                .unwrap_or(false)
                || n.name == name
        })
        .ok_or_else(|| ApiError::not_found(format!("Function '{}' not found", name)))
}

/// Find an Asset node by ID across all workspaces.
pub(super) async fn find_asset_node_by_id(
    state: &AppState,
    repo: &str,
    node_id: &str,
    auth_context: Option<&AuthContext>,
) -> Result<Node, ApiError> {
    // Try functions workspace first (most common case)
    let node_svc = state.node_service_for_context(
        TENANT_ID,
        repo,
        DEFAULT_BRANCH,
        FUNCTIONS_WORKSPACE,
        auth_context.cloned(),
    );

    if let Some(node) = node_svc.get(node_id).await.map_err(map_storage_error)? {
        return Ok(node);
    }

    // Try content workspace as fallback
    let node_svc = state.node_service_for_context(
        TENANT_ID,
        repo,
        DEFAULT_BRANCH,
        "content",
        auth_context.cloned(),
    );

    node_svc
        .get(node_id)
        .await
        .map_err(map_storage_error)?
        .ok_or_else(|| ApiError::not_found(format!("Asset node '{}' not found", node_id)))
}

// ============================================================================
// Metadata and code loading
// ============================================================================

/// Build a `FunctionDetails` response from a node.
pub(super) fn build_function_details(
    node: &Node,
    code: Option<String>,
) -> Result<FunctionDetails, ApiError> {
    // Support both new entry_file format and legacy entrypoint
    let entry_file = property_as_string(node.properties.get("entry_file"))
        .or_else(|| {
            // Backward compatibility: convert old entrypoint to entry_file format
            property_as_string(node.properties.get("entrypoint")).map(|ep| {
                if ep.contains(':') {
                    ep
                } else {
                    format!("index.js:{}", ep)
                }
            })
        })
        .unwrap_or_else(|| "index.js:handler".into());

    // Extract just the function name for backward compat entrypoint field
    let entrypoint_compat = entry_file
        .rsplit_once(':')
        .map(|(_, func)| func.to_string())
        .unwrap_or_else(|| entry_file.clone());

    Ok(FunctionDetails {
        path: node.path.clone(),
        name: property_as_string(node.properties.get("name")).unwrap_or_else(|| node.name.clone()),
        title: property_as_string(node.properties.get("title"))
            .unwrap_or_else(|| node.name.clone()),
        description: property_as_string(node.properties.get("description")),
        language: property_as_string(node.properties.get("language"))
            .unwrap_or_else(|| "javascript".into()),
        enabled: property_as_bool(node.properties.get("enabled")).unwrap_or(true),
        execution_mode: property_as_string(node.properties.get("execution_mode"))
            .unwrap_or_else(|| "async".into()),
        entry_file,
        entrypoint: Some(entrypoint_compat), // Deprecated, for backward compat
        resource_limits: property_as_json(node.properties.get("resource_limits")),
        network_policy: property_as_json(node.properties.get("network_policy")),
        triggers: property_as_json(node.properties.get("triggers")),
        input_schema: property_as_json(node.properties.get("input_schema")),
        output_schema: property_as_json(node.properties.get("output_schema")),
        code,
        created_at: node.created_at.unwrap_or_else(Utc::now).to_rfc3339(),
        updated_at: node.updated_at.unwrap_or_else(Utc::now).to_rfc3339(),
    })
}

/// Load the code content for a function node via the canonical code_loader.
///
/// Delegates to `code_loader::load_function_code` which resolves the `entry_file`
/// property to find the asset node containing the code.
pub(crate) async fn load_function_code(
    state: &AppState,
    repo: &str,
    function_node: &Node,
) -> Result<String, ApiError> {
    let (code, _metadata) = raisin_functions::execution::code_loader::load_function_code(
        state.storage.as_ref(),
        state.bin.as_ref(),
        TENANT_ID,
        repo,
        DEFAULT_BRANCH,
        FUNCTIONS_WORKSPACE,
        function_node,
        &function_node.path,
    )
    .await
    .map_err(|e| ApiError::validation_failed(e.to_string()))?;
    Ok(code)
}

/// Load binary resource content as UTF-8 string.
async fn load_resource_content(
    bin: &Arc<impl BinaryStorage>,
    res: &Resource,
) -> Result<String, String> {
    if let Some(meta) = &res.metadata {
        if let Some(PropertyValue::String(key)) = meta.get("storage_key") {
            let bytes = bin
                .get(key)
                .await
                .map_err(|e| format!("Failed to load code asset: {}", e))?;
            return String::from_utf8(bytes.to_vec())
                .map_err(|e| format!("Invalid UTF-8 in code asset: {}", e));
        }
    }
    Err("Resource missing storage_key metadata".into())
}

/// Load code content from an Asset node.
pub(super) async fn load_asset_code(
    state: &AppState,
    repo: &str,
    asset: &Node,
) -> Result<String, ApiError> {
    // Check inline code property first
    if let Some(code) = property_as_string(asset.properties.get("code")) {
        return Ok(code);
    }

    // Check if "file" property is a String (inline upload case)
    if let Some(code) = property_as_string(asset.properties.get("file")) {
        return Ok(code);
    }

    // Fallback to file Resource property (external blob storage)
    if let Some(PropertyValue::Resource(res)) = asset.properties.get("file") {
        return load_resource_content(&state.bin, res)
            .await
            .map_err(ApiError::validation_failed);
    }

    Err(ApiError::validation_failed(
        "Asset missing code or file content",
    ))
}

/// Build `FunctionMetadata` from a node's properties.
pub(super) fn build_metadata(node: &Node) -> Result<FunctionMetadata, ApiError> {
    let name = property_as_string(node.properties.get("name")).unwrap_or_else(|| node.name.clone());
    let mut metadata = FunctionMetadata::new(
        name.clone(),
        parse_language(node.properties.get("language")),
    );

    metadata.title =
        property_as_string(node.properties.get("title")).unwrap_or_else(|| name.clone());
    metadata.description = property_as_string(node.properties.get("description"));
    metadata.execution_mode = parse_execution_mode(node.properties.get("execution_mode"));
    metadata.version = property_as_number(node.properties.get("version"))
        .map(|v| v as u32)
        .unwrap_or(1);
    metadata.enabled = property_as_bool(node.properties.get("enabled")).unwrap_or(true);
    // Support both new entry_file format and legacy entrypoint
    metadata.entry_file = property_as_string(node.properties.get("entry_file"))
        .or_else(|| property_as_string(node.properties.get("entrypoint")))
        .unwrap_or_else(|| "index.js:handler".into());

    if let Some(json) = property_as_json(node.properties.get("resource_limits")) {
        metadata.resource_limits =
            serde_json::from_value(json).unwrap_or_else(|_| ResourceLimits::default());
    }

    if let Some(json) = property_as_json(node.properties.get("network_policy")) {
        metadata.network_policy =
            serde_json::from_value(json).unwrap_or_else(|_| NetworkPolicy::default());
    }

    if let Some(json) = property_as_json(node.properties.get("triggers")) {
        metadata.triggers = serde_json::from_value(json).unwrap_or_default();
    }

    metadata.input_schema = property_as_json(node.properties.get("input_schema"));
    metadata.output_schema = property_as_json(node.properties.get("output_schema"));

    if let Some(json) = property_as_json(node.properties.get("metadata")) {
        metadata.metadata = serde_json::from_value(json).unwrap_or_default();
    }

    Ok(metadata)
}

/// Build a `LoadedFunction` from a node and its code.
pub(crate) fn build_loaded_function(node: &Node, code: String) -> Result<LoadedFunction, ApiError> {
    let metadata = build_metadata(node)?;
    Ok(LoadedFunction::new(
        metadata,
        code,
        node.path.clone(),
        node.id.clone(),
        node.workspace
            .clone()
            .unwrap_or_else(|| FUNCTIONS_WORKSPACE.into()),
    ))
}

// ============================================================================
// Property parsing utilities
// ============================================================================

pub(super) fn property_as_string(value: Option<&PropertyValue>) -> Option<String> {
    value.and_then(|v| match v {
        PropertyValue::String(s) => Some(s.clone()),
        _ => None,
    })
}

pub(super) fn property_as_bool(value: Option<&PropertyValue>) -> Option<bool> {
    value.and_then(|v| match v {
        PropertyValue::Boolean(b) => Some(*b),
        _ => None,
    })
}

pub(super) fn property_as_number(value: Option<&PropertyValue>) -> Option<f64> {
    value.and_then(|v| match v {
        PropertyValue::Integer(i) => Some(*i as f64),
        PropertyValue::Float(f) => Some(*f),
        _ => None,
    })
}

pub(super) fn property_as_json(value: Option<&PropertyValue>) -> Option<serde_json::Value> {
    value.and_then(|prop| serde_json::to_value(prop).ok())
}

pub(super) fn parse_execution_mode(value: Option<&PropertyValue>) -> ExecutionMode {
    property_as_string(value)
        .map(|s| match s.as_str() {
            "sync" => ExecutionMode::Sync,
            "both" => ExecutionMode::Both,
            _ => ExecutionMode::Async,
        })
        .unwrap_or(ExecutionMode::Async)
}

pub(super) fn parse_language(value: Option<&PropertyValue>) -> FunctionLanguage {
    property_as_string(value)
        .and_then(|s| s.parse().ok())
        .unwrap_or(FunctionLanguage::JavaScript)
}

/// Analyze trigger types from a function node's triggers property.
pub(super) fn analyze_triggers(triggers: Option<&PropertyValue>) -> (bool, bool, bool) {
    if let Some(value) = triggers {
        if let Some(serde_json::Value::Array(arr)) = property_as_json(Some(value)) {
            let mut has_http = false;
            let mut has_event = false;
            let mut has_schedule = false;
            for trigger in arr {
                if let Some(t) = trigger.get("trigger_type").and_then(|v| v.as_str()) {
                    match t {
                        "http" => has_http = true,
                        "node_event" | "event" => has_event = true,
                        "schedule" | "cron" => has_schedule = true,
                        _ => {}
                    }
                }
            }
            return (has_http, has_event, has_schedule);
        }
    }
    (false, false, false)
}

// ============================================================================
// Error mapping
// ============================================================================

pub(super) fn map_storage_error(err: raisin_error::Error) -> ApiError {
    ApiError::internal(err.to_string())
}

#[cfg(feature = "storage-rocksdb")]
pub(super) fn job_status_label(status: &raisin_storage::jobs::JobStatus) -> String {
    match status {
        raisin_storage::jobs::JobStatus::Scheduled => "scheduled",
        raisin_storage::jobs::JobStatus::Running => "running",
        raisin_storage::jobs::JobStatus::Executing => "executing",
        raisin_storage::jobs::JobStatus::Completed => "completed",
        raisin_storage::jobs::JobStatus::Cancelled => "cancelled",
        raisin_storage::jobs::JobStatus::Failed(_) => "failed",
    }
    .to_string()
}
