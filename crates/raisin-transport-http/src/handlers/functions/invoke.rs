// SPDX-License-Identifier: BSL-1.1

//! Function invocation handler.
//!
//! Handles synchronous and asynchronous function invocation via the
//! HTTP API. Synchronous invocations execute inline and return the
//! result; asynchronous invocations register a background job.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_functions::{ExecutionContext, ExecutionMode, FunctionExecutor};
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, state::AppState};

use super::helpers::{
    build_loaded_function, find_function_node, load_function_code, map_storage_error,
    parse_execution_mode, property_as_bool,
};
use super::types::{InlineFunctionResult, InvokeFunctionRequest, InvokeFunctionResponse};
use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE, TENANT_ID};

#[cfg(feature = "storage-rocksdb")]
use super::api_factory::build_function_api;
#[cfg(feature = "storage-rocksdb")]
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobStatus, JobType};

/// Invoke a function.
///
/// The auth context is extracted from the request middleware and passed to the
/// function execution. This enables RLS filtering based on the calling user's
/// permissions.
#[cfg(feature = "storage-rocksdb")]
pub async fn invoke_function(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<InvokeFunctionRequest>,
) -> Result<Json<InvokeFunctionResponse>, ApiError> {
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?
        .clone();

    let function_node = find_function_node(&state, &repo, &name).await?;

    let execution_mode = parse_execution_mode(function_node.properties.get("execution_mode"));
    if req.sync && execution_mode == ExecutionMode::Async {
        return Err(ApiError::validation_failed(format!(
            "Function '{}' does not support synchronous execution",
            name
        )));
    }
    if req.sync && req.wait_for_completion {
        return Err(ApiError::validation_failed(
            "wait_for_completion cannot be used with sync=true",
        ));
    }

    // Extract auth context from request
    let auth_context = auth.map(|Extension(ctx)| ctx);

    // Register job for tracking
    let async_execution_id = nanoid::nanoid!();
    let job_id = register_function_job(
        &rocksdb,
        &repo,
        &function_node.path,
        req.input.clone(),
        async_execution_id.clone(),
    )
    .await?;

    if req.sync {
        rocksdb
            .job_registry()
            .update_status(&job_id, JobStatus::Running)
            .await
            .map_err(map_storage_error)?;

        match execute_function_inline(
            &state,
            &repo,
            &function_node,
            req.input,
            req.timeout_ms,
            auth_context,
        )
        .await
        {
            Ok(result) => {
                persist_job_result(&rocksdb, &job_id, &result).await?;
                let response = InvokeFunctionResponse {
                    execution_id: result.execution_id.clone(),
                    sync: true,
                    result: result.result.clone(),
                    error: result.error.clone(),
                    job_id: Some(job_id.to_string()),
                    duration_ms: Some(result.duration_ms),
                    logs: Some(result.logs.clone()),
                    status: Some("completed".to_string()),
                    completed: Some(true),
                    timed_out: Some(false),
                    waited: Some(true),
                };
                Ok(Json(response))
            }
            Err(err) => {
                rocksdb
                    .job_registry()
                    .mark_failed(&job_id, err.message.clone())
                    .await
                    .map_err(map_storage_error)?;
                Err(err)
            }
        }
    } else {
        if req.wait_for_completion {
            let wait_timeout_ms = req.wait_timeout_ms.unwrap_or(60_000).clamp(1_000, 300_000);
            let waited_job =
                wait_for_job_terminal_state(&rocksdb, &job_id, wait_timeout_ms).await?;

            let (status, completed, timed_out, result, error, duration_ms, logs) = match waited_job
            {
                WaitedJob::Completed(job_info) => {
                    let (result, error, duration_ms, logs) = extract_result_fields(&job_info);
                    (
                        Some(job_status_to_string(&job_info.status)),
                        Some(true),
                        Some(false),
                        result,
                        error,
                        duration_ms,
                        logs,
                    )
                }
                WaitedJob::TimedOut => (
                    Some("running".to_string()),
                    Some(false),
                    Some(true),
                    None,
                    None,
                    None,
                    None,
                ),
            };

            return Ok(Json(InvokeFunctionResponse {
                execution_id: async_execution_id,
                sync: false,
                result,
                error,
                job_id: Some(job_id.to_string()),
                duration_ms,
                logs,
                status,
                completed,
                timed_out,
                waited: Some(true),
            }));
        }

        Ok(Json(InvokeFunctionResponse {
            execution_id: async_execution_id,
            sync: false,
            result: None,
            error: None,
            job_id: Some(job_id.to_string()),
            duration_ms: None,
            logs: None,
            status: Some("scheduled".to_string()),
            completed: Some(false),
            timed_out: Some(false),
            waited: Some(false),
        }))
    }
}

