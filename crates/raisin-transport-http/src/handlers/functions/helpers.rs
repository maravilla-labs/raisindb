// SPDX-License-Identifier: BSL-1.1

//! Shared helper functions for function handlers.
//!
//! Provides node lookup, metadata extraction, code loading, and property
//! parsing utilities used by invoke, list, and webhook execution paths.

use std::collections::HashMap;
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
use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE};

// ============================================================================
// Node lookup helpers
// ============================================================================

/// Find a function node by name in the functions workspace, on `main`.
///
/// The caller's auth context MUST be forwarded - a `None` context is
/// subject to RLS as an anonymous principal and sees no nodes at all,
/// which surfaces as a bogus 404 for every function.
pub(crate) async fn find_function_node(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    name: &str,
    auth_context: Option<&AuthContext>,
) -> Result<Node, ApiError> {
    find_function_node_on_branch(state, tenant_id, repo, DEFAULT_BRANCH, name, auth_context).await
}

/// Find a function node by name in the functions workspace of a given branch.
///
/// Exists because MCP servers are addressed per branch
/// (`POST /mcp/{repo}/{branch}/{slug}`) while the HTTP function API is
/// `main`-only; resolving a branch's tools against `main` made every function
/// unreachable from a non-`main` MCP endpoint.
pub(crate) async fn find_function_node_on_branch(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    name: &str,
    auth_context: Option<&AuthContext>,
) -> Result<Node, ApiError> {
    let node_svc = state.node_service_for_context(
        tenant_id,
        repo,
        branch,
        FUNCTIONS_WORKSPACE,
        auth_context.cloned(),
    );
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
    tenant_id: &str,
    repo: &str,
    node_id: &str,
    auth_context: Option<&AuthContext>,
) -> Result<Node, ApiError> {
    // Try functions workspace first (most common case)
    let node_svc = state.node_service_for_context(
        tenant_id,
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
        tenant_id,
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
    tenant_id: &str,
    repo: &str,
    function_node: &Node,
) -> Result<String, ApiError> {
    load_function_code_on_branch(state, tenant_id, repo, DEFAULT_BRANCH, function_node).await
}

/// Load a function node's code from a given branch.
///
/// The branch must match the one the node was resolved from, or the
/// `entry_file` asset lookup misses and a function that exists appears to have
/// no code.
pub(crate) async fn load_function_code_on_branch(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    function_node: &Node,
) -> Result<String, ApiError> {
    let (code, _metadata) = raisin_functions::execution::code_loader::load_function_code(
        state.storage.as_ref(),
        state.bin.as_ref(),
        tenant_id,
        repo,
        branch,
        FUNCTIONS_WORKSPACE,
        function_node,
        &function_node.path,
    )
    .await
    .map_err(|e| ApiError::validation_failed(e.to_string()))?;
    Ok(code)
}

/// Load a function's importable module files (siblings + `../dir/` externals).
///
/// `LoadedFunction::files` is what the QuickJS resolver consults; when it is
/// empty the resolver rejects EVERY import, because the loader is rebuilt per
/// execution for tenant isolation. Callers that build a `LoadedFunction` by hand
/// must therefore fill this in, or any function whose entry uses `import` fails
/// with `Error resolving module '…' from 'entry'` — even though its sibling
/// files exist as nodes.
///
/// Both loads are best-effort: a function that imports nothing must not fail
/// because a directory scan did, and a genuinely missing module still surfaces
/// as a resolver error at declare time.
pub(crate) async fn load_function_modules_on_branch(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    function_path: &str,
    entry_file_name: &str,
    entry_code: &str,
) -> HashMap<String, String> {
    load_function_modules_in(
        state,
        tenant_id,
        repo,
        branch,
        FUNCTIONS_WORKSPACE,
        function_path,
        entry_file_name,
        entry_code,
    )
    .await
}

/// As [`load_function_modules_on_branch`], but for a caller whose functions do
/// NOT live in the default workspace.
///
/// SQL function calls carry the workspace the query named, and a scan against
/// `functions` would silently find nothing there — an empty module map, which
/// is indistinguishable at the resolver from "this function has no modules".
/// One implementation, two entry points, rather than a second copy of the scan.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn load_function_modules_in(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    function_path: &str,
    entry_file_name: &str,
    entry_code: &str,
) -> HashMap<String, String> {
    use raisin_functions::execution::code_loader;

    let mut files = code_loader::load_sibling_files(
        state.storage.as_ref(),
        state.bin.as_ref(),
        tenant_id,
        repo,
        branch,
        workspace,
        function_path,
        entry_file_name,
    )
    .await
    .unwrap_or_default();

    let external = code_loader::load_external_modules(
        state.storage.as_ref(),
        state.bin.as_ref(),
        tenant_id,
        repo,
        branch,
        workspace,
        function_path,
        entry_code,
        &files,
    )
    .await
    .unwrap_or_default();
    files.extend(external);

    files
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

// ============================================================================
// Execution history (persisted + live registry merge)
// ============================================================================

/// List a tenant's jobs by merging the persisted metadata store with the
/// in-memory registry (see `RocksDBStorage::list_tenant_jobs_merged`).
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn merged_function_jobs(
    rocksdb: &raisin_rocksdb::RocksDBStorage,
    tenant_id: &str,
) -> Result<Vec<raisin_storage::JobInfo>, ApiError> {
    rocksdb
        .list_tenant_jobs_merged(tenant_id)
        .await
        .map_err(map_storage_error)
}

/// True when the job's persisted context records the given repository — or
/// when no context exists (legacy jobs), preserving the old path-only match.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn job_belongs_to_repo(
    rocksdb: &raisin_rocksdb::RocksDBStorage,
    tenant_id: &str,
    job_id: &raisin_storage::JobId,
    repo: &str,
) -> bool {
    match rocksdb.job_data_store().get(tenant_id, job_id) {
        Ok(Some(ctx)) => ctx.repo_id == repo,
        Ok(None) => true,
        Err(_) => true,
    }
}

