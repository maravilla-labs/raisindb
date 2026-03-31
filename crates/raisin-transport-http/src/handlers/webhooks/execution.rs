// SPDX-License-Identifier: BSL-1.1

//! Trigger execution logic (sync and async modes) and job registration.
//!
// NOTE: File slightly exceeds 300 lines - the execution functions are tightly
// coupled and splitting further would be artificial.

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::{HeaderMap, Method};
use axum::Json;
use raisin_functions::{ExecutionContext, FunctionExecutor, HttpRequestData, HttpRouteMode};
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use crate::error::ApiError;
use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
use raisin_storage::jobs::{JobContext, JobId, JobStatus, JobType};

use super::config::{find_trigger_target, parse_http_config, parse_path_params};
use super::helpers::{
    extract_query_params, header_as_bool, headers_to_map, property_as_bool, property_as_string,
};
use super::lookup::{find_trigger_by_name, find_trigger_by_webhook_id};
use super::types::{
    InvokeQuery, TriggerLookup, WebhookResponse, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE, TENANT_ID,
};

/// Internal implementation for HTTP trigger invocation
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn invoke_http_trigger_internal(
    state: &AppState,
    repo: &str,
    lookup: TriggerLookup,
    path_suffix: Option<String>,
    method: Method,
    headers: HeaderMap,
    query: InvokeQuery,
    body: Option<Json<serde_json::Value>>,
) -> Result<Json<WebhookResponse>, ApiError> {
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?
        .clone();

    // 1. Look up the trigger node
    let trigger_node = match &lookup {
        TriggerLookup::ByWebhookId(id) => find_trigger_by_webhook_id(state, repo, id).await?,
        TriggerLookup::ByName(name) => find_trigger_by_name(state, repo, name).await?,
    };

    // 2. Verify trigger is enabled
    let enabled = property_as_bool(trigger_node.properties.get("enabled")).unwrap_or(true);
    if !enabled {
        return Err(ApiError::validation_failed("Trigger is disabled"));
    }

    // 3. Verify trigger type is HTTP
    let trigger_type = property_as_string(trigger_node.properties.get("trigger_type"))
        .ok_or_else(|| ApiError::validation_failed("Trigger has no trigger_type"))?;
    if trigger_type != "http" {
        return Err(ApiError::validation_failed(format!(
            "Trigger type is '{}', expected 'http'",
            trigger_type
        )));
    }

    // 4. Parse HTTP trigger config
    let config = parse_http_config(&trigger_node)?;

    // 5. Validate HTTP method
    let method_str = method.to_string().to_uppercase();
    let allowed_methods: Vec<String> = config.methods.iter().map(|m| m.to_string()).collect();
    if !allowed_methods.contains(&method_str) {
        return Err(ApiError::validation_failed(format!(
            "Method {} not allowed. Allowed methods: {:?}",
            method_str, allowed_methods
        )));
    }

    // 6. Parse path parameters using matchit (if config mode with path_pattern)
    let path_params = if config.route_mode == HttpRouteMode::Config {
        if let Some(ref pattern) = config.path_pattern {
            let suffix = path_suffix.as_deref().unwrap_or("");
            parse_path_params(pattern, suffix)?
        } else {
            HashMap::new()
        }
    } else {
        HashMap::new()
    };

    // 7. Build HTTP request data for function context
    let http_request = HttpRequestData {
        method: method_str.clone(),
        path: path_suffix.clone().unwrap_or_default(),
        path_params: path_params.clone(),
        query_params: extract_query_params(&headers),
        headers: headers_to_map(&headers),
        body: body.as_ref().map(|b| b.0.clone()),
    };

    // 8. Determine execution mode (sync vs async)
    // Priority: query param > header > trigger config default > async
    let sync = query
        .sync
        .or_else(|| header_as_bool(&headers, "X-Raisin-Sync"))
        .unwrap_or(config.default_sync);

    // 9. Generate execution ID
    let execution_id = nanoid::nanoid!();

    // 10. Build input with HTTP context
    let input = serde_json::json!({
        "http": {
            "method": method_str,
            "path": path_suffix.clone().unwrap_or_default(),
            "params": path_params,
            "query": http_request.query_params,
            "headers": http_request.headers,
            "body": body.as_ref().map(|b| &b.0),
        }
    });

    // 11. Find the target function or flow
    let (function_path, _flow_data) = find_trigger_target(&trigger_node)?;

    if sync {
        // Synchronous execution: wait for result
        execute_sync(
            state,
            &rocksdb,
            repo,
            &trigger_node,
            &function_path,
            execution_id,
            input,
            http_request,
        )
        .await
    } else {
        // Asynchronous execution: queue job and return immediately
        execute_async(
            &rocksdb,
            repo,
            &trigger_node,
            &function_path,
            execution_id,
            input,
        )
        .await
    }
}

