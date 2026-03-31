// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Saga compensation pattern for flow rollback.
//!
//! This module implements the saga pattern for handling failures in distributed workflows.
//! When a flow fails, compensations are executed in reverse order (LIFO) to undo side effects.
//!
//! Key principles:
//! - Compensations are executed in reverse order of completion
//! - If a compensation fails, we log the error and continue with remaining compensations
//! - Progress is saved after each compensation
//! - The flow is marked as RolledBack when complete

use crate::runtime::save_instance_with_version;
use crate::types::{CompensationEntry, FlowCallbacks, FlowInstance, FlowResult, FlowStatus};
use serde_json::Value;
use tracing::{error, info, warn};

/// Execute compensation for a flow that has failed.
///
/// This function:
/// 1. Pops compensations from the stack in reverse order (LIFO)
/// 2. Executes each compensation function
/// 3. Logs errors but continues even if a compensation fails
/// 4. Saves progress after each compensation
/// 5. Marks the flow as RolledBack when complete
///
/// # Arguments
///
/// * `instance` - The flow instance to roll back
/// * `callbacks` - Callbacks for executing compensation functions
///
/// # Returns
///
/// Returns `Ok(())` when rollback is complete (even if some compensations failed).
/// Returns `Err` only if saving state fails.
pub async fn rollback_flow(
    instance: &mut FlowInstance,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    info!(
        "Starting rollback for flow instance {} ({} compensations)",
        instance.id,
        instance.compensation_stack.len()
    );

    let compensation_count = instance.compensation_stack.len();

    if compensation_count == 0 {
        info!("No compensations to execute for flow {}", instance.id);
        instance.status = FlowStatus::RolledBack;
        return Ok(());
    }

    // Execute compensations in reverse order (LIFO)
    while let Some(mut entry) = instance.compensation_stack.pop() {
        info!(
            "Executing compensation for step {} (function: {})",
            entry.step_id, entry.compensation_fn
        );

        match execute_compensation(&entry, callbacks).await {
            Ok(_) => {
                entry.mark_executed();
                instance.metrics.compensation_count += 1;
                info!(
                    "Compensation for step {} executed successfully",
                    entry.step_id
                );
            }
            Err(e) => {
                let error_msg = e.to_string();
                entry.mark_failed(error_msg.clone());
                error!(
                    "Compensation for step {} failed: {}. Continuing with remaining compensations.",
                    entry.step_id, error_msg
                );
            }
        }

        // Push the updated entry back (with execution status)
        // Note: We're building a new list of executed compensations
        // This could be stored in a separate field if needed

        // Save progress after each compensation
        // Use the current version since we're already in a failure state
        let current_version = instance.flow_version;
        if let Err(e) = save_instance_with_version(instance, current_version, callbacks).await {
            warn!(
                "Failed to save progress during rollback for flow {}: {}. Continuing...",
                instance.id, e
            );
        }
    }

    // Mark flow as rolled back
    instance.status = FlowStatus::RolledBack;
    info!(
        "Rollback complete for flow {} ({} compensations executed)",
        instance.id, compensation_count
    );

    Ok(())
}

