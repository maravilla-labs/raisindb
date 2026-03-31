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

//! AIToolCall execution handler - struct definition and main job processing
//!
//! This handler processes AIToolCallExecution jobs by executing the referenced
//! function inline and creating the AIToolResult node.

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{Storage, StorageScope};
use std::sync::Arc;

use super::types::{property_value_to_json, NodeCreatorCallback, FUNCTIONS_WORKSPACE};
use crate::jobs::handlers::function_execution::{
    FunctionExecutionResult, FunctionExecutorCallback,
};

/// Handler for AIToolCall execution jobs
///
/// This handler processes AIToolCallExecution jobs by executing the referenced
/// function inline and creating the AIToolResult node.
pub struct AIToolCallExecutionHandler<S: Storage> {
    /// Storage for node operations (used for reading nodes)
    pub(super) storage: Arc<S>,
    /// Optional function executor callback (set by transport layer)
    pub(super) executor: Option<FunctionExecutorCallback>,
    /// Optional node creator callback for creating nodes through NodeService
    /// When set, result nodes are created via NodeService for proper event publishing
    pub(super) node_creator: Option<NodeCreatorCallback>,
}

impl<S: Storage + 'static> AIToolCallExecutionHandler<S> {
    /// Create a new AIToolCall execution handler
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            executor: None,
            node_creator: None,
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

    /// Set the node creator callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide NodeService-based node creation. When set, AIToolResult nodes
    /// will be created through NodeService, ensuring proper event publishing
    /// and trigger firing.
    pub fn with_node_creator(mut self, node_creator: NodeCreatorCallback) -> Self {
        self.node_creator = Some(node_creator);
        self
    }

    /// Handle AIToolCall execution job
    ///
    /// This method:
    /// 1. Loads the AIToolCall node
    /// 2. Updates status to 'running'
    /// 3. Executes the function inline
    /// 4. Creates AIToolResult child node
    /// 5. Updates status to 'completed' or 'failed'
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract tool call info from JobType
        let (tool_call_path, tool_call_workspace) = match &job.job_type {
            JobType::AIToolCallExecution {
                tool_call_path,
                tool_call_workspace,
            } => (tool_call_path.clone(), tool_call_workspace.clone()),
            _ => {
                return Err(Error::Validation(
                    "Expected AIToolCallExecution job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            tool_call_path = %tool_call_path,
            tool_call_workspace = %tool_call_workspace,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Processing AIToolCall execution job"
        );

        // 1. Load and validate the AIToolCall node
        let tool_call_node = self
            .load_and_validate_tool_call(context, &tool_call_path, &tool_call_workspace)
            .await?;

        // Check idempotency - skip if already processed
        if let Some(skip_result) = self
            .check_idempotency(
                context,
                &tool_call_node,
                &tool_call_path,
                &tool_call_workspace,
            )
            .await?
        {
            return Ok(Some(skip_result));
        }

        // Execute the tool call with safety-net error recovery.
        // If any step fails before finalize_execution creates the result node,
        // the safety net creates an error result node so the agent loop can continue.
        let (tool_call_id, fallback_function_name) =
            self.extract_tool_call_metadata(&tool_call_node, None);
        let execution_result = self
            .execute_tool_call_inner(
                context,
                &tool_call_node,
                &tool_call_path,
                &tool_call_workspace,
            )
            .await;

        match execution_result {
            Ok(v) => Ok(v),
            Err(e) => {
                tracing::warn!(
                    tool_call_path = %tool_call_path,
                    error = %e,
                    "Tool call handler failed — creating safety-net error result node"
                );
                // Safety net: create error result node so the agent loop doesn't hang
                let _ = self
                    .create_tool_result(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        &tool_call_workspace,
                        &tool_call_path,
                        tool_call_id.as_deref(),
                        fallback_function_name.as_deref(),
                        None,
                        Some(format!("Tool execution failed: {}", e)),
                        0,
                    )
                    .await;
                let _ = self
                    .update_status(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        &tool_call_workspace,
                        &tool_call_path,
                        "failed",
                    )
                    .await;
                // Return Ok so the job system doesn't retry — the error is recorded
                Ok(Some(serde_json::json!({
                    "success": false,
                    "error": e.to_string(),
                    "recovered": true,
                })))
            }
        }
    }

    /// Timeout recovery path for AIToolCallExecution jobs.
    ///
    /// Creates an error result node (idempotent) and marks the tool call as failed
    /// so aggregation can continue and orchestration does not hang.
    pub async fn recover_timeout(
        &self,
        job: &JobInfo,
        context: &JobContext,
        timeout_error: &str,
    ) -> Result<()> {
        let (tool_call_path, tool_call_workspace) = match &job.job_type {
            JobType::AIToolCallExecution {
                tool_call_path,
                tool_call_workspace,
            } => (tool_call_path.clone(), tool_call_workspace.clone()),
            _ => return Ok(()),
        };

        let tool_call_node = self
            .load_and_validate_tool_call(context, &tool_call_path, &tool_call_workspace)
            .await?;
        let (tool_call_id, function_name) = self.extract_tool_call_metadata(&tool_call_node, None);

        // Ensure aggregator exists before result creation to keep aggregation flow unblocked.
        let _ = self
            .ensure_aggregator_exists(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &tool_call_workspace,
                &tool_call_path,
            )
            .await;

        self.create_tool_result(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &tool_call_workspace,
            &tool_call_path,
            tool_call_id.as_deref(),
            function_name.as_deref(),
            None,
            Some(timeout_error.to_string()),
            0,
        )
        .await?;

        if let Err(e) = self
            .update_status(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &tool_call_workspace,
                &tool_call_path,
                "failed",
            )
            .await
        {
            tracing::warn!(
                tool_call_path = %tool_call_path,
                error = %e,
                "Failed to mark timed-out tool call as failed during recovery"
            );
        }

        Ok(())
    }

    /// Inner execution logic for tool calls — separated for safety-net wrapping
    async fn execute_tool_call_inner(
        &self,
        context: &JobContext,
        tool_call_node: &raisin_models::nodes::Node,
        tool_call_path: &str,
        tool_call_workspace: &str,
    ) -> Result<Option<serde_json::Value>> {
        // Resolve function reference from the tool call node
        let (function_workspace, function_path) =
            self.resolve_function_ref(&tool_call_node.properties)?;

        let arguments = tool_call_node
            .properties
            .get("arguments")
            .map(property_value_to_json)
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        tracing::info!(
            tool_call_path = %tool_call_path,
            function_workspace = %function_workspace,
            function_path = %function_path,
            "Executing tool call"
        );

        // 1.5. Ensure aggregator node exists (for multi-tool coordination)
        self.ensure_aggregator_exists(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            tool_call_workspace,
            tool_call_path,
        )
        .await?;

        // 2. Update status to 'running'
        self.update_status(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            tool_call_workspace,
            tool_call_path,
            "running",
        )
        .await?;

        // 2.5. Determine auth context based on agent's execution_context setting
        let auth_context = self
            .resolve_auth_context_for_tool_call(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                tool_call_workspace,
                tool_call_path,
            )
            .await;

        // 3. Execute the function inline
        let start_time = std::time::Instant::now();
        let execution_id = nanoid::nanoid!();

        tracing::debug!(
            tool_call_path = %tool_call_path,
            function_path = %function_path,
            execution_id = %execution_id,
            auth_context_type = if auth_context.is_some() { "user" } else { "system" },
            "Starting function execution for tool call"
        );

        let result = self
            .execute_function(
                &function_path,
                &execution_id,
                arguments,
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &function_workspace,
                auth_context,
            )
            .await;

        let duration_ms = start_time.elapsed().as_millis() as u64;
        let (tool_call_id, function_name) =
            self.extract_tool_call_metadata(tool_call_node, Some(&function_path));

        tracing::debug!(
            tool_call_path = %tool_call_path,
            function_path = %function_path,
            execution_id = %execution_id,
            duration_ms = duration_ms,
            success = result.is_ok(),
            result_has_data = result.as_ref().map(|r| r.result.is_some()).unwrap_or(false),
            "Function execution completed for tool call"
        );

        // 4-5. Create result node and update status
        self.finalize_execution(
            context,
            tool_call_path,
            tool_call_workspace,
            &function_path,
            tool_call_id.as_deref(),
            function_name.as_deref(),
            result,
            duration_ms,
        )
        .await
    }

    /// Load and validate the AIToolCall node
    async fn load_and_validate_tool_call(
        &self,
        context: &JobContext,
        tool_call_path: &str,
        tool_call_workspace: &str,
    ) -> Result<raisin_models::nodes::Node> {
        use raisin_storage::NodeRepository;

        let tool_call_node = self
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    tool_call_workspace,
                ),
                tool_call_path,
                None,
            )
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!("AIToolCall node not found: {}", tool_call_path))
            })?;

        // Verify it's an AIToolCall node
        if tool_call_node.node_type != "raisin:AIToolCall" {
            return Err(Error::Validation(format!(
                "Expected raisin:AIToolCall but got {}",
                tool_call_node.node_type
            )));
        }

        Ok(tool_call_node)
    }

    /// Check idempotency - skip if already processed
    async fn check_idempotency(
        &self,
        context: &JobContext,
        tool_call_node: &raisin_models::nodes::Node,
        tool_call_path: &str,
        tool_call_workspace: &str,
    ) -> Result<Option<serde_json::Value>> {
        let status = tool_call_node
            .properties
            .get("status")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("pending");

        let has_result = self
            .has_existing_result(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                tool_call_workspace,
                tool_call_path,
            )
            .await?;

        if has_result || matches!(status, "completed" | "failed") {
            tracing::info!(
                tool_call_path = %tool_call_path,
                status = %status,
                has_result = has_result,
                "AIToolCall already processed, skipping execution"
            );
            // Ensure status reflects completion when a result exists
            if has_result && status == "pending" {
                let _ = self
                    .update_status(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        tool_call_workspace,
                        tool_call_path,
                        "completed",
                    )
                    .await;
            }
            return Ok(Some(serde_json::json!({
                "skipped": true,
                "reason": "already processed"
            })));
        }

        Ok(None)
    }

    /// Resolve function_ref from tool call properties
    fn resolve_function_ref(
        &self,
        properties: &std::collections::HashMap<String, PropertyValue>,
    ) -> Result<(String, String)> {
        match properties.get("function_ref") {
            Some(PropertyValue::Reference(r)) => {
                let ws = if r.workspace.is_empty() {
                    FUNCTIONS_WORKSPACE.to_string()
                } else {
                    r.workspace.clone()
                };
                Ok((ws, r.path.clone()))
            }
            Some(PropertyValue::Object(obj)) => {
                let ws = obj
                    .get("raisin:workspace")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| FUNCTIONS_WORKSPACE.to_string());
                let path = obj
                    .get("raisin:path")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        Error::Validation("function_ref missing raisin:path".to_string())
                    })?;
                Ok((ws, path))
            }
            Some(PropertyValue::String(name)) => {
                // Backward compatibility: treat string as function path in functions workspace
                let path = if name.starts_with('/') {
                    name.clone()
                } else {
                    format!("/{}", name)
                };
                tracing::warn!(
                    function_ref = %name,
                    resolved_path = %path,
                    "function_ref is a string; treating as path in functions workspace"
                );
                Ok((FUNCTIONS_WORKSPACE.to_string(), path))
            }
            _ => Err(Error::Validation(
                "function_ref missing or invalid".to_string(),
            )),
        }
    }

    fn extract_tool_call_metadata(
        &self,
        tool_call_node: &raisin_models::nodes::Node,
        function_path: Option<&str>,
    ) -> (Option<String>, Option<String>) {
        let tool_call_id = tool_call_node
            .properties
            .get("tool_call_id")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            });

        let function_name = tool_call_node
            .properties
            .get("function_name")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .or_else(|| {
                function_path.and_then(|path| {
                    path.split('/')
                        .next_back()
                        .map(|n| n.to_string())
                        .filter(|n| !n.is_empty())
                })
            });

        (tool_call_id, function_name)
    }

    /// Finalize execution by creating result node and updating status
    async fn finalize_execution(
        &self,
        context: &JobContext,
        tool_call_path: &str,
        tool_call_workspace: &str,
        function_path: &str,
        tool_call_id: Option<&str>,
        function_name: Option<&str>,
        result: Result<FunctionExecutionResult>,
        duration_ms: u64,
    ) -> Result<Option<serde_json::Value>> {
        match result {
            Ok(exec_result) => {
                if exec_result.success {
                    // Create AIToolResult with success
                    self.create_tool_result(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        tool_call_workspace,
                        tool_call_path,
                        tool_call_id,
                        function_name,
                        exec_result.result.clone(),
                        None,
                        duration_ms,
                    )
                    .await?;

                    // Update status to 'completed'
                    self.update_status(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        tool_call_workspace,
                        tool_call_path,
                        "completed",
                    )
                    .await?;

                    tracing::info!(
                        tool_call_path = %tool_call_path,
                        function_path = %function_path,
                        duration_ms = duration_ms,
                        "Tool call completed successfully"
                    );

                    // Return the execution result
                    Ok(serde_json::to_value(&exec_result).ok())
                } else {
                    let error_msg = exec_result
                        .error
                        .clone()
                        .unwrap_or_else(|| "Function execution returned success=false".to_string());

                    // Create AIToolResult with semantic failure details
                    self.create_tool_result(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        tool_call_workspace,
                        tool_call_path,
                        tool_call_id,
                        function_name,
                        exec_result.result.clone(),
                        Some(error_msg.clone()),
                        duration_ms,
                    )
                    .await?;

                    // Mark tool call as failed
                    self.update_status(
                        &context.tenant_id,
                        &context.repo_id,
                        &context.branch,
                        tool_call_workspace,
                        tool_call_path,
                        "failed",
                    )
                    .await?;

                    tracing::warn!(
                        tool_call_path = %tool_call_path,
                        function_path = %function_path,
                        error = %error_msg,
                        duration_ms = duration_ms,
                        "Tool call reported semantic failure (success=false)"
                    );

                    Ok(Some(serde_json::json!({
                        "success": false,
                        "error": error_msg,
                        "result": exec_result.result,
                        "duration_ms": duration_ms
                    })))
                }
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Create AIToolResult with error
                self.create_tool_result(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    tool_call_workspace,
                    tool_call_path,
                    tool_call_id,
                    function_name,
                    None,
                    Some(error_msg.clone()),
                    duration_ms,
                )
                .await?;

                // Update status to 'failed'
                self.update_status(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    tool_call_workspace,
                    tool_call_path,
                    "failed",
                )
                .await?;

                tracing::warn!(
                    tool_call_path = %tool_call_path,
                    function_path = %function_path,
                    error = %error_msg,
                    duration_ms = duration_ms,
                    "Tool call failed"
                );

                // Don't propagate error - we've created the error result node
                // This allows the agent-continue-handler to process the error
                Ok(Some(serde_json::json!({
                    "success": false,
                    "error": error_msg,
                    "duration_ms": duration_ms
                })))
            }
        }
    }
}
