// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Function execution callbacks for tool call handling.
//!
//! This module implements the `raisin.functions.execute()` API that handles
//! AIToolCall execution with automatic status updates and result node creation.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_binary::BinaryStorage;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobId, JobStatus, JobType};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use serde_json::{json, Value};

use crate::api::{FunctionCallCallback, FunctionExecuteCallback};
use crate::execution::types::ExecutionDependencies;

/// Create function_execute callback: `raisin.functions.execute(path, args, context)`
///
/// This callback handles the full lifecycle of tool call execution:
/// 1. Updates AIToolCall status → 'running'
/// 2. Executes the function via FunctionExecution job
/// 3. Creates AIToolResult child node with result/error
/// 4. Updates AIToolCall status → 'completed' or 'failed'
pub fn create_function_execute<S, B>(
    deps: Arc<ExecutionDependencies<S, B>>,
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    job_data_store: Arc<raisin_rocksdb::JobDataStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> FunctionExecuteCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(move |function_path, arguments, ctx| {
        let deps = deps.clone();
        let job_registry = job_registry.clone();
        let job_data_store = job_data_store.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();

        Box::pin(async move {
            let start = std::time::Instant::now();
            let execution_id = uuid::Uuid::new_v4().to_string();

            // 1. Update AIToolCall status → 'running'
            update_tool_call_status(
                &deps,
                &tenant,
                &repo,
                &branch,
                &ctx.tool_call_workspace,
                &ctx.tool_call_path,
                "running",
            )
            .await?;

            // 2. Execute function via job system
            let result = execute_function_job(
                &job_registry,
                &job_data_store,
                &function_path,
                &execution_id,
                &arguments,
                &tenant,
                &repo,
                &branch,
            )
            .await;

            let duration_ms = start.elapsed().as_millis() as u64;

            // 3-4. Create AIToolResult and update status based on result
            match result {
                Ok(func_result) => {
                    let result_data = json!({
                        "result": func_result,
                        "duration_ms": duration_ms
                    });

                    create_tool_result(
                        &deps,
                        &tenant,
                        &repo,
                        &branch,
                        &ctx.tool_call_workspace,
                        &ctx.tool_call_path,
                        result_data,
                    )
                    .await?;

                    update_tool_call_status(
                        &deps,
                        &tenant,
                        &repo,
                        &branch,
                        &ctx.tool_call_workspace,
                        &ctx.tool_call_path,
                        "completed",
                    )
                    .await?;

                    Ok(func_result)
                }
                Err(e) => {
                    let error_data = json!({
                        "error": e.to_string(),
                        "duration_ms": duration_ms
                    });

                    create_tool_result(
                        &deps,
                        &tenant,
                        &repo,
                        &branch,
                        &ctx.tool_call_workspace,
                        &ctx.tool_call_path,
                        error_data,
                    )
                    .await?;

                    update_tool_call_status(
                        &deps,
                        &tenant,
                        &repo,
                        &branch,
                        &ctx.tool_call_workspace,
                        &ctx.tool_call_path,
                        "failed",
                    )
                    .await?;

                    Err(e)
                }
            }
        })
    })
}

/// Create function_call callback: `raisin.functions.call(path, args)`
///
/// Simple function-to-function call without AI tool call context.
/// This callback:
/// 1. Creates FunctionExecution job
/// 2. Waits for completion
/// 3. Returns function result (or error)
///
/// Unlike `create_function_execute`, this does NOT:
/// - Update any AIToolCall status
/// - Create AIToolResult nodes
pub fn create_function_call(
    job_registry: Arc<raisin_storage::jobs::JobRegistry>,
    job_data_store: Arc<raisin_rocksdb::JobDataStore>,
    tenant_id: String,
    repo_id: String,
    branch: String,
) -> FunctionCallCallback {
    Arc::new(move |function_path, arguments| {
        let job_registry = job_registry.clone();
        let job_data_store = job_data_store.clone();
        let tenant = tenant_id.clone();
        let repo = repo_id.clone();
        let branch = branch.clone();

        Box::pin(async move {
            let execution_id = uuid::Uuid::new_v4().to_string();

            tracing::debug!(
                function_path = %function_path,
                execution_id = %execution_id,
                "Executing function call"
            );

            // Execute function via job system (same as function_execute, but no status updates)
            execute_function_job(
                &job_registry,
                &job_data_store,
                &function_path,
                &execution_id,
                &arguments,
                &tenant,
                &repo,
                &branch,
            )
            .await
        })
    })
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Update the status property of an AIToolCall node
async fn update_tool_call_status<S, B>(
    deps: &ExecutionDependencies<S, B>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    tool_call_path: &str,
    status: &str,
) -> Result<()>
where
    S: Storage + TransactionalStorage,
    B: BinaryStorage,
{
    deps.storage
        .nodes()
        .update_property_by_path(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            tool_call_path,
            "status",
            PropertyValue::String(status.to_string()),
        )
        .await?;

    tracing::debug!(
        tool_call_path = %tool_call_path,
        status = %status,
        "Updated AIToolCall status"
    );

    Ok(())
}

/// Create an AIToolResult child node under the AIToolCall
async fn create_tool_result<S, B>(
    deps: &ExecutionDependencies<S, B>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    tool_call_path: &str,
    result_data: Value,
) -> Result<()>
where
    S: Storage + TransactionalStorage,
    B: BinaryStorage,
{
    // Parse result_data into properties
    let mut properties = HashMap::new();

    if let Some(result) = result_data.get("result") {
        properties.insert(
            "result".to_string(),
            json_to_property_value(result.clone())?,
        );
    }

    if let Some(error) = result_data.get("error").and_then(|v| v.as_str()) {
        properties.insert(
            "error".to_string(),
            PropertyValue::String(error.to_string()),
        );
    }

    if let Some(duration) = result_data.get("duration_ms").and_then(|v| v.as_u64()) {
        properties.insert(
            "duration_ms".to_string(),
            PropertyValue::Integer(duration as i64),
        );
    }

    // Create result node
    let result_name = format!("result-{}", uuid::Uuid::new_v4());
    let result_path = format!("{}/{}", tool_call_path, result_name);

    let result_node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: result_name,
        path: result_path.clone(),
        node_type: "raisin:AIToolResult".to_string(),
        properties,
        created_at: Some(chrono::Utc::now()),
        ..Default::default()
    };

    deps.storage
        .nodes()
        .create(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            result_node,
            CreateNodeOptions::default(),
        )
        .await?;

    tracing::debug!(
        tool_call_path = %tool_call_path,
        result_path = %result_path,
        "Created AIToolResult node"
    );

    Ok(())
}

