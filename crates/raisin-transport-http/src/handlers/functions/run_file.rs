// SPDX-License-Identifier: BSL-1.1

//! Direct file execution handler (SSE streaming).
//!
//! Executes a standalone JavaScript/Starlark/SQL file without requiring
//! a parent `raisin:Function` node. Useful for testing individual files
//! from the editor. Returns an SSE stream with started, log, result,
//! and done events.

use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
    Extension, Json,
};
use chrono::Utc;
use futures::stream::Stream;
use raisin_functions::{ExecutionContext, FunctionExecutor, LoadedFunction};
use raisin_models::auth::AuthContext;
use std::convert::Infallible;
use std::time::Duration;

use crate::{error::ApiError, state::AppState};

use super::file_helpers::{
    build_synthetic_metadata_from_name, resolve_file_input, validate_runnable_asset,
    validate_runnable_asset_name,
};
use super::helpers::{find_asset_node_by_id, load_asset_code};
use super::types::{RunFileEvent, RunFileRequest};
use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE, TENANT_ID};

/// Run a JavaScript file directly by node ID (SSE streaming).
///
/// This endpoint executes a standalone JS file (`raisin:Asset`) without requiring
/// a parent `raisin:Function` node. Useful for testing individual files.
///
/// Returns an SSE stream with events:
/// - `started`: Execution started
/// - `log`: Each console.log/error/warn output
/// - `result`: Final execution result
/// - `done`: Stream complete
#[cfg(feature = "storage-rocksdb")]
pub async fn run_file(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<RunFileRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    use super::api_factory::build_function_api;
    use super::file_helpers::find_parent_function_config;

    let execution_id = nanoid::nanoid!();
    let started_at = Utc::now();

    // Extract auth context for RLS filtering
    let auth_context = auth.map(|Extension(ctx)| ctx);

    // Clone what we need for the async stream
    let state_clone = state.clone();
    let repo_clone = repo.clone();
    let auth_clone = auth_context.clone();
    let req_node_id = req.node_id.clone();
    let req_code = req.code.clone();
    let req_file_name = req.file_name.clone();
    let req_function_path = req.function_path.clone();
    let req_handler = req.handler.clone();
    let req_input = req.input.clone();
    let req_input_node_id = req.input_node_id.clone();
    let req_input_workspace = req.input_workspace.clone();
    let req_timeout = req.timeout_ms;
    let exec_id = execution_id.clone();

    let stream = async_stream::stream! {
        // Determine code source: inline code OR load from node
        // Returns (code, file_name, path, node_id, workspace)
        let code_source: (String, String, String, String, String) = if let Some(inline_code) = req_code {
            // Use inline code directly (unsaved file case)
            let name = req_file_name.unwrap_or_else(|| "inline.js".to_string());

            // Validate it's a runnable file name
            if let Err(e) = validate_runnable_asset_name(&name) {
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: false,
                        result: None,
                        error: Some(e.into_message()),
                        duration_ms: 0,
                    }).unwrap_or_default()
                ));
                yield Ok(Event::default().event("done").data(
                    serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                ));
                return;
            }

            // Synthetic values for inline execution
            let synthetic_path = format!("/_inline/{}", name);
            let synthetic_id = format!("inline-{}", exec_id);
            (inline_code, name, synthetic_path, synthetic_id, FUNCTIONS_WORKSPACE.to_string())
        } else if let Some(node_id) = req_node_id {
            // Load from saved node (existing flow)
            let asset_result = find_asset_node_by_id(&state_clone, &repo_clone, &node_id, auth_clone.as_ref()).await;
            let asset_node = match asset_result {
                Ok(node) => node,
                Err(e) => {
                    yield Ok(Event::default().event("result").data(
                        serde_json::to_string(&RunFileEvent::Result {
                            execution_id: exec_id.clone(),
                            success: false,
                            result: None,
                            error: Some(e.into_message()),
                            duration_ms: 0,
                        }).unwrap_or_default()
                    ));
                    yield Ok(Event::default().event("done").data(
                        serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                    ));
                    return;
                }
            };

            // Validate it's a runnable file
            if let Err(e) = validate_runnable_asset(&asset_node) {
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: false,
                        result: None,
                        error: Some(e.into_message()),
                        duration_ms: 0,
                    }).unwrap_or_default()
                ));
                yield Ok(Event::default().event("done").data(
                    serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                ));
                return;
            }

            // Load code from asset
            let code_result = load_asset_code(&state_clone, &repo_clone, &asset_node).await;
            match code_result {
                Ok(c) => (
                    c,
                    asset_node.name.clone(),
                    asset_node.path.clone(),
                    asset_node.id.clone(),
                    asset_node.workspace.clone().unwrap_or_else(|| FUNCTIONS_WORKSPACE.into()),
                ),
                Err(e) => {
                    yield Ok(Event::default().event("result").data(
                        serde_json::to_string(&RunFileEvent::Result {
                            execution_id: exec_id.clone(),
                            success: false,
                            result: None,
                            error: Some(e.into_message()),
                            duration_ms: (Utc::now() - started_at).num_milliseconds() as u64,
                        }).unwrap_or_default()
                    ));
                    yield Ok(Event::default().event("done").data(
                        serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                    ));
                    return;
                }
            }
        } else {
            // Neither code nor node_id provided
            yield Ok(Event::default().event("result").data(
                serde_json::to_string(&RunFileEvent::Result {
                    execution_id: exec_id.clone(),
                    success: false,
                    result: None,
                    error: Some("Either 'code' or 'node_id' must be provided".to_string()),
                    duration_ms: 0,
                }).unwrap_or_default()
            ));
            yield Ok(Event::default().event("done").data(
                serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
            ));
            return;
        };

        let (code, file_name, asset_path, asset_id, asset_workspace) = code_source;

        // Send started event
        yield Ok(Event::default().event("started").data(
            serde_json::to_string(&RunFileEvent::Started {
                execution_id: exec_id.clone(),
                file_name: file_name.clone(),
                handler: req_handler.clone(),
            }).unwrap_or_default()
        ));

        // Resolve input
        let input = resolve_file_input(&state_clone, &repo_clone, &req_input, &req_input_node_id, &req_input_workspace).await;

        // Build synthetic function metadata
        let mut metadata = build_synthetic_metadata_from_name(&file_name, &req_handler);

        // Look up parent raisin:Function node to get network_policy and resource_limits
        let lookup_path = req_function_path.as_deref().unwrap_or(&asset_path);
        if let Some((network_policy, resource_limits)) = find_parent_function_config(&state_clone, &repo_clone, lookup_path).await {
            metadata.network_policy = network_policy;
            metadata.resource_limits = resource_limits;
        }

        let mut loaded = LoadedFunction::new(
            metadata,
            code,
            asset_path,
            asset_id,
            asset_workspace,
        );

        // Apply timeout override
        if let Some(timeout) = req_timeout {
            loaded.metadata.resource_limits.timeout_ms = timeout;
        }

        // Execute the function
        let context = ExecutionContext::new(TENANT_ID, &repo_clone, DEFAULT_BRANCH, "system")
            .with_workspace(FUNCTIONS_WORKSPACE)
            .with_input(input);

        eprintln!(
            "[DEBUG] run_file - passing network_policy to build_function_api: http_enabled={}, allowed_urls={:?}",
            loaded.metadata.network_policy.http_enabled,
            loaded.metadata.network_policy.allowed_urls
        );
        let api = build_function_api(&state_clone, &repo_clone, loaded.metadata.network_policy.clone(), None);
        let executor = FunctionExecutor::new();

        let exec_result = executor.execute(&loaded, context.clone(), api.clone()).await;

        match exec_result {
            Ok(result) => {
                // Stream logs
                for log_entry in &result.logs {
                    yield Ok(Event::default().event("log").data(
                        serde_json::to_string(&RunFileEvent::Log {
                            level: log_entry.level.to_string(),
                            message: log_entry.message.clone(),
                            timestamp: log_entry.timestamp.to_rfc3339(),
                        }).unwrap_or_default()
                    ));
                }

                // Also include API logs
                for log_entry in api.get_logs() {
                    yield Ok(Event::default().event("log").data(
                        serde_json::to_string(&RunFileEvent::Log {
                            level: log_entry.level.to_string(),
                            message: log_entry.message.clone(),
                            timestamp: log_entry.timestamp.to_rfc3339(),
                        }).unwrap_or_default()
                    ));
                }

                // Send result
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: result.success,
                        result: result.output.clone(),
                        error: result.error.map(|e| format!("{}", e)),
                        duration_ms: result.stats.duration_ms,
                    }).unwrap_or_default()
                ));
            }
            Err(e) => {
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: false,
                        result: None,
                        error: Some(e.to_string()),
                        duration_ms: (Utc::now() - started_at).num_milliseconds() as u64,
                    }).unwrap_or_default()
                ));
            }
        }

        // Send done
        yield Ok(Event::default().event("done").data(
            serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
        ));
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Stub `run_file` without RocksDB.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn run_file(
    State(_state): State<AppState>,
    Path(_repo): Path<String>,
    Json(_req): Json<RunFileRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = async_stream::stream! {
        yield Ok(Event::default().event("result").data(
            serde_json::to_string(&RunFileEvent::Result {
                execution_id: "none".into(),
                success: false,
                result: None,
                error: Some("File execution requires RocksDB backend".into()),
                duration_ms: 0,
            }).unwrap_or_default()
        ));
        yield Ok(Event::default().event("done").data(
            serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
        ));
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
