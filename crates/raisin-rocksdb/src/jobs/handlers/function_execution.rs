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

//! Function execution job handler
//!
//! This module handles the execution of serverless functions (JavaScript/Starlark).
//! When a FunctionExecution job is queued, this handler loads the function code
//! from the raisin:Function node and executes it using the appropriate runtime.
//!
//! # Architecture Note
//!
//! Due to dependency structure (raisin-functions depends on this crate for storage),
//! the actual function execution is done by a callback provided at runtime rather
//! than directly using FunctionExecutor here. The transport layer provides the
//! executor callback when starting the job system.
//!
//! # Function Enabled Check
//!
//! Before executing a function, this handler checks if the function is enabled
//! via the `function_enabled_checker` callback. If the function is disabled,
//! the job fails with a validation error.

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_storage::jobs::{JobContext, JobInfo, JobRegistry, JobType};
use raisin_storage::LogEmitter;
use std::sync::Arc;

/// Functions are always stored in the "functions" workspace
const FUNCTIONS_WORKSPACE: &str = "functions";

/// Result of a function execution
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FunctionExecutionResult {
    /// Unique execution ID
    pub execution_id: String,
    /// Whether execution was successful
    pub success: bool,
    /// Function return value (as JSON)
    pub result: Option<serde_json::Value>,
    /// Error message if failed
    pub error: Option<String>,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
    /// Logs captured during execution
    pub logs: Vec<String>,
}

