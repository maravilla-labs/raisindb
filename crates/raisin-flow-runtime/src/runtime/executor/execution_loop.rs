// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Main flow execution loop with hybrid batching and OCC
//!
//! Implements the core execution loop that:
//! - Loads flow instances with version for optimistic concurrency
//! - Executes steps continuously until an async boundary
//! - Handles idempotency (skips already completed flows)
//! - Retries on version conflicts
//! - Manages compensation stack for rollback

use crate::types::{
    FlowCallbacks, FlowDefinition, FlowExecutionEvent, FlowResult, FlowStatus, StepResult,
};
use chrono::Utc;
use serde_json::Value;
use std::time::Instant;
use tracing::info;

use super::isolated_branch::execute_step;
use super::result_handlers::{handle_complete_result, handle_error_result, handle_wait_result};

/// Main flow execution function with hybrid batching.
///
/// This function implements the core execution loop:
/// 1. Loads the flow instance with version for OCC
/// 2. Checks idempotency (skips if already completed/cancelled)
/// 3. Executes steps in a loop until:
///    - An async boundary is reached (Wait result)
///    - Flow completes
///    - An error occurs
/// 4. Saves state with version check (OCC)
/// 5. Retries on version conflict (with limit)
///
/// # Arguments
///
/// * `instance_id` - The flow instance ID to execute
/// * `callbacks` - Callbacks for node operations, AI calls, etc.
///
/// # Returns
///
/// Returns `Ok(())` if execution succeeds or pauses at async boundary.
/// Returns `Err` if execution fails after retries.
pub async fn execute_flow(instance_id: &str, callbacks: &dyn FlowCallbacks) -> FlowResult<()> {
    execute_flow_with_retry(instance_id, callbacks, 0).await
}