/// Execute a function via the job system and wait for completion
async fn execute_function_job(
    job_registry: &raisin_storage::jobs::JobRegistry,
    job_data_store: &raisin_rocksdb::JobDataStore,
    function_path: &str,
    execution_id: &str,
    arguments: &Value,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> Result<Value> {
    // Normalize function path: ensure leading slash for storage lookup
    let function_path = if function_path.starts_with('/') {
        function_path.to_string()
    } else {
        format!("/{}", function_path)
    };
    let function_path = function_path.as_str();

    // Create job context with function input
    let mut metadata = HashMap::new();
    metadata.insert("input".to_string(), arguments.clone());

    let context = JobContext {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.to_string(),
        workspace_id: "functions".to_string(), // Functions always in 'functions' workspace
        revision: raisin_hlc::HLC::new(0, 0),  // Not applicable for function execution
        metadata,
    };

    // Register FunctionExecution job
    let job_id = job_registry
        .register_job(
            JobType::FunctionExecution {
                function_path: function_path.to_string(),
                trigger_name: None, // Not triggered by an event
                execution_id: execution_id.to_string(),
            },
            Some(tenant_id.to_string()),
            None,    // No handle - job system will create one
            None,    // No cancel token needed
            Some(0), // max_retries=0: caller is blocked, don't retry with backoff
        )
        .await?;

    // Store job context
    job_data_store.put(&job_id, &context)?;

    tracing::info!(
        job_id = %job_id,
        execution_id = %execution_id,
        function_path = %function_path,
        "Queued FunctionExecution job for tool call"
    );

    // Wait for job completion (blocking wait)
    wait_for_job_completion(job_registry, &job_id).await
}

/// Wait for a job to complete and return its result
async fn wait_for_job_completion(
    job_registry: &raisin_storage::jobs::JobRegistry,
    job_id: &JobId,
) -> Result<Value> {
    use tokio::time::{sleep, Duration};

    const POLL_INTERVAL_MS: u64 = 100;
    const MAX_WAIT_MS: u64 = 300_000; // 5 minutes
    let start = std::time::Instant::now();

    loop {
        let status = job_registry.get_status(job_id).await?;

        match status {
            JobStatus::Completed => {
                // Get job result
                let job_info = job_registry.get_job_info(job_id).await?;

                return job_info
                    .result
                    .and_then(|r| r.get("result").cloned())
                    .ok_or_else(|| Error::Backend("Function did not return a result".to_string()));
            }
            JobStatus::Failed(error_msg) => {
                return Err(Error::Backend(format!(
                    "Function execution failed: {}",
                    error_msg
                )));
            }
            JobStatus::Cancelled => {
                return Err(Error::Backend(
                    "Function execution was cancelled".to_string(),
                ));
            }
            _ => {
                // Defensive: detect jobs stuck in Scheduled with exhausted retries
                if matches!(status, JobStatus::Scheduled) {
                    let info = job_registry.get_job_info(job_id).await?;
                    if info.retry_count > 0 && info.retry_count >= info.max_retries {
                        let error_msg = info.error.unwrap_or_else(|| "Unknown error".to_string());
                        return Err(Error::Backend(format!(
                            "Function execution failed after {} retries: {}",
                            info.retry_count, error_msg
                        )));
                    }
                }

                // Still running/scheduled, check timeout
                if start.elapsed().as_millis() as u64 > MAX_WAIT_MS {
                    return Err(Error::Backend("Function execution timeout".to_string()));
                }
                sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        }
    }
}

/// Convert a JSON Value to a PropertyValue
fn json_to_property_value(value: Value) -> Result<PropertyValue> {
    match value {
        Value::Null => Ok(PropertyValue::Null),
        Value::Bool(b) => Ok(PropertyValue::Boolean(b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(PropertyValue::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(PropertyValue::Float(f))
            } else {
                Err(Error::Validation("Invalid number".to_string()))
            }
        }
        Value::String(s) => Ok(PropertyValue::String(s)),
        Value::Array(arr) => {
            let items: Result<Vec<_>> = arr.into_iter().map(json_to_property_value).collect();
            Ok(PropertyValue::Array(items?))
        }
        Value::Object(obj) => {
            let mut map = HashMap::new();
            for (k, v) in obj {
                map.insert(k, json_to_property_value(v)?);
            }
            Ok(PropertyValue::Object(map))
        }
    }
}
