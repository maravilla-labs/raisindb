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

//! Flow instance execution handler
//!
//! This handler executes stateful flow instances using the raisin-flow-runtime crate.
//! It bridges the job system to the flow runtime by creating RocksDBFlowCallbacks with
//! actual callback implementations provided by the transport layer.
//!
//! # Architecture
//!
//! The handler:
//! 1. Extracts instance_id and execution_type from JobType::FlowInstanceExecution
//! 2. Creates RocksDBFlowCallbacks with callback implementations
//! 3. Calls raisin-flow-runtime's execute_flow function
//! 4. Handles errors and returns job result
//!
//! # Callbacks
//!
//! The transport layer must provide callbacks for:
//! - Node loading/saving/creating (for flow instance persistence)
//! - Job queuing (for async operations)
//! - AI calls (for AI steps)
//! - Function execution (for function steps)

use raisin_error::{Error, Result};
use raisin_storage::jobs::{JobContext, JobInfo, JobType};

use super::flow_callbacks::{
    AICallerCallback, AIStreamingCallerCallback, ChildrenListerCallback,
    FlowEventEmitterCallback, FunctionExecutorCallback, JobQueuerCallback, NodeCreatorCallback,
    NodeLoaderCallback, NodeSaverCallback, RocksDBFlowCallbacks,
};

/// Handler for flow instance execution jobs
///
/// This handler processes FlowInstanceExecution jobs by delegating to the
/// raisin-flow-runtime executor. It requires callbacks to be provided by
/// the transport layer for all flow operations.
pub struct FlowInstanceExecutionHandler {
    // Callbacks to be set by transport layer
    node_loader: Option<NodeLoaderCallback>,
    node_saver: Option<NodeSaverCallback>,
    node_creator: Option<NodeCreatorCallback>,
    job_queuer: Option<JobQueuerCallback>,
    ai_caller: Option<AICallerCallback>,
    ai_streaming_caller: Option<AIStreamingCallerCallback>,
    function_executor: Option<FunctionExecutorCallback>,
    children_lister: Option<ChildrenListerCallback>,
    event_emitter: Option<FlowEventEmitterCallback>,
}

impl FlowInstanceExecutionHandler {
    /// Create a new flow instance execution handler
    pub fn new() -> Self {
        Self {
            node_loader: None,
            node_saver: None,
            node_creator: None,
            job_queuer: None,
            ai_caller: None,
            ai_streaming_caller: None,
            function_executor: None,
            children_lister: None,
            event_emitter: None,
        }
    }

    /// Set the node loader callback
    ///
    /// This callback is used to load nodes from storage (flow instances, data nodes, etc.)
    pub fn with_node_loader(mut self, loader: NodeLoaderCallback) -> Self {
        self.node_loader = Some(loader);
        self
    }

    /// Set the node saver callback
    ///
    /// This callback is used to save/update nodes in storage
    pub fn with_node_saver(mut self, saver: NodeSaverCallback) -> Self {
        self.node_saver = Some(saver);
        self
    }

    /// Set the node creator callback
    ///
    /// This callback is used to create new nodes in storage
    pub fn with_node_creator(mut self, creator: NodeCreatorCallback) -> Self {
        self.node_creator = Some(creator);
        self
    }

    /// Set the job queuer callback
    ///
    /// This callback is used to queue jobs for async operations (e.g., human tasks)
    pub fn with_job_queuer(mut self, queuer: JobQueuerCallback) -> Self {
        self.job_queuer = Some(queuer);
        self
    }

    /// Set the AI caller callback
    ///
    /// This callback is used to invoke AI agents
    pub fn with_ai_caller(mut self, caller: AICallerCallback) -> Self {
        self.ai_caller = Some(caller);
        self
    }

    /// Set the streaming AI caller callback
    pub fn with_ai_streaming_caller(mut self, caller: AIStreamingCallerCallback) -> Self {
        self.ai_streaming_caller = Some(caller);
        self
    }

    /// Set the function executor callback
    ///
    /// This callback is used to execute serverless functions
    pub fn with_function_executor(mut self, executor: FunctionExecutorCallback) -> Self {
        self.function_executor = Some(executor);
        self
    }

