// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow execution handler struct and main job processing logic
//!
//! Provides the `FlowExecutionHandler` which processes flow execution
//! jobs and manages the step-by-step execution lifecycle.

use std::sync::Arc;

use crate::types::{
    FlowCallbacks, FlowDefinition, FlowError, FlowResult, FlowStatus, StepResult, WaitInfo,
};

/// Handler for flow execution jobs
///
/// This handler processes flow execution jobs by:
/// 1. Loading the flow instance from storage
/// 2. Executing the flow using the runtime executor
/// 3. Persisting state changes at async boundaries
/// 4. Queueing new jobs when the flow needs to wait for external events
///
/// # Dependencies
///
/// The handler requires a `FlowCallbacks` implementation to interact with
/// storage and external systems. This is provided by the transport/storage layer.
///
/// # Usage
///
/// ```rust,ignore
/// use raisin_flow_runtime::integration::FlowExecutionHandler;
/// use std::sync::Arc;
///
/// // Create handler with callbacks
/// let handler = FlowExecutionHandler::new(callbacks);
///
/// // Process a flow execution job
/// let result = handler.handle(&job_info, &job_context).await?;
/// ```
pub struct FlowExecutionHandler {
    /// Callbacks for storage and external operations
    pub(super) callbacks: Arc<dyn FlowCallbacks>,
}

impl FlowExecutionHandler {
    /// Create a new flow execution handler
    ///
    /// # Arguments
    ///
    /// * `callbacks` - Implementation of FlowCallbacks for storage operations
    pub fn new(callbacks: Arc<dyn FlowCallbacks>) -> Self {
        Self { callbacks }
    }

    /// Handle a flow execution job
    ///
    /// This method is called by the job worker when a FlowExecution job is ready
    /// to be processed. It loads the flow instance, executes steps, and manages
    /// state persistence.
    ///
    /// # Arguments
    ///
    /// * `flow_instance_id` - Unique identifier for the flow instance
    /// * `tenant_id` - Tenant identifier for multi-tenancy
    /// * `repo_id` - Repository identifier
    /// * `branch` - Branch name
    ///
    /// # Returns
    ///
    /// - `Ok(Some(result))` - Flow completed successfully with result
    /// - `Ok(None)` - Flow is waiting for external event (will resume later)
    /// - `Err(e)` - Flow failed with error
    pub async fn handle(
        &self,
        flow_instance_id: &str,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
    ) -> FlowResult<Option<serde_json::Value>> {
        tracing::info!(
            flow_instance_id = %flow_instance_id,
            "Processing flow execution job"
        );

        // Load flow instance from storage
        let mut instance = self.load_instance(flow_instance_id).await?;

        // Check if instance can be executed
        if instance.is_terminated() {
            tracing::warn!(
                flow_instance_id = %flow_instance_id,
                status = ?instance.status,
                "Flow instance is already in terminal state, skipping execution"
            );
            return Ok(None);
        }

        // Update status to Running if not already
        if instance.status != FlowStatus::Running {
            instance.status = FlowStatus::Running;
        }

        // Parse flow definition from snapshot
        // Supports both runtime format (step_type) and designer format (node_type)
        let flow_definition =
            FlowDefinition::from_workflow_data(instance.flow_definition_snapshot.clone())?;

        // Build flow context from instance
        let mut context = self.build_context(&instance)?;

        // Execute current step
        let step_result = self
            .execute_current_step(&mut instance, &flow_definition, &mut context)
            .await?;

        // Handle the step result
        match step_result {
            StepResult::Continue {
                next_node_id,
                output,
            } => {
                // Record step output
                context.record_step_output(instance.current_node_id.clone(), output.clone());

                // Update instance with context changes
                self.update_instance_from_context(&mut instance, &context);

                // Move to next node
                instance.current_node_id = next_node_id;

                // Save instance state
                self.save_instance(&instance).await?;

                // Queue next step execution
                let payload = serde_json::json!({
                    "flow_instance_id": instance.id,
                });
                self.callbacks.queue_job("flow_execution", payload).await?;

                Ok(None)
            }
            StepResult::Wait { reason, metadata } => {
                // Update instance to waiting state
                instance.status = FlowStatus::Waiting;
                instance.wait_info = Some(WaitInfo {
                    subscription_id: nanoid::nanoid!(),
                    wait_type: self.determine_wait_type(&reason),
                    target_path: metadata
                        .get("target_path")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    expected_event: metadata
                        .get("expected_event")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    timeout_at: None, // TODO: Calculate from timeout config
                });

                // Update instance with context changes
                self.update_instance_from_context(&mut instance, &context);

                // Save instance state
                self.save_instance(&instance).await?;

                tracing::info!(
                    flow_instance_id = %instance.id,
                    reason = %reason,
                    "Flow paused, waiting for external event"
                );

                Ok(None)
            }
            StepResult::Complete { output } => {
                // Mark flow as completed
                instance.status = FlowStatus::Completed;
                instance.output = Some(output.clone());
                instance.completed_at = Some(chrono::Utc::now());

                // Update metrics
                if let Some(started) = instance
                    .started_at
                    .checked_sub_signed(chrono::Duration::zero())
                {
                    instance.metrics.total_duration_ms =
                        (chrono::Utc::now() - started).num_milliseconds() as u64;
                }

                // Save final state
                self.save_instance(&instance).await?;

                tracing::info!(
                    flow_instance_id = %instance.id,
                    "Flow completed successfully"
                );

                Ok(Some(output))
            }
            StepResult::Error { error } => {
                // Mark flow as failed
                instance.status = FlowStatus::Failed;
                instance.error = Some(error.to_string());
                instance.completed_at = Some(chrono::Utc::now());

                // TODO: Run compensation if configured

                // Save final state
                self.save_instance(&instance).await?;

                tracing::error!(
                    flow_instance_id = %instance.id,
                    error = %error,
                    "Flow execution failed"
                );

                Err(error)
            }
            StepResult::SameStep { metadata } => {
                // Re-execute the same step (used for AI container iterations)
                tracing::debug!(
                    flow_instance_id = %instance.id,
                    metadata = ?metadata,
                    "Re-executing same step"
                );

                // Update instance with context changes (state was saved by handler)
                self.update_instance_from_context(&mut instance, &context);

                // Save instance state
                self.save_instance(&instance).await?;

                // Queue same step execution again
                let payload = serde_json::json!({
                    "flow_instance_id": instance.id,
                });
                self.callbacks.queue_job("flow_execution", payload).await?;

                Ok(None)
            }
        }
    }

    /// Resume a flow that is waiting for an external event
    ///
    /// This is called when:
    /// - A tool result arrives
    /// - A human task is completed
    /// - A scheduled time is reached
    ///
    /// This is a wrapper around the runtime's `resume_flow` function.
    pub async fn resume_flow(
        &self,
        instance_id: &str,
        resume_data: serde_json::Value,
    ) -> FlowResult<()> {
        crate::runtime::resume_flow(instance_id, resume_data, self.callbacks.as_ref()).await
    }
}