/// Internal flow execution with version conflict retry tracking
pub(super) async fn execute_flow_with_retry(
    instance_id: &str,
    callbacks: &dyn FlowCallbacks,
    version_conflict_retries: u32,
) -> FlowResult<()> {
    info!("Starting flow execution for instance: {}", instance_id);

    // 1. Load Instance with version for OCC
    let mut instance = callbacks
        .load_instance(&format!("/flows/instances/{}", instance_id))
        .await?;

    let expected_version = instance.version;

    // 2. Skip if already completed (idempotency check)
    if instance.is_terminated() {
        info!(
            "Flow {} already in terminal state: {:?}, skipping",
            instance_id, instance.status
        );
        return Ok(());
    }

    // Mark as running if pending
    if instance.status == FlowStatus::Pending {
        instance.status = FlowStatus::Running;
        instance.started_at = Utc::now();
    }

    // Parse flow definition from snapshot
    // Supports both runtime format (step_type) and designer format (node_type)
    let flow_def = FlowDefinition::from_workflow_data(instance.flow_definition_snapshot.clone())?;

    // Track flow start time for duration calculation
    let flow_start = Instant::now();

    // Guard against unbounded SameStep re-execution
    let mut same_step_count: u32 = 0;
    const MAX_SAME_STEP_ITERATIONS: u32 = 100;

    // 3. Main execution loop - continue until async boundary or completion
    loop {
        let current_step = flow_def
            .find_node(&instance.current_node_id)
            .ok_or_else(|| {
                crate::types::FlowError::StepNotFound(instance.current_node_id.clone())
            })?;

        info!(
            "Executing step {} of type {:?}",
            current_step.id, current_step.step_type
        );

        // Emit StepStarted event
        let step_start = Instant::now();
        let step_name = current_step
            .properties
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let _ = callbacks
            .emit_event(
                instance_id,
                FlowExecutionEvent::step_started(
                    &current_step.id,
                    step_name,
                    format!("{:?}", current_step.step_type),
                ),
            )
            .await;

        // Execute step based on type
        let result = execute_step(current_step, &mut instance, &flow_def, callbacks).await;

        // Calculate step duration
        let step_duration_ms = step_start.elapsed().as_millis() as u64;

        // Handle step execution errors with event emission
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                // Emit StepFailed event
                let _ = callbacks
                    .emit_event(
                        instance_id,
                        FlowExecutionEvent::step_failed(
                            &current_step.id,
                            e.to_string(),
                            step_duration_ms,
                        ),
                    )
                    .await;
                return Err(e);
            }
        };

        // Increment step counter
        instance.metrics.step_count += 1;

        match result {
            // 4. Sync Success -> Move to next step immediately (Hot Path)
            StepResult::Continue {
                next_node_id,
                output,
            } => {
                info!(
                    "Step {} completed, continuing to {}",
                    current_step.id, next_node_id
                );

                // Emit StepCompleted event
                let _ = callbacks
                    .emit_event(
                        instance_id,
                        FlowExecutionEvent::step_completed(
                            &current_step.id,
                            output.clone(),
                            step_duration_ms,
                        ),
                    )
                    .await;

                // Merge output into variables
                if let Value::Object(map) = output {
                    if let Value::Object(ref mut vars) = instance.variables {
                        for (key, value) in map {
                            vars.insert(key, value);
                        }
                    }
                }

                instance.current_node_id = next_node_id;
                same_step_count = 0; // Reset SameStep guard on step transition
                // OPTIMIZATION: Don't persist to DB yet if next step is also sync
                // Only persist at async boundaries
            }

            // 5. Async Boundary -> Persist and Exit
            StepResult::Wait { reason, metadata } => {
                handle_wait_result(
                    instance_id,
                    &mut instance,
                    &current_step.id,
                    &reason,
                    &metadata,
                    expected_version,
                    version_conflict_retries,
                    callbacks,
                )
                .await?;
                return Ok(());
            }

            // 6. Flow completed
            StepResult::Complete { output } => {
                handle_complete_result(
                    instance_id,
                    &mut instance,
                    &current_step.id,
                    output,
                    expected_version,
                    step_duration_ms,
                    &flow_start,
                    callbacks,
                )
                .await?;
                return Ok(());
            }

            // 7. Error handling with retry, error edges, and continue-on-fail
            StepResult::Error { error } => {
                let should_return = handle_error_result(
                    instance_id,
                    &mut instance,
                    current_step,
                    &flow_def,
                    error,
                    expected_version,
                    step_duration_ms,
                    &flow_start,
                    callbacks,
                )
                .await?;
                if should_return {
                    return Ok(());
                }
                // else continue the loop (error edge or continue_on_fail)
            }

            // 8. Re-execute same step (for internal loops like AI agent iterations)
            StepResult::SameStep { metadata } => {
                same_step_count += 1;
                if same_step_count > MAX_SAME_STEP_ITERATIONS {
                    tracing::error!(
                        step_id = %current_step.id,
                        iterations = same_step_count,
                        "SameStep loop guard exceeded, failing step"
                    );
                    return Err(crate::types::FlowError::MaxIterationsExceeded {
                        limit: MAX_SAME_STEP_ITERATIONS,
                    });
                }
                tracing::debug!(
                    step_id = %current_step.id,
                    iteration = same_step_count,
                    ?metadata,
                    "Step requesting re-execution"
                );
                // Continue the loop - the step will be re-executed with updated state
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::{json, Value};

    use crate::types::{
        FlowCallbacks, FlowExecutionEvent, FlowInstance, FlowResult, FlowStatus,
    };

    // -----------------------------------------------------------------------
    // Mock FlowCallbacks
    // -----------------------------------------------------------------------

    struct MockLoopCallbacks {
        instance: Arc<Mutex<FlowInstance>>,
        events: Arc<Mutex<Vec<FlowExecutionEvent>>>,
        save_count: Arc<Mutex<u32>>,
    }

    impl MockLoopCallbacks {
        fn new(instance: FlowInstance) -> Self {
            Self {
                instance: Arc::new(Mutex::new(instance)),
                events: Arc::new(Mutex::new(Vec::new())),
                save_count: Arc::new(Mutex::new(0)),
            }
        }

        fn saved_instance(&self) -> FlowInstance {
            self.instance.lock().unwrap().clone()
        }

        fn emitted_events(&self) -> Vec<FlowExecutionEvent> {
            self.events.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl FlowCallbacks for MockLoopCallbacks {
        async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
            Ok(self.instance.lock().unwrap().clone())
        }

        async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()> {
            *self.instance.lock().unwrap() = instance.clone();
            Ok(())
        }

        async fn save_instance_with_version(
            &self,
            instance: &FlowInstance,
            _expected_version: i32,
        ) -> FlowResult<()> {
            *self.instance.lock().unwrap() = instance.clone();
            *self.save_count.lock().unwrap() += 1;
            Ok(())
        }

        async fn create_node(
            &self,
            _node_type: &str,
            _path: &str,
            _properties: Value,
        ) -> FlowResult<Value> {
            Ok(json!({}))
        }

        async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
            Ok(json!({}))
        }

        async fn get_node(&self, _path: &str) -> FlowResult<Option<Value>> {
            Ok(None)
        }

        async fn queue_job(&self, _job_type: &str, _payload: Value) -> FlowResult<String> {
            Ok("mock-job-id".to_string())
        }

        async fn call_ai(
            &self,
            _agent_workspace: &str,
            _agent_ref: &str,
            _messages: Vec<Value>,
            _response_format: Option<Value>,
        ) -> FlowResult<Value> {
            Ok(json!({ "content": "AI response" }))
        }

        async fn execute_function(
            &self,
            _function_ref: &str,
            _input: Value,
        ) -> FlowResult<Value> {
            Ok(json!({ "status": "ok" }))
        }

        async fn emit_event(
            &self,
            _instance_id: &str,
            event: FlowExecutionEvent,
        ) -> FlowResult<()> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Minimal flow: Start -> End (no intermediate steps)
    fn start_end_flow() -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "end" },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    /// Flow with a function step: Start -> FunctionStep -> End
    /// FunctionStepHandler queues a job and returns Wait, so the flow
    /// pauses at the function step (async boundary).
    fn function_step_flow(func_ref: &str) -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "step1" },
                {
                    "id": "step1",
                    "step_type": "function_step",
                    "properties": { "function_ref": func_ref, "action": "test-action" },
                    "next_node": "end"
                },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    /// Flow that simulates resuming a function step with a pre-seeded result.
    /// The instance starts at `step1` (already past start) and has
    /// `__function_result` in variables so the FunctionStepHandler returns Continue.
    fn resumed_function_flow(func_ref: &str) -> Value {
        json!({
            "nodes": [
                { "id": "start", "step_type": "start", "next_node": "step1" },
                {
                    "id": "step1",
                    "step_type": "function_step",
                    "properties": { "function_ref": func_ref, "action": "test-action" },
                    "next_node": "end"
                },
                { "id": "end", "step_type": "end" }
            ]
        })
    }

    fn make_instance(flow_def_snapshot: Value) -> FlowInstance {
        FlowInstance {
            id: "test-instance-1".to_string(),
            version: 1,
            flow_ref: "/flows/test".to_string(),
            flow_version: 1,
            flow_definition_snapshot: flow_def_snapshot,
            status: FlowStatus::Pending,
            current_node_id: "start".to_string(),
            wait_info: None,
            variables: json!({}),
            input: json!({}),
            output: None,
            compensation_stack: Vec::new(),
            error: None,
            retry_count: 0,
            started_at: chrono::Utc::now(),
            completed_at: None,
            parent_instance_ref: None,
            metrics: Default::default(),
            test_config: None,
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    /// Start -> End completes immediately with no async boundaries.
    #[tokio::test]
    async fn test_execute_start_to_end_completes() {
        let flow_def = start_end_flow();
        let instance = make_instance(flow_def);
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "Flow should complete successfully");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Completed);
        assert!(saved.output.is_some());
        assert!(saved.completed_at.is_some());
        assert!(saved.metrics.step_count >= 2); // start + end
    }

    /// Function step causes the flow to pause at an async boundary (Wait state).
    #[tokio::test]
    async fn test_execute_function_step_pauses_at_wait() {
        let flow_def = function_step_flow("/functions/greet");
        let instance = make_instance(flow_def);
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "Flow should pause without error");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Waiting);
        assert!(saved.wait_info.is_some());
        let wait_info = saved.wait_info.as_ref().unwrap();
        assert_eq!(
            wait_info.wait_type,
            crate::types::WaitType::FunctionCall
        );
    }

    /// Resuming a function step with __function_result completes the flow.
    #[tokio::test]
    async fn test_execute_resumed_function_step_completes() {
        let flow_def = resumed_function_flow("/functions/greet");
        let mut instance = make_instance(flow_def);
        // Simulate resume: instance is already at step1, status Running
        instance.current_node_id = "step1".to_string();
        instance.status = FlowStatus::Running;
        // Pre-seed the function result so FunctionStepHandler returns Continue
        instance.variables = json!({
            "__function_result": {
                "success": true,
                "result": { "greeting": "hello" }
            }
        });

        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok(), "Flow should complete after resume");

        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Completed);
        assert!(saved.output.is_some());

        // The greeting output should be in the final variables
        let output = saved.output.as_ref().unwrap();
        assert_eq!(output.get("greeting"), Some(&json!("hello")));
    }

    /// Already-completed flows are skipped (idempotency).
    #[tokio::test]
    async fn test_execute_already_completed_is_noop() {
        let flow_def = start_end_flow();
        let mut instance = make_instance(flow_def);
        instance.status = FlowStatus::Completed;
        instance.output = Some(json!({ "done": true }));

        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok());

        // save should NOT have been called since flow was already completed
        assert_eq!(*callbacks.save_count.lock().unwrap(), 0);
    }

    /// Events are emitted for step lifecycle (Start -> End flow).
    #[tokio::test]
    async fn test_execute_emits_lifecycle_events() {
        let flow_def = start_end_flow();
        let instance = make_instance(flow_def);
        let callbacks = MockLoopCallbacks::new(instance);

        let result = super::execute_flow("test-instance-1", &callbacks).await;
        assert!(result.is_ok());

        let events = callbacks.emitted_events();
        // Start -> End: step_started(start), step_completed(start),
        //   step_started(end), step_completed(end), flow_completed
        assert!(
            events.len() >= 4,
            "Expected at least 4 events, got {}",
            events.len()
        );
    }

    /// Pending instance transitions to Running before step execution.
    #[tokio::test]
    async fn test_execute_pending_becomes_running() {
        let flow_def = function_step_flow("/functions/greet");
        let instance = make_instance(flow_def);
        assert_eq!(instance.status, FlowStatus::Pending);

        let callbacks = MockLoopCallbacks::new(instance);

        let _ = super::execute_flow("test-instance-1", &callbacks).await;

        // After execution (paused at Wait), status should be Waiting
        // but it went through Running first
        let saved = callbacks.saved_instance();
        assert_eq!(saved.status, FlowStatus::Waiting);
    }
}
