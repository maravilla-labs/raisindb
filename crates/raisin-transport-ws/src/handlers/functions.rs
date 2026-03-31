// SPDX-License-Identifier: BSL-1.1

//! Function invocation handler for WebSocket transport.
//!
//! Provides two modes of function execution:
//! - **Async** (`FunctionInvoke`): Registers a `FunctionExecution` job via the
//!   RocksDB job registry and returns `{ execution_id, job_id }` immediately.
//! - **Sync** (`FunctionInvokeSync`): Executes the function inline within the
//!   WS request handler and returns `{ execution_id, result, ... }` directly.

use parking_lot::RwLock;
use serde::Deserialize;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope},
};

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FunctionInvokePayload {
    function_name: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    wait_for_completion: bool,
    #[serde(default)]
    wait_timeout_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// RocksDB-backed implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
mod inner {
    use super::*;
    use raisin_storage::{jobs::JobStatus, Storage};
    use std::collections::HashMap;
    use std::time::Duration;

    const TENANT_ID: &str = "default";
    const DEFAULT_BRANCH: &str = "main";
    const FUNCTIONS_WORKSPACE: &str = "functions";

    /// Require `context.repository` from the request.
    fn require_repo(request: &RequestEnvelope) -> Result<String, WsError> {
        request
            .context
            .repository
            .clone()
            .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))
    }

    // -----------------------------------------------------------------------
    // Async invoke (background job)
    // -----------------------------------------------------------------------

    pub async fn handle_function_invoke<S, B>(
        state: &Arc<WsState<S, B>>,
        _connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: FunctionInvokePayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;

        let rocksdb = state
            .rocksdb_storage
            .as_ref()
            .ok_or_else(|| WsError::InternalError("RocksDB storage not available".to_string()))?
            .clone();

        let function_node = raisin_functions::execution::code_loader::find_function(
            &*state.storage,
            TENANT_ID,
            &repo,
            DEFAULT_BRANCH,
            FUNCTIONS_WORKSPACE,
            &payload.function_name,
        )
        .await
        .map_err(|e| WsError::InvalidRequest(e.to_string()))?;

        // Register a background job for execution
        let execution_id = nanoid::nanoid!();
        let job_type = raisin_storage::jobs::JobType::FunctionExecution {
            function_path: function_node.path.clone(),
            trigger_name: Some("ws".into()),
            execution_id: execution_id.clone(),
        };

        let mut metadata = HashMap::new();
        metadata.insert("input".to_string(), payload.input);

        let context = raisin_storage::jobs::JobContext {
            tenant_id: TENANT_ID.to_string(),
            repo_id: repo.clone(),
            branch: DEFAULT_BRANCH.into(),
            workspace_id: FUNCTIONS_WORKSPACE.into(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        let job_id = rocksdb
            .job_registry()
            .register_job(job_type, Some(TENANT_ID.to_string()), None, None, None)
            .await
            .map_err(|e| WsError::StorageError(e.to_string()))?;

        rocksdb
            .job_data_store()
            .put(&job_id, &context)
            .map_err(|e| WsError::InternalError(e.to_string()))?;

        tracing::info!(
            job_id = %job_id,
            execution_id = %execution_id,
            function = %payload.function_name,
            "Queued function execution via WS"
        );

        if payload.wait_for_completion {
            let wait_timeout_ms = payload.wait_timeout_ms.unwrap_or(60_000).clamp(1_000, 300_000);
            let waited = wait_for_job_terminal_state(&rocksdb, &job_id, wait_timeout_ms).await?;

            return Ok(Some(ResponseEnvelope::success(
                request.request_id,
                match waited {
                    WaitedJob::Completed(job_info) => {
                        let (result, error, duration_ms, logs) = extract_result_fields(&job_info);
                        serde_json::json!({
                            "execution_id": execution_id,
                            "job_id": job_id.to_string(),
                            "status": job_status_to_string(&job_info.status),
                            "completed": true,
                            "timed_out": false,
                            "waited": true,
                            "result": result,
                            "error": error,
                            "duration_ms": duration_ms,
                            "logs": logs
                        })
                    }
                    WaitedJob::TimedOut => serde_json::json!({
                        "execution_id": execution_id,
                        "job_id": job_id.to_string(),
                        "status": "running",
                        "completed": false,
                        "timed_out": true,
                        "waited": true
                    }),
                },
            )));
        }

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "execution_id": execution_id,
                "job_id": job_id.to_string(),
                "status": "scheduled",
                "completed": false,
                "timed_out": false,
                "waited": false
            }),
        )))
    }

    enum WaitedJob {
        Completed(raisin_storage::jobs::JobInfo),
        TimedOut,
    }

    async fn wait_for_job_terminal_state(
        rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
        job_id: &raisin_storage::jobs::JobId,
        wait_timeout_ms: u64,
    ) -> Result<WaitedJob, WsError> {
        let poll = async {
            loop {
                let info = rocksdb
                    .job_registry()
                    .get_job_info(job_id)
                    .await
                    .map_err(|e| WsError::StorageError(e.to_string()))?;

                if !matches!(info.status, JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled) {
                    return Ok::<WaitedJob, WsError>(WaitedJob::Completed(info));
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };

        match tokio::time::timeout(Duration::from_millis(wait_timeout_ms), poll).await {
            Ok(result) => result,
            Err(_) => Ok(WaitedJob::TimedOut),
        }
    }

    fn job_status_to_string(status: &JobStatus) -> &'static str {
        match status {
            JobStatus::Scheduled => "scheduled",
            JobStatus::Running | JobStatus::Executing => "running",
            JobStatus::Completed => "completed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed(_) => "failed",
        }
    }

    fn extract_result_fields(
        job_info: &raisin_storage::jobs::JobInfo,
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

    // -----------------------------------------------------------------------
    // Sync invoke (inline execution)
    // -----------------------------------------------------------------------

    pub async fn handle_function_invoke_sync<S, B>(
        state: &Arc<WsState<S, B>>,
        _connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        use raisin_functions::{
            execution::callbacks::create_production_callbacks,
            execution::ExecutionDependencies,
            ExecutionContext, FunctionExecutor, RaisinFunctionApi,
        };

        let payload: FunctionInvokePayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let request_id = request.request_id.clone();

        // Find function via canonical code_loader
        let function_node = raisin_functions::execution::code_loader::find_function(
            &*state.storage,
            TENANT_ID,
            &repo,
            DEFAULT_BRANCH,
            FUNCTIONS_WORKSPACE,
            &payload.function_name,
        )
        .await
        .map_err(|e| WsError::InvalidRequest(e.to_string()))?;

        // Load function code via canonical code_loader (resolves entry_file property)
        let (code, metadata) = raisin_functions::execution::code_loader::load_function_code(
            &*state.storage,
            &*state.bin,
            TENANT_ID,
            &repo,
            DEFAULT_BRANCH,
            FUNCTIONS_WORKSPACE,
            &function_node,
            &function_node.path,
        )
        .await
        .map_err(|e| WsError::InternalError(format!("Failed to load function code: {}", e)))?;

        let loaded = raisin_functions::LoadedFunction::new(
            metadata.clone(),
            code,
            function_node.path.clone(),
            function_node.id.clone(),
            function_node
                .workspace
                .clone()
                .unwrap_or_else(|| FUNCTIONS_WORKSPACE.into()),
        );

        // Build ExecutionDependencies
        let deps = Arc::new(ExecutionDependencies {
            storage: state.storage.clone(),
            binary_storage: state.bin.clone(),
            indexing_engine: state.indexing_engine.clone(),
            hnsw_engine: state.hnsw_engine.clone(),
            http_client: reqwest::Client::new(),
            ai_config_store: None,
            job_registry: None,
            job_data_store: None,
        });

        // Build callbacks via canonical create_production_callbacks
        let callbacks = create_production_callbacks(
            deps,
            TENANT_ID.to_string(),
            repo.clone(),
            DEFAULT_BRANCH.to_string(),
            None, // no auth context from WS for now
        );

        let api = Arc::new(RaisinFunctionApi::new(
            ExecutionContext::new(TENANT_ID, &repo, DEFAULT_BRANCH, "system")
                .with_workspace(FUNCTIONS_WORKSPACE),
            metadata.network_policy.clone(),
            callbacks,
        ));

        let context = ExecutionContext::new(TENANT_ID, &repo, DEFAULT_BRANCH, "system")
            .with_workspace(FUNCTIONS_WORKSPACE)
            .with_input(payload.input);

        let executor = FunctionExecutor::new();
        let result = executor
            .execute(&loaded, context.clone(), api.clone())
            .await
            .map_err(|e| WsError::InternalError(e.to_string()))?;

        let logs: Vec<String> = result
            .logs
            .iter()
            .map(|entry| format!("[{}] {}", entry.level, entry.message))
            .chain(
                api.get_logs()
                    .into_iter()
                    .map(|entry| format!("[{}] {}", entry.level, entry.message)),
            )
            .collect();

        tracing::info!(
            execution_id = %context.execution_id,
            function = %payload.function_name,
            duration_ms = %result.stats.duration_ms,
            success = %result.success,
            "Executed function inline via WS"
        );

        Ok(Some(ResponseEnvelope::success(
            request_id,
            serde_json::json!({
                "execution_id": context.execution_id,
                "result": result.output,
                "error": result.error.map(|e| format!("{}", e)),
                "duration_ms": result.stats.duration_ms,
                "logs": logs,
            }),
        )))
    }
}

// ---------------------------------------------------------------------------
// Feature-gated re-exports / fallback stubs
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
pub use inner::handle_function_invoke;

#[cfg(feature = "storage-rocksdb")]
pub use inner::handle_function_invoke_sync;

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_function_invoke<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Function invocation requires RocksDB backend".to_string(),
    )))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_function_invoke_sync<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Function invocation requires RocksDB backend".to_string(),
    )))
}