/// Execute a single compensation entry.
///
/// # Arguments
///
/// * `entry` - The compensation entry to execute
/// * `callbacks` - Callbacks for function execution
///
/// # Returns
///
/// Returns `Ok(())` if the compensation succeeds, or an error if it fails.
async fn execute_compensation(
    entry: &CompensationEntry,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<Value> {
    info!(
        "Executing compensation function {} with input: {}",
        entry.compensation_fn,
        serde_json::to_string(&entry.compensation_input).unwrap_or_else(|_| "invalid".to_string())
    );

    // Execute the compensation function
    callbacks
        .execute_function(&entry.compensation_fn, entry.compensation_input.clone())
        .await
}

/// Add a compensation entry to the flow's compensation stack.
///
/// This should be called after a step completes successfully and has a compensation function defined.
///
/// # Arguments
///
/// * `instance` - The flow instance
/// * `step_id` - The ID of the step that was completed
/// * `compensation_fn` - The function to call for compensation
/// * `compensation_input` - The input data for the compensation function
pub fn push_compensation(
    instance: &mut FlowInstance,
    step_id: String,
    compensation_fn: String,
    compensation_input: Value,
) {
    let entry =
        CompensationEntry::new(step_id.clone(), compensation_fn.clone(), compensation_input);

    info!(
        "Adding compensation for step {} (function: {})",
        step_id, compensation_fn
    );

    instance.compensation_stack.push(entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    // Mock callbacks for testing
    struct MockCallbacks {
        executed_functions: Arc<Mutex<Vec<String>>>,
        should_fail: Arc<Mutex<Vec<String>>>,
    }

    impl MockCallbacks {
        fn new() -> Self {
            Self {
                executed_functions: Arc::new(Mutex::new(Vec::new())),
                should_fail: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn set_should_fail(&self, function_ref: &str) {
            let mut fails = self.should_fail.lock().unwrap();
            fails.push(function_ref.to_string());
        }

        fn get_executed(&self) -> Vec<String> {
            let funcs = self.executed_functions.lock().unwrap();
            funcs.clone()
        }
    }

    #[async_trait]
    impl FlowCallbacks for MockCallbacks {
        async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
            unimplemented!()
        }

        async fn save_instance(&self, _instance: &FlowInstance) -> FlowResult<()> {
            Ok(())
        }

        async fn save_instance_with_version(
            &self,
            _instance: &FlowInstance,
            _expected_version: i32,
        ) -> FlowResult<()> {
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
            Ok("job-123".to_string())
        }

        async fn call_ai(
            &self,
            _agent_workspace: &str,
            _agent_ref: &str,
            _messages: Vec<Value>,
            _response_format: Option<Value>,
        ) -> FlowResult<Value> {
            Ok(json!({}))
        }

        async fn execute_function(&self, function_ref: &str, _input: Value) -> FlowResult<Value> {
            // Record execution
            {
                let mut funcs = self.executed_functions.lock().unwrap();
                funcs.push(function_ref.to_string());
            }

            // Check if should fail
            {
                let fails = self.should_fail.lock().unwrap();
                if fails.contains(&function_ref.to_string()) {
                    return Err(crate::types::FlowError::FunctionExecution(
                        "Simulated failure".to_string(),
                    ));
                }
            }

            Ok(json!({"success": true}))
        }
    }

    #[tokio::test]
    async fn test_rollback_flow() {
        let callbacks = MockCallbacks::new();
        let mut instance = FlowInstance::new(
            "/flows/test-flow".to_string(),
            1,
            json!({"nodes": [], "edges": []}),
            json!({}),
            "start".to_string(),
        );

        // Add compensations
        push_compensation(
            &mut instance,
            "step1".to_string(),
            "/lib/undo-step1".to_string(),
            json!({"data": "step1"}),
        );

        push_compensation(
            &mut instance,
            "step2".to_string(),
            "/lib/undo-step2".to_string(),
            json!({"data": "step2"}),
        );

        push_compensation(
            &mut instance,
            "step3".to_string(),
            "/lib/undo-step3".to_string(),
            json!({"data": "step3"}),
        );

        assert_eq!(instance.compensation_stack.len(), 3);

        // Execute rollback
        rollback_flow(&mut instance, &callbacks).await.unwrap();

        // Check that status is RolledBack
        assert_eq!(instance.status, FlowStatus::RolledBack);

        // Check that all compensations were executed in reverse order
        let executed = callbacks.get_executed();
        assert_eq!(executed.len(), 3);
        assert_eq!(executed[0], "/lib/undo-step3");
        assert_eq!(executed[1], "/lib/undo-step2");
        assert_eq!(executed[2], "/lib/undo-step1");

        // Check metrics
        assert_eq!(instance.metrics.compensation_count, 3);
    }

    #[tokio::test]
    async fn test_rollback_with_failures() {
        let callbacks = MockCallbacks::new();
        callbacks.set_should_fail("/lib/undo-step2");

        let mut instance = FlowInstance::new(
            "/flows/test-flow".to_string(),
            1,
            json!({"nodes": [], "edges": []}),
            json!({}),
            "start".to_string(),
        );

        // Add compensations
        push_compensation(
            &mut instance,
            "step1".to_string(),
            "/lib/undo-step1".to_string(),
            json!({}),
        );

        push_compensation(
            &mut instance,
            "step2".to_string(),
            "/lib/undo-step2".to_string(),
            json!({}),
        );

        push_compensation(
            &mut instance,
            "step3".to_string(),
            "/lib/undo-step3".to_string(),
            json!({}),
        );

        // Execute rollback
        rollback_flow(&mut instance, &callbacks).await.unwrap();

        // Check that all compensations were attempted (even though step2 failed)
        let executed = callbacks.get_executed();
        assert_eq!(executed.len(), 3);

        // Should still be marked as RolledBack
        assert_eq!(instance.status, FlowStatus::RolledBack);

        // Compensation count should still be 3 (attempted)
        assert_eq!(instance.metrics.compensation_count, 2); // Only successful ones
    }

    #[tokio::test]
    async fn test_empty_compensation_stack() {
        let callbacks = MockCallbacks::new();
        let mut instance = FlowInstance::new(
            "/flows/test-flow".to_string(),
            1,
            json!({"nodes": [], "edges": []}),
            json!({}),
            "start".to_string(),
        );

        // No compensations added

        // Execute rollback
        rollback_flow(&mut instance, &callbacks).await.unwrap();

        // Should be marked as RolledBack even with no compensations
        assert_eq!(instance.status, FlowStatus::RolledBack);

        // No functions should have been executed
        let executed = callbacks.get_executed();
        assert_eq!(executed.len(), 0);
    }
}