/// Invoke function stub when RocksDB is not available.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn invoke_function(
    State(_state): State<AppState>,
    Path((_repo, name)): Path<(String, String)>,
    Json(_req): Json<InvokeFunctionRequest>,
) -> Result<Json<InvokeFunctionResponse>, ApiError> {
    Err(ApiError::internal(format!(
        "Function '{}' cannot be invoked without RocksDB backend",
        name
    )))
}

// ============================================================================
// Job registration and inline execution
// ============================================================================

/// Register a background job for function execution tracking.
#[cfg(feature = "storage-rocksdb")]
async fn register_function_job(
    rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
    repo_id: &str,
    function_path: &str,
    input: serde_json::Value,
    execution_id: String,
) -> Result<JobId, ApiError> {
    use raisin_hlc::HLC;
    use std::collections::HashMap;

    let mut metadata = HashMap::new();
    metadata.insert("input".to_string(), input);

    let context = JobContext {
        tenant_id: TENANT_ID.to_string(),
        repo_id: repo_id.to_string(),
        branch: DEFAULT_BRANCH.into(),
        workspace_id: FUNCTIONS_WORKSPACE.into(),
        revision: HLC::new(0, 0),
        metadata,
    };

    let job_id = rocksdb
        .job_registry()
        .register_job(
            JobType::FunctionExecution {
                function_path: function_path.to_string(),
                trigger_name: Some("http".into()),
                execution_id,
            },
            Some(TENANT_ID.to_string()),
            None,
            None,
            None,
        )
        .await
        .map_err(map_storage_error)?;

    rocksdb
        .job_data_store()
        .put(&job_id, &context)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(job_id)
}

#[cfg(feature = "storage-rocksdb")]
enum WaitedJob {
    Completed(JobInfo),
    TimedOut,
}

#[cfg(feature = "storage-rocksdb")]
async fn wait_for_job_terminal_state(
    rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
    job_id: &JobId,
    wait_timeout_ms: u64,
) -> Result<WaitedJob, ApiError> {
    let poll = async {
        loop {
            let info = rocksdb
                .job_registry()
                .get_job_info(job_id)
                .await
                .map_err(map_storage_error)?;

            if !matches!(
                info.status,
                JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled
            ) {
                return Ok::<WaitedJob, ApiError>(WaitedJob::Completed(info));
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };

    match tokio::time::timeout(Duration::from_millis(wait_timeout_ms), poll).await {
        Ok(result) => result,
        Err(_) => Ok(WaitedJob::TimedOut),
    }
}

#[cfg(feature = "storage-rocksdb")]
fn job_status_to_string(status: &JobStatus) -> String {
    match status {
        JobStatus::Scheduled => "scheduled".to_string(),
        JobStatus::Running => "running".to_string(),
        JobStatus::Executing => "executing".to_string(),
        JobStatus::Completed => "completed".to_string(),
        JobStatus::Cancelled => "cancelled".to_string(),
        JobStatus::Failed(_) => "failed".to_string(),
    }
}

#[cfg(feature = "storage-rocksdb")]
fn extract_result_fields(
    job_info: &JobInfo,
) -> (
    Option<serde_json::Value>,
    Option<String>,
    Option<u64>,
    Option<Vec<String>>,
) {
    let mut result = None;
    let mut error = job_info.error.clone();
    let mut duration_ms = None;
    let mut logs = None;

    if let Some(payload) = &job_info.result {
        if let Some(obj) = payload.as_object() {
            result = obj.get("result").cloned();

            if error.is_none() {
                if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
                    error = Some(err.to_string());
                } else if obj.get("success").and_then(|v| v.as_bool()) == Some(false) {
                    error = Some("Function execution failed".to_string());
                }
            }

            duration_ms = obj.get("duration_ms").and_then(|v| v.as_u64());
            logs = obj.get("logs").and_then(|v| {
                v.as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|entry| entry.as_str().map(ToString::to_string))
                        .collect::<Vec<String>>()
                })
            });
        } else {
            result = Some(payload.clone());
        }
    }

    if error.is_none() {
        if let JobStatus::Failed(msg) = &job_info.status {
            error = Some(msg.clone());
        }
    }

    (result, error, duration_ms, logs)
}