/// Execute trigger synchronously, waiting for result
#[cfg(feature = "storage-rocksdb")]
async fn execute_sync(
    state: &AppState,
    rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
    repo: &str,
    trigger_node: &Node,
    function_path: &str,
    execution_id: String,
    input: serde_json::Value,
    http_request: HttpRequestData,
) -> Result<Json<WebhookResponse>, ApiError> {
    use crate::handlers::functions::{
        build_function_api, build_loaded_function, load_function_code,
    };

    // Register job for tracking
    let job_id =
        register_trigger_job(rocksdb, repo, trigger_node, &execution_id, input.clone()).await?;

    // Update job status to Running
    rocksdb
        .job_registry()
        .update_status(&job_id, JobStatus::Running)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Find the function node
    let function_node = state
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT_ID, repo, DEFAULT_BRANCH, FUNCTIONS_WORKSPACE),
            function_path,
            None,
        )
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found(format!("Function not found: {}", function_path)))?;

    // Load and execute function
    let code = load_function_code(state, repo, &function_node).await?;
    let loaded = build_loaded_function(&function_node, code)?;

    let context = ExecutionContext::new(TENANT_ID, repo, DEFAULT_BRANCH, "http-trigger")
        .with_workspace(FUNCTIONS_WORKSPACE)
        .with_input(input.clone())
        .with_http_request(http_request);

    eprintln!(
        "[DEBUG] execute_trigger_function - passing network_policy to build_function_api: http_enabled={}, allowed_urls={:?}",
        loaded.metadata.network_policy.http_enabled,
        loaded.metadata.network_policy.allowed_urls
    );
    let api = build_function_api(state, repo, loaded.metadata.network_policy.clone(), None);
    let executor = FunctionExecutor::new();

    match executor
        .execute(&loaded, context.clone(), api.clone())
        .await
    {
        Ok(result) => {
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

            // Mark job as completed
            rocksdb
                .job_registry()
                .mark_completed(&job_id)
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;
            let _ = rocksdb.job_data_store().delete(&job_id);

            Ok(Json(WebhookResponse {
                execution_id: context.execution_id,
                status: if result.success {
                    "completed".to_string()
                } else {
                    "failed".to_string()
                },
                result: result.output,
                error: result.error.map(|e| format!("{}", e)),
                job_id: Some(job_id.to_string()),
                duration_ms: Some(result.stats.duration_ms),
                logs: Some(logs),
            }))
        }
        Err(err) => {
            // Mark job as failed
            rocksdb
                .job_registry()
                .mark_failed(&job_id, err.to_string())
                .await
                .map_err(|e| ApiError::internal(e.to_string()))?;

            Err(ApiError::internal(err.to_string()))
        }
    }
}

/// Execute trigger asynchronously, returning job_id immediately
#[cfg(feature = "storage-rocksdb")]
async fn execute_async(
    rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
    repo: &str,
    trigger_node: &Node,
    _function_path: &str,
    execution_id: String,
    input: serde_json::Value,
) -> Result<Json<WebhookResponse>, ApiError> {
    // Register job for background execution
    let job_id = register_trigger_job(rocksdb, repo, trigger_node, &execution_id, input).await?;

    Ok(Json(WebhookResponse {
        execution_id,
        status: "queued".to_string(),
        result: None,
        error: None,
        job_id: Some(job_id.to_string()),
        duration_ms: None,
        logs: None,
    }))
}

/// Register a job for trigger execution
#[cfg(feature = "storage-rocksdb")]
async fn register_trigger_job(
    rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
    repo: &str,
    trigger_node: &Node,
    execution_id: &str,
    input: serde_json::Value,
) -> Result<JobId, ApiError> {
    let trigger_path = trigger_node.path.clone();
    let trigger_name = property_as_string(trigger_node.properties.get("name"))
        .unwrap_or_else(|| trigger_node.name.clone());

    // Determine job type based on trigger target
    let job_type = if trigger_node.properties.contains_key("function_flow") {
        // Multi-function flow execution
        let flow_data = trigger_node
            .properties
            .get("function_flow")
            .map(|v| serde_json::to_value(v).unwrap_or_default())
            .unwrap_or_default();

        JobType::FlowExecution {
            flow_execution_id: execution_id.to_string(),
            trigger_path: trigger_path.clone(),
            flow: flow_data,
            current_step_index: 0,
            step_results: serde_json::json!({}),
        }
    } else {
        // Single function execution
        let function_path = property_as_string(trigger_node.properties.get("function_path"))
            .ok_or_else(|| {
                ApiError::validation_failed("Trigger has no function_path or function_flow")
            })?;

        JobType::FunctionExecution {
            function_path,
            trigger_name: Some(trigger_name.clone()),
            execution_id: execution_id.to_string(),
        }
    };

    // Build job context
    let mut metadata = HashMap::new();
    metadata.insert("input".to_string(), input);
    metadata.insert("trigger_type".to_string(), serde_json::json!("http"));
    metadata.insert("trigger_path".to_string(), serde_json::json!(trigger_path));

    let context = JobContext {
        tenant_id: TENANT_ID.to_string(),
        repo_id: repo.to_string(),
        branch: DEFAULT_BRANCH.to_string(),
        workspace_id: FUNCTIONS_WORKSPACE.to_string(),
        revision: raisin_hlc::HLC::new(0, 0),
        metadata,
    };

    // Register with JobRegistry
    let job_id = rocksdb
        .job_registry()
        .register_job(job_type, Some(TENANT_ID.to_string()), None, None, None)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Store context in JobDataStore
    rocksdb
        .job_data_store()
        .put(&job_id, &context)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(job_id)
}