    /// Set the children lister callback
    ///
    /// This callback is used to list child nodes for conversation history loading
    pub fn with_children_lister(mut self, lister: ChildrenListerCallback) -> Self {
        self.children_lister = Some(lister);
        self
    }

    /// Set the event emitter callback
    ///
    /// This callback is used to emit flow execution events for real-time SSE streaming
    pub fn with_event_emitter(mut self, emitter: FlowEventEmitterCallback) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Handle flow instance execution job
    ///
    /// This method:
    /// 1. Extracts flow instance info from the job
    /// 2. For "start" execution: extracts FlowInstance from metadata and saves to storage
    /// 3. Builds RocksDBFlowCallbacks with configured callbacks
    /// 4. Calls raisin-flow-runtime's execute_flow function
    /// 5. Returns execution result
    ///
    /// # Arguments
    ///
    /// * `job` - Job information containing JobType::FlowInstanceExecution
    /// * `context` - Job context with tenant, repo, branch, workspace info
    ///
    /// # Returns
    ///
    /// On success, returns a JSON object with instance_id, execution_type, and status
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        use raisin_flow_runtime::types::FlowCallbacks;

        // Extract job info
        let (instance_id, execution_type, resume_reason) = match &job.job_type {
            JobType::FlowInstanceExecution {
                instance_id,
                execution_type,
                resume_reason,
            } => (
                instance_id.clone(),
                execution_type.clone(),
                resume_reason.clone(),
            ),
            _ => {
                return Err(Error::Validation(
                    "Expected FlowInstanceExecution job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            instance_id = %instance_id,
            execution_type = %execution_type,
            resume_reason = ?resume_reason,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            branch = %context.branch,
            workspace = %context.workspace_id,
            "Processing flow instance execution job"
        );

        // Build RocksDBFlowCallbacks with the configured callbacks
        let mut callbacks = RocksDBFlowCallbacks::new(
            context.tenant_id.clone(),
            context.repo_id.clone(),
            context.branch.clone(),
        );

        // Wire up callbacks if available
        if let Some(loader) = &self.node_loader {
            callbacks = callbacks.with_node_loader(loader.clone());
        }
        if let Some(saver) = &self.node_saver {
            callbacks = callbacks.with_node_saver(saver.clone());
        }
        if let Some(creator) = &self.node_creator {
            callbacks = callbacks.with_node_creator(creator.clone());
        }
        if let Some(queuer) = &self.job_queuer {
            callbacks = callbacks.with_job_queuer(queuer.clone());
        }
        if let Some(caller) = &self.ai_caller {
            callbacks = callbacks.with_ai_caller(caller.clone());
        }
        if let Some(caller) = &self.ai_streaming_caller {
            callbacks = callbacks.with_ai_streaming_caller(caller.clone());
        }
        if let Some(executor) = &self.function_executor {
            callbacks = callbacks.with_function_executor(executor.clone());
        }
        if let Some(lister) = &self.children_lister {
            callbacks = callbacks.with_children_lister(lister.clone());
        }
        if let Some(emitter) = &self.event_emitter {
            callbacks = callbacks.with_event_emitter(emitter.clone());
        }

        // For "start" execution, extract FlowInstance from metadata and save to storage first
        if execution_type == "start" {
            if let Some(flow_instance_value) = context.metadata.get("flow_instance") {
                // Deserialize the FlowInstance from metadata
                let flow_instance: raisin_flow_runtime::types::FlowInstance =
                    serde_json::from_value(flow_instance_value.clone()).map_err(|e| {
                        Error::Validation(format!(
                            "Failed to deserialize flow instance from metadata: {}",
                            e
                        ))
                    })?;

                tracing::debug!(
                    instance_id = %flow_instance.id,
                    flow_ref = %flow_instance.flow_ref,
                    "Saving flow instance to storage before execution"
                );

                // Save the instance to storage so execute_flow can load it
                callbacks
                    .save_instance(&flow_instance)
                    .await
                    .map_err(|e| Error::Backend(format!("Failed to save flow instance: {}", e)))?;
            } else {
                tracing::warn!(
                    instance_id = %instance_id,
                    "No flow_instance in metadata for 'start' execution - instance must already exist in storage"
                );
            }
        }

        // Call raisin-flow-runtime executor
        let start = std::time::Instant::now();
        let result = match execution_type.as_str() {
            "start" => {
                // Start: execute flow from the beginning
                raisin_flow_runtime::runtime::execute_flow(&instance_id, &callbacks).await
            }
            "resume" => {
                // Resume: use resume_flow to properly handle the function result
                // Extract function result from job context metadata if available
                let resume_data = context
                    .metadata
                    .get("function_result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                tracing::debug!(
                    instance_id = %instance_id,
                    has_result = !resume_data.is_null(),
                    "Resuming flow with result data"
                );

                raisin_flow_runtime::runtime::resume_flow(&instance_id, resume_data, &callbacks)
                    .await
            }
            "timeout_check" => {
                // Timeout check: verify if a waiting flow has timed out
                // Resume with empty data - resume_flow will check timeout_at and fail if expired
                tracing::debug!(
                    instance_id = %instance_id,
                    "Checking flow timeout"
                );

                raisin_flow_runtime::runtime::resume_flow(
                    &instance_id,
                    serde_json::Value::Null,
                    &callbacks,
                )
                .await
            }
            _ => {
                return Err(Error::Validation(format!(
                    "Unknown execution type: {}",
                    execution_type
                )));
            }
        };

        let elapsed = start.elapsed();

        // Load the instance to get its final state for reporting
        // Note: load_instance expects full path, not just the instance_id
        let instance_path = format!("/flows/instances/{}", instance_id);
        let final_instance = callbacks.load_instance(&instance_path).await.ok();
        let flow_status = final_instance
            .as_ref()
            .map(|i| i.status.as_str())
            .unwrap_or("unknown");
        let flow_error = final_instance.as_ref().and_then(|i| i.error.clone());
        let current_node_id = final_instance.as_ref().map(|i| i.current_node_id.clone());

        match result {
            Ok(_) => {
                tracing::info!(
                    job_id = %job.id,
                    instance_id = %instance_id,
                    execution_type = %execution_type,
                    flow_status = %flow_status,
                    duration_ms = elapsed.as_millis(),
                    "Flow instance execution completed successfully"
                );

                Ok(Some(serde_json::json!({
                    "instance_id": instance_id,
                    "execution_type": execution_type,
                    "resume_reason": resume_reason,
                    "status": "executed",
                    "flow_status": flow_status,
                    "flow_error": flow_error,
                    "current_node_id": current_node_id,
                    "duration_ms": elapsed.as_millis(),
                })))
            }
            Err(e) => {
                tracing::error!(
                    job_id = %job.id,
                    instance_id = %instance_id,
                    execution_type = %execution_type,
                    flow_status = %flow_status,
                    error = %e,
                    duration_ms = elapsed.as_millis(),
                    "Flow instance execution failed"
                );

                // Still return success for the job but include the error info
                // This allows the job to complete while reporting the flow failure
                Ok(Some(serde_json::json!({
                    "instance_id": instance_id,
                    "execution_type": execution_type,
                    "resume_reason": resume_reason,
                    "status": "executed",
                    "flow_status": flow_status,
                    "flow_error": flow_error.or_else(|| Some(e.to_string())),
                    "current_node_id": current_node_id,
                    "duration_ms": elapsed.as_millis(),
                })))
            }
        }
    }
}

impl Default for FlowInstanceExecutionHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handler_creation() {
        let handler = FlowInstanceExecutionHandler::new();
        assert!(handler.node_loader.is_none());
        assert!(handler.node_saver.is_none());
        assert!(handler.node_creator.is_none());
        assert!(handler.job_queuer.is_none());
        assert!(handler.ai_caller.is_none());
        assert!(handler.function_executor.is_none());
        assert!(handler.children_lister.is_none());
        assert!(handler.event_emitter.is_none());
    }

    #[test]
    fn test_builder_pattern() {
        let handler = FlowInstanceExecutionHandler::new();

        // Test that we can chain builder methods (type checking only)
        let _handler = handler;
    }
}