/// Persist the result of inline function execution to the job store.
#[cfg(feature = "storage-rocksdb")]
async fn persist_job_result(
    rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
    job_id: &JobId,
    result: &InlineFunctionResult,
) -> Result<(), ApiError> {
    let result_json = serde_json::to_value(result)
        .map_err(|e| ApiError::internal(format!("Failed to serialize result: {}", e)))?;
    rocksdb
        .job_registry()
        .set_result(job_id, result_json)
        .await
        .map_err(map_storage_error)?;
    rocksdb
        .job_registry()
        .mark_completed(job_id)
        .await
        .map_err(map_storage_error)?;
    let _ = rocksdb.job_data_store().delete(job_id);
    Ok(())
}

/// Execute a function synchronously (inline) and return the result.
#[cfg(feature = "storage-rocksdb")]
async fn execute_function_inline(
    state: &AppState,
    repo: &str,
    node: &raisin_models::nodes::Node,
    input: serde_json::Value,
    timeout_override: Option<u64>,
    auth_context: Option<AuthContext>,
) -> Result<InlineFunctionResult, ApiError> {
    let code = load_function_code(state, repo, node).await?;
    let mut loaded = build_loaded_function(node, code)?;

    if let Some(timeout) = timeout_override {
        loaded.metadata.resource_limits.timeout_ms = timeout;
    }

    // Check if function requires admin escalation (from function metadata)
    let requires_admin = property_as_bool(node.properties.get("requiresAdmin")).unwrap_or(false);

    // Determine actor from auth context or default to "system"
    let actor = auth_context
        .as_ref()
        .and_then(|a| a.user_id.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("system");

    let mut context = ExecutionContext::new(TENANT_ID, repo, DEFAULT_BRANCH, actor)
        .with_workspace(FUNCTIONS_WORKSPACE)
        .with_input(input)
        .with_admin_escalation(requires_admin);

    // Clone auth context for transaction callbacks before moving into context
    let tx_auth_context = auth_context.clone();

    // Set auth context if provided
    if let Some(auth) = auth_context {
        context = context.with_auth(auth);
    }

    eprintln!(
        "[DEBUG] execute_function_inline - passing network_policy to build_function_api: http_enabled={}, allowed_urls={:?}",
        loaded.metadata.network_policy.http_enabled,
        loaded.metadata.network_policy.allowed_urls
    );
    let api = build_function_api(
        state,
        repo,
        loaded.metadata.network_policy.clone(),
        tx_auth_context,
    );
    let executor = FunctionExecutor::new();

    let result = executor
        .execute(&loaded, context.clone(), api.clone())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let logs = result
        .logs
        .iter()
        .map(|entry| format!("[{}] {}", entry.level, entry.message))
        .chain(
            api.get_logs()
                .into_iter()
                .map(|entry| format!("[{}] {}", entry.level, entry.message)),
        )
        .collect();

    Ok(InlineFunctionResult {
        execution_id: context.execution_id,
        success: result.success,
        result: result.output.clone(),
        error: result.error.map(|e| format!("{}", e)),
        duration_ms: result.stats.duration_ms,
        logs,
    })
}