/// Callback type for function execution
///
/// This callback is provided by the transport layer which has access to FunctionExecutor.
/// Arguments: (function_path, execution_id, input_json, tenant_id, repo_id, branch, workspace, auth_context)
/// Returns: FunctionExecutionResult
///
/// The `auth_context` parameter controls permissions for the function:
/// - `None`: Function runs with system context (full access, bypasses RLS)
/// - `Some(auth)`: Function runs with the provided user's permissions (RLS applied)
pub type FunctionExecutorCallback = Arc<
    dyn Fn(
            String,              // function_path
            String,              // execution_id
            serde_json::Value,   // input (from context.metadata["input"])
            String,              // tenant_id
            String,              // repo_id
            String,              // branch
            String,              // workspace
            Option<AuthContext>, // auth_context for permission control
            Option<LogEmitter>,  // log_emitter for real-time log streaming
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<FunctionExecutionResult>> + Send>,
        > + Send
        + Sync,
>;

/// Callback type for checking if a function is enabled
///
/// This callback is provided by the transport layer to check the enabled status
/// of a function before execution. Returns true if enabled, false if disabled.
/// Arguments: (function_path, tenant_id, repo_id, branch, workspace)
/// Returns: Result<bool> - true if enabled, false if disabled
pub type FunctionEnabledChecker = Arc<
    dyn Fn(
            String, // function_path
            String, // tenant_id
            String, // repo_id
            String, // branch
            String, // workspace
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for resuming a flow after function execution
///
/// This callback is provided by the transport layer to queue a FlowInstanceExecution
/// job with execution_type="resume" after a function completes.
/// Arguments: (instance_id, result, tenant_id, repo_id, branch)
/// Returns: Result<()>
pub type FlowResumeCallback = Arc<
    dyn Fn(
            String,            // instance_id
            serde_json::Value, // result (function execution result)
            String,            // tenant_id
            String,            // repo_id
            String,            // branch
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

/// Handler for function execution jobs
///
/// This handler processes FunctionExecution jobs. Due to the dependency structure,
/// the actual function execution is delegated to a callback provided by the
/// transport layer which has access to FunctionExecutor and FunctionApi.
pub struct FunctionExecutionHandler {
    /// Optional function executor callback (set by transport layer)
    executor: Option<FunctionExecutorCallback>,
    /// Optional function enabled checker callback (set by transport layer)
    enabled_checker: Option<FunctionEnabledChecker>,
    /// Job registry for storing results (needed to preserve logs on failure)
    job_registry: Option<Arc<JobRegistry>>,
    /// Optional flow resume callback (set by transport layer)
    /// Called after function execution completes to resume waiting flows
    flow_resumer: Option<FlowResumeCallback>,
}

impl FunctionExecutionHandler {
    /// Create a new function execution job handler
    pub fn new() -> Self {
        Self {
            executor: None,
            enabled_checker: None,
            job_registry: None,
            flow_resumer: None,
        }
    }

    /// Set the function executor callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the FunctionExecutor-based executor.
    pub fn with_executor(mut self, executor: FunctionExecutorCallback) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Set the function enabled checker callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide the callback that checks if a function is enabled.
    pub fn with_enabled_checker(mut self, checker: FunctionEnabledChecker) -> Self {
        self.enabled_checker = Some(checker);
        self
    }

    /// Set the job registry for storing results
    ///
    /// This allows storing the execution result (including logs) even when
    /// the function fails, ensuring logs are visible in SSE events.
    pub fn with_job_registry(mut self, job_registry: Arc<JobRegistry>) -> Self {
        self.job_registry = Some(job_registry);
        self
    }

    /// Set the flow resume callback
    ///
    /// This callback is called after function execution completes (success or failure)
    /// to resume any waiting flow instance that triggered this function.
    pub fn with_flow_resumer(mut self, resumer: FlowResumeCallback) -> Self {
        self.flow_resumer = Some(resumer);
        self
    }

    /// Handle function execution job
    ///
    /// If no executor is configured, returns an error indicating that
    /// function execution is not available.
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing the JobType::FunctionExecution variant
    /// * `context` - Job context with tenant, repo, branch, workspace info
    ///
    /// # Returns
    ///
    /// On success, returns the function execution result as JSON (includes logs for SSE streaming)
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract function info from JobType
        let (function_path, trigger_name, execution_id) = match &job.job_type {
            JobType::FunctionExecution {
                function_path,
                trigger_name,
                execution_id,
            } => (
                function_path.clone(),
                trigger_name.clone(),
                execution_id.clone(),
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected FunctionExecution job type".to_string(),
                ))
            }
        };

        // Validate tenant/repo context is properly set for isolation
        if context.tenant_id.is_empty() || context.repo_id.is_empty() {
            return Err(Error::Validation(format!(
                "Function execution requires non-empty tenant_id and repo_id for proper isolation (got tenant_id='{}', repo_id='{}')",
                context.tenant_id,
                context.repo_id
            )));
        }

        tracing::info!(
            job_id = %job.id,
            execution_id = %execution_id,
            function_path = %function_path,
            trigger_name = ?trigger_name,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace = %context.workspace_id,
            "Processing function execution job"
        );

        // Check if function is enabled (if enabled_checker is configured)
        if let Some(checker) = &self.enabled_checker {
            let is_enabled = checker(
                function_path.clone(),
                context.tenant_id.clone(),
                context.repo_id.clone(),
                context.branch.clone(),
                context.workspace_id.clone(),
            )
            .await?;

            if !is_enabled {
                tracing::warn!(
                    job_id = %job.id,
                    execution_id = %execution_id,
                    function_path = %function_path,
                    trigger_name = ?trigger_name,
                    "Function execution skipped: function is disabled"
                );
                return Err(Error::Validation(format!(
                    "Function '{}' is disabled",
                    function_path
                )));
            }
        }

        // Check if executor is available
        let executor = self.executor.as_ref().ok_or_else(|| {
            Error::Validation(
                "Function executor not configured. The transport layer must provide the executor callback.".to_string()
            )
        })?;

        // Get input from job context metadata
        let input = context
            .metadata
            .get("input")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // Construct log emitter for real-time streaming to SSE clients
        let log_emitter: Option<LogEmitter> = self.job_registry.as_ref().map(|registry| {
            tracing::debug!(
                job_id = %job.id,
                "FunctionExecutionHandler: created log emitter for real-time streaming"
            );
            let registry = registry.clone();
            let job_id = job.id.clone();
            Arc::new(move |level: String, message: String| {
                let r = registry.clone();
                let id = job_id.clone();
                tokio::spawn(async move {
                    r.emit_log(&id, &level, &message).await;
                });
            }) as LogEmitter
        });

        // Execute via callback.
        // Async HTTP invocations persist the caller's auth context in job
        // metadata (see the HTTP invoke handler) so the function runs with the
        // caller's permissions. Trigger-invoked functions set no such key and
        // fall back to system context (None). AI tool calls use the dedicated
        // AIToolCallExecutionHandler which passes appropriate auth.
        let auth_context: Option<AuthContext> = context
            .metadata
            .get(crate::jobs::AUTH_CONTEXT_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        // A genuine trigger firing (as opposed to the "ws"/"http" pseudo-names
        // the transports use for their own async invokes, which already log
        // their own absence-of-auth case) structurally has no caller — a
        // schema or timer trigger was never a person's request. If the
        // function declares `execution_context: "user"` (the default), it
        // still runs, with no auth context (equivalent to `system`, per
        // `execute_function`'s doc comment); this is the audit trail for that
        // fallback, not a way to prevent the trigger from firing.
        if auth_context.is_none() {
            if let Some(name) = trigger_name.as_deref() {
                if name != "ws" && name != "http" {
                    tracing::warn!(
                        job_id = %job.id,
                        execution_id = %execution_id,
                        function_path = %function_path,
                        trigger_name = %name,
                        "Trigger has no originating caller identity; function runs \
                         with no auth context (system-equivalent) regardless of its \
                         declared execution_context"
                    );
                }
            }
        }
        // The flow runtime's context, if a flow step queued this job. A job
        // with no such key (trigger, HTTP/WS async invoke) runs in a `None`
        // scope: nothing the caller supplied can produce a flow context.
        let flow_run: Option<crate::jobs::FlowRunContext> = context
            .metadata
            .get(crate::jobs::FLOW_CONTEXT_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let start = std::time::Instant::now();
        let result = crate::jobs::FLOW_RUN
            .scope(
                flow_run,
                executor(
                    function_path.clone(),
                    execution_id.clone(),
                    input,
                    context.tenant_id.clone(),
                    context.repo_id.clone(),
                    context.branch.clone(),
                    FUNCTIONS_WORKSPACE.to_string(),
                    auth_context,
                    log_emitter,
                ),
            )
            .await?;

        let elapsed = start.elapsed();

        if result.success {
            tracing::info!(
                job_id = %job.id,
                execution_id = %execution_id,
                function_path = %function_path,
                duration_ms = result.duration_ms,
                total_elapsed_ms = elapsed.as_millis(),
                log_count = result.logs.len(),
                "Function execution completed successfully"
            );
        } else {
            tracing::warn!(
                job_id = %job.id,
                execution_id = %execution_id,
                function_path = %function_path,
                error = ?result.error,
                duration_ms = result.duration_ms,
                log_count = result.logs.len(),
                "Function execution failed"
            );
        }

        // Log captured console output at debug level
        for log in &result.logs {
            tracing::debug!(
                execution_id = %execution_id,
                "Function log: {}",
                log
            );
        }

        // Serialize result to JSON for SSE streaming (includes logs)
        let result_json = serde_json::to_value(&result)
            .map_err(|e| Error::Backend(format!("Failed to serialize function result: {}", e)))?;

        // Store the result in job registry BEFORE returning error
        // This ensures logs and error details are available via SSE even on failure
        if let Some(job_registry) = &self.job_registry {
            if let Err(e) = job_registry.set_result(&job.id, result_json.clone()).await {
                tracing::warn!(
                    job_id = %job.id,
                    error = %e,
                    "Failed to store function execution result"
                );
            }
        }

        // Get instance_id if this function was triggered by a flow step
        let instance_id = context
            .metadata
            .get("instance_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Helper to resume flow with result
        let resume_flow = async |instance_id: &str, result: serde_json::Value| {
            if let Some(flow_resumer) = &self.flow_resumer {
                tracing::info!(
                    job_id = %job.id,
                    instance_id = %instance_id,
                    "Resuming flow after function execution"
                );

                if let Err(e) = flow_resumer(
                    instance_id.to_string(),
                    result,
                    context.tenant_id.clone(),
                    context.repo_id.clone(),
                    context.branch.clone(),
                )
                .await
                {
                    tracing::error!(
                        job_id = %job.id,
                        instance_id = %instance_id,
                        error = %e,
                        "Failed to resume flow after function execution"
                    );
                    // Don't fail the job if flow resume fails - the result is already stored
                }
            } else if !instance_id.is_empty() {
                tracing::warn!(
                    job_id = %job.id,
                    instance_id = %instance_id,
                    "Flow resume callback not configured - flow will not be resumed"
                );
            }
        };

        if result.success {
            // Success: resume flow immediately
            if let Some(ref instance_id) = instance_id {
                resume_flow(instance_id, result_json.clone()).await;
            }
            return Ok(Some(result_json));
        }

        // Function failed - check if this is the final retry
        // max_retries defaults to 3, retry_count is 0-indexed
        // So retry_count 2 means this is the 3rd attempt (0, 1, 2)
        let is_final_retry = job.retry_count >= 2;

        if is_final_retry {
            // Final retry failed: resume flow with error result so it can handle it
            if let Some(ref instance_id) = instance_id {
                tracing::warn!(
                    job_id = %job.id,
                    instance_id = %instance_id,
                    retry_count = job.retry_count,
                    "Final retry failed, resuming flow with error"
                );
                resume_flow(instance_id, result_json.clone()).await;
            }
        } else {
            // More retries available: DON'T resume flow yet, let retry handle it
            tracing::info!(
                job_id = %job.id,
                retry_count = job.retry_count,
                "Function failed, will retry (not resuming flow yet)"
            );
        }

        // Return error to trigger retry logic in worker
        if let Some(error_msg) = &result.error {
            return Err(Error::Backend(format!(
                "Function execution failed: {}",
                error_msg
            )));
        }

        // No error message but marked as failed
        Err(Error::Backend("Function execution failed".to_string()))
    }
}

impl Default for FunctionExecutionHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{FlowRunContext, FLOW_CONTEXT_KEY, FLOW_RUN};
    use raisin_hlc::HLC;
    use raisin_storage::jobs::{JobId, JobStatus};
    use std::collections::HashMap;

    fn job() -> JobInfo {
        JobInfo {
            id: JobId::new(),
            job_type: JobType::FunctionExecution {
                function_path: "/lib/f".into(),
                trigger_name: None,
                execution_id: "e1".into(),
            },
            status: JobStatus::Running,
            tenant: "t".into(),
            started_at: chrono::Utc::now(),
            completed_at: None,
            progress: None,
            error: None,
            result: None,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
            executing_since: None,
        }
    }

    fn context(metadata: HashMap<String, serde_json::Value>) -> JobContext {
        JobContext {
            tenant_id: "t".into(),
            repo_id: "r".into(),
            branch: "main".into(),
            workspace_id: "functions".into(),
            revision: HLC::new(0, 0),
            metadata,
        }
    }

    /// An executor that reports the flow context the call observed, as `result`.
    fn handler() -> FunctionExecutionHandler {
        FunctionExecutionHandler::new().with_executor(Arc::new(
            |_path, id, _input, _t, _r, _b, _w, _auth, _log| {
                Box::pin(async move {
                    let seen = FLOW_RUN.try_with(|f| f.clone()).ok().flatten();
                    Ok(FunctionExecutionResult {
                        execution_id: id,
                        success: true,
                        result: Some(serde_json::to_value(seen).unwrap()),
                        error: None,
                        duration_ms: 0,
                        logs: vec![],
                    })
                })
            },
        ))
    }

    /// The flow runtime's `flow_context` (written by the flow `queue_job`
    /// callback) reaches the executor as the call's flow run.
    #[tokio::test]
    async fn a_flow_step_job_runs_in_the_flow_scope() {
        let mut metadata = HashMap::new();
        metadata.insert(
            FLOW_CONTEXT_KEY.to_string(),
            serde_json::json!({
                "instance_id": "inst-1", "step_id": "remind",
                "flow_path": "/flows/f", "attempt": 1, "trigger": "flow"
            }),
        );
        let out = handler()
            .handle(&job(), &context(metadata))
            .await
            .unwrap()
            .unwrap();
        let seen: FlowRunContext = serde_json::from_value(out["result"].clone()).unwrap();
        assert_eq!(seen.instance_id, "inst-1");
        assert_eq!(seen.step_id, "remind");
        assert_eq!(seen.flow_path.as_deref(), Some("/flows/f"));
        assert_eq!(seen.attempt, 1);
    }

    /// A job queued any other way (HTTP/WS async invoke, trigger) -- even one
    /// whose INPUT claims a flow -- runs with no flow context.
    #[tokio::test]
    async fn a_job_without_the_runtime_key_has_no_flow_scope_whatever_its_input_says() {
        let mut metadata = HashMap::new();
        metadata.insert(
            "input".to_string(),
            serde_json::json!({
                "__raisin_flow": {"instance_id": "inst-1", "step_id": "remind"},
                "flow_context": {"instance_id": "inst-1", "step_id": "remind"},
            }),
        );
        let out = handler()
            .handle(&job(), &context(metadata))
            .await
            .unwrap()
            .unwrap();
        assert!(out["result"].is_null(), "{out}");
    }
}