#[cfg(test)]
mod module_loading_invariant {
    //! Every inline execution path must fill `LoadedFunction::files`.
    //!
    //! This is a source-level guard, which is unusual and deliberate. The bug it
    //! exists for shipped: six separate call sites built a `LoadedFunction` by
    //! hand and left `files` empty, so the QuickJS resolver rejected every
    //! `import` and any multi-file function failed with
    //! `Error resolving module '…' from 'entry'` — over HTTP, over WS, from SQL
    //! and from a webhook — while the SAME function worked from a trigger, a
    //! flow or a sync run, because those go through the job executor which loads
    //! modules for them. That asymmetry is what made it survive review and reach
    //! production.
    //!
    //! There is no unit test that can catch it: the omission is at the call
    //! site, not in any function's behaviour, and neither transport crate has an
    //! integration harness that could execute a real multi-module function. So
    //! this asserts the property directly on the source — file granularity
    //! rather than line windows, so reformatting cannot break it.
    //!
    //! If this fails on a file you just added: call
    //! [`load_function_modules_on_branch`] (or `load_function_modules_in`, or
    //! the WS crate's `handlers::function_modules`) and assign the result to
    //! `loaded.files`.

    use std::path::{Path, PathBuf};

    /// Defines the helpers rather than executing anything.
    const ALLOWED_WITHOUT_ASSIGNMENT: &[&str] = &["handlers/functions/helpers.rs"];

    fn crate_src(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    fn rust_files(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn every_file_that_builds_a_loaded_function_also_loads_its_modules() {
        // Both transports, because the same omission occurred independently in
        // each and a fix in one says nothing about the other.
        let roots = [crate_src("src"), crate_src("../raisin-transport-ws/src")];

        let mut offenders = Vec::new();
        for root in &roots {
            let mut files = Vec::new();
            rust_files(root, &mut files);
            for file in files {
                let Ok(src) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let builds =
                    src.contains("LoadedFunction::new(") || src.contains("build_loaded_function(");
                if !builds {
                    continue;
                }
                let display = file.display().to_string();
                if ALLOWED_WITHOUT_ASSIGNMENT
                    .iter()
                    .any(|a| display.replace('\\', "/").ends_with(a))
                {
                    continue;
                }
                let loads = src.contains(".files =") || src.contains("with_files(");
                if !loads {
                    offenders.push(display);
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "these build a LoadedFunction but never fill `files`, so every `import` \
             in the function they run will fail to resolve:\n  {}",
            offenders.join("\n  ")
        );
    }
}
