//! Function step handler
//!
//! Executes RaisinDB functions by queuing them in the job system.
//! Returns immediately with a Wait result, allowing the flow to pause
//! until the function completes.
//!
//! # Example
//!
//! ```yaml
//! nodes:
//!   - id: validate-input
//!     type: function_step
//!     properties:
//!       function_ref: "/lib/validate-input"
//!       arguments:
//!         data: "{{ input.data }}"
//!       compensation_ref: "/lib/cleanup-validation"
//! ```

use super::StepHandler;
use crate::runtime::DataMapper;
use crate::types::{
    CompensationEntry, FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use tracing::{debug, error, instrument};

/// Handler for function execution steps
///
/// Queues a function for execution via the job system and returns
/// a Wait result. The flow will resume when the function completes.
#[derive(Debug)]
pub struct FunctionStepHandler;

impl FunctionStepHandler {
    /// Create a new function step handler
    pub fn new() -> Self {
        Self
    }

    /// Extract function reference from step properties
    fn get_function_ref(&self, step: &FlowNode) -> Result<String, FlowError> {
        step.get_string("function_ref").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Function step '{}' missing required property: function_ref",
                step.id
            ))
        })
    }

    /// Build function arguments from step properties and context
    ///
    /// Template expressions like `{{ input.value }}` or `${steps.prev.id}`
    /// are resolved against the flow context.
    fn build_arguments(&self, step: &FlowNode, context: &FlowContext) -> Result<Value, FlowError> {
        if let Some(args) = step.get_object("arguments") {
            DataMapper::map(&Value::Object(args.clone()), context)
        } else {
            // No arguments specified, use empty object
            Ok(Value::Object(serde_json::Map::new()))
        }
    }

    /// Add compensation to the stack after the function completed.
    ///
    /// Compensation is registered only once the forward operation actually
    /// succeeded - a function that never ran must not be compensated. The
    /// step's output is exposed as `output.*` to the
    /// `compensation_input_mapping` expressions (e.g.
    /// `{ "charge_id": "${output.charge_id}" }`).
    fn push_compensation_after_success(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        output: &Value,
    ) -> Result<(), FlowError> {
        let Some(compensation_fn) = step.get_string("compensation_ref") else {
            return Ok(());
        };

        debug!(
            "Adding compensation for step '{}': {}",
            step.id, compensation_fn
        );

        // Build compensation input: resolve the mapping against the flow
        // context (with the fresh output available), otherwise reuse the
        // forward arguments.
        let compensation_input =
            if let Some(mapping) = step.get_object("compensation_input_mapping") {
                let previous_output = context.current_output.take();
                context.current_output = Some(output.clone());
                let resolved = DataMapper::map(&Value::Object(mapping.clone()), context);
                context.current_output = previous_output;
                resolved?
            } else {
                self.build_arguments(step, context)?
            };

        let entry = CompensationEntry {
            step_id: step.id.clone(),
            completed_at: Utc::now(),
            compensation_fn,
            compensation_input,
            compensation_status: crate::types::CompensationStatus::Pending,
            executed_at: None,
        };

        context.push_compensation(entry);
        Ok(())
    }
}

impl Default for FunctionStepHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StepHandler for FunctionStepHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing function step: {}", step.id);

        // Check if we're resuming with an existing function result
        // This happens when the flow resumes after a function execution completes
        if let Some(result) = context.variables.remove("__function_result") {
            debug!(
                step_id = %step.id,
                "Found existing function result, processing cached value"
            );

            // Check if the function succeeded
            let success = result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if success {
                // Get the function's return value
                let output = result.get("result").cloned().unwrap_or(Value::Null);

                debug!(
                    step_id = %step.id,
                    "Function succeeded, continuing to next step"
                );

                // The forward operation completed - register its
                // compensation for saga rollback (if configured).
                self.push_compensation_after_success(step, context, &output)?;

                // Get next node
                let next_node_id = step.next_node.clone().unwrap_or_else(|| "end".to_string());

                return Ok(StepResult::Continue {
                    next_node_id,
                    output,
                });
            } else {
                // Function failed - return error
                let error_msg = result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Function execution failed");

                error!(
                    step_id = %step.id,
                    error = %error_msg,
                    "Function execution failed"
                );

                return Ok(StepResult::Error {
                    error: FlowError::FunctionExecution(error_msg.to_string()),
                });
            }
        }

        // No existing result - queue function execution

        // Get function reference
        let function_ref = self.get_function_ref(step)?;
        debug!("Function reference: {}", function_ref);

        // === Security: Validate execution identity and permissions ===
        let execution_identity = step
            .get_string("execution_identity")
            .unwrap_or_else(|| "agent".to_string());

        // Get caller ID from trigger info if available
        let caller_id = context.trigger_info.as_ref().and_then(|info| {
            serde_json::to_value(info)
                .ok()
                .and_then(|v| v.get("actor").and_then(|a| a.as_str()).map(String::from))
        });

        // Validate permission for function execution
        let has_permission = callbacks
            .validate_permission(
                &execution_identity,
                "execute",
                &function_ref,
                caller_id.as_deref(),
            )
            .await?;

        if !has_permission {
            // Log security audit event
            let _ = callbacks
                .audit_log(
                    "permission_denied",
                    serde_json::json!({
                        "step_id": step.id,
                        "function_ref": function_ref,
                        "identity_mode": execution_identity,
                        "caller_id": caller_id,
                        "operation": "execute",
                    }),
                )
                .await;

            error!(
                step_id = %step.id,
                function_ref = %function_ref,
                identity = %execution_identity,
                "Permission denied for function execution"
            );

            return Err(FlowError::PermissionDenied(format!(
                "Execution identity '{}' does not have permission to execute function '{}'",
                execution_identity, function_ref
            )));
        }

        // Log successful permission check for audit trail
        let _ = callbacks
            .audit_log(
                "function_execution_started",
                serde_json::json!({
                    "step_id": step.id,
                    "function_ref": function_ref,
                    "identity_mode": execution_identity,
                    "caller_id": caller_id,
                    "instance_id": context.instance_id,
                }),
            )
            .await;

        // Build arguments
        let arguments = self.build_arguments(step, context)?;
        debug!("Function arguments: {}", arguments);

        // Create job payload
        // Note: Use "function_path" to match flow_callbacks_factory.rs expectation
        // Include instance_id for flow resumption after function completes
        let payload = serde_json::json!({
            "function_path": function_ref,
            "arguments": arguments,
            "step_id": step.id,
            "instance_id": context.instance_id,
        });

        // Queue the job
        let job_id = match callbacks.queue_job("function_execution", payload).await {
            Ok(id) => {
                debug!("Function queued with job ID: {}", id);
                id
            }
            Err(e) => {
                error!("Failed to queue function: {}", e);
                return Err(FlowError::FunctionExecution(format!(
                    "Failed to queue function '{}': {}",
                    function_ref, e
                )));
            }
        };

        // Return Wait result. An optional step timeout (timeout_ms) becomes
        // the wait deadline so stuck function executions don't hang the flow.
        let mut metadata = serde_json::json!({
            "job_id": job_id,
            "function_ref": function_ref,
            "step_id": step.id,
        });
        if let Some(timeout_ms) = step
            .get_property("timeout_ms")
            .and_then(|v| v.as_u64())
            .filter(|ms| *ms > 0)
        {
            metadata["timeout_ms"] = serde_json::json!(timeout_ms);
        }

        Ok(StepResult::Wait {
            reason: "function_call".to_string(),
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompensationStatus, FlowInstance, StepType};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Mock implementation of FlowCallbacks for testing
    struct MockCallbacks {
        job_queued: Arc<Mutex<bool>>,
        should_fail: bool,
    }

    impl MockCallbacks {
        fn new() -> Self {
            Self {
                job_queued: Arc::new(Mutex::new(false)),
                should_fail: false,
            }
        }

        fn with_failure() -> Self {
            Self {
                job_queued: Arc::new(Mutex::new(false)),
                should_fail: true,
            }
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
            Ok(Value::Null)
        }

        async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
            Ok(Value::Null)
        }

        async fn get_node(&self, _path: &str) -> FlowResult<Option<Value>> {
            Ok(None)
        }

        async fn queue_job(&self, _job_type: &str, _payload: Value) -> FlowResult<String> {
            if self.should_fail {
                return Err(FlowError::FunctionExecution("Queue full".to_string()));
            }
            *self.job_queued.lock().await = true;
            Ok("job-123".to_string())
        }

        async fn call_ai(
            &self,
            _agent_workspace: &str,
            _agent_ref: &str,
            _messages: Vec<Value>,
            _response_format: Option<Value>,
        ) -> FlowResult<Value> {
            Ok(Value::Null)
        }

        async fn execute_function(&self, _function_ref: &str, _input: Value) -> FlowResult<Value> {
            Ok(Value::Null)
        }
    }

    fn create_test_context() -> FlowContext {
        FlowContext::new(
            "test-instance".to_string(),
            serde_json::json!({
                "data": "test-value"
            }),
        )
    }

    fn create_function_step_node() -> FlowNode {
        let mut properties = HashMap::new();
        properties.insert(
            "function_ref".to_string(),
            Value::String("/lib/test-function".to_string()),
        );

        let mut args = serde_json::Map::new();
        args.insert("input".to_string(), Value::String("test".to_string()));
        properties.insert("arguments".to_string(), Value::Object(args));

        FlowNode {
            id: "function-1".to_string(),
            step_type: StepType::FunctionStep,
            properties,
            children: vec![],
            next_node: Some("next-step".to_string()),
        }
    }

    #[tokio::test]
    async fn test_function_step_queues_job() {
        let handler = FunctionStepHandler::new();
        let node = create_function_step_node();
        let mut context = create_test_context();

        let callbacks = MockCallbacks::new();
        let job_queued = callbacks.job_queued.clone();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());

        match result.unwrap() {
            StepResult::Wait { reason, metadata } => {
                assert_eq!(reason, "function_call");
                assert_eq!(metadata["job_id"], "job-123");
                assert_eq!(metadata["function_ref"], "/lib/test-function");
            }
            _ => panic!("Expected Wait result"),
        }

        assert!(*job_queued.lock().await);
    }

    #[tokio::test]
    async fn test_function_step_missing_function_ref() {
        let handler = FunctionStepHandler::new();
        let mut context = create_test_context();

        let node = FlowNode {
            id: "function-1".to_string(),
            step_type: StepType::FunctionStep,
            properties: HashMap::new(),
            children: vec![],
            next_node: Some("next-step".to_string()),
        };

        let callbacks = MockCallbacks::new();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FlowError::MissingProperty(_)));
    }

    fn compensation_step_node() -> FlowNode {
        let mut properties = HashMap::new();
        properties.insert(
            "function_ref".to_string(),
            Value::String("/lib/charge-payment".to_string()),
        );
        properties.insert(
            "compensation_ref".to_string(),
            Value::String("/lib/refund-payment".to_string()),
        );

        let mut args = serde_json::Map::new();
        args.insert("amount".to_string(), Value::Number(100.into()));
        properties.insert("arguments".to_string(), Value::Object(args));

        FlowNode {
            id: "charge-step".to_string(),
            step_type: StepType::FunctionStep,
            properties,
            children: vec![],
            next_node: Some("next-step".to_string()),
        }
    }

    /// Compensation must NOT be registered at queue time - a function that
    /// never ran must not be compensated.
    #[tokio::test]
    async fn test_function_step_no_compensation_at_queue_time() {
        let handler = FunctionStepHandler::new();
        let mut context = create_test_context();
        let node = compensation_step_node();
        let callbacks = MockCallbacks::new();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), StepResult::Wait { .. }));

        assert_eq!(
            context.compensation_stack.len(),
            0,
            "compensation must only be pushed after the function succeeds"
        );
    }

    /// Compensation is registered once the function result arrives with
    /// success=true.
    #[tokio::test]
    async fn test_function_step_compensation_on_success() {
        let handler = FunctionStepHandler::new();
        let mut context = create_test_context();
        context.variables.insert(
            "__function_result".to_string(),
            serde_json::json!({
                "success": true,
                "result": { "charge_id": "ch_123" }
            }),
        );

        let node = compensation_step_node();
        let callbacks = MockCallbacks::new();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), StepResult::Continue { .. }));

        assert_eq!(context.compensation_stack.len(), 1);
        let compensation = &context.compensation_stack[0];
        assert_eq!(compensation.step_id, "charge-step");
        assert_eq!(compensation.compensation_fn, "/lib/refund-payment");
        assert!(matches!(
            compensation.compensation_status,
            CompensationStatus::Pending
        ));
    }

    /// compensation_input_mapping can reference the step output via
    /// `output.*` expressions.
    #[tokio::test]
    async fn test_function_step_compensation_input_mapping_uses_output() {
        let handler = FunctionStepHandler::new();
        let mut context = create_test_context();
        context.variables.insert(
            "__function_result".to_string(),
            serde_json::json!({
                "success": true,
                "result": { "charge_id": "ch_123" }
            }),
        );

        let mut node = compensation_step_node();
        let mut mapping = serde_json::Map::new();
        mapping.insert(
            "charge_id".to_string(),
            Value::String("${output.charge_id}".to_string()),
        );
        node.properties.insert(
            "compensation_input_mapping".to_string(),
            Value::Object(mapping),
        );

        let callbacks = MockCallbacks::new();
        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());

        let compensation = &context.compensation_stack[0];
        assert_eq!(
            compensation.compensation_input,
            serde_json::json!({"charge_id": "ch_123"})
        );
    }

    /// A failed function result must not register compensation.
    #[tokio::test]
    async fn test_function_step_no_compensation_on_failure() {
        let handler = FunctionStepHandler::new();
        let mut context = create_test_context();
        context.variables.insert(
            "__function_result".to_string(),
            serde_json::json!({
                "success": false,
                "error": "card declined"
            }),
        );

        let node = compensation_step_node();
        let callbacks = MockCallbacks::new();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), StepResult::Error { .. }));
        assert_eq!(context.compensation_stack.len(), 0);
    }

    #[tokio::test]
    async fn test_function_step_job_queue_error() {
        let handler = FunctionStepHandler::new();
        let node = create_function_step_node();
        let mut context = create_test_context();

        let callbacks = MockCallbacks::with_failure();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FlowError::FunctionExecution(_)
        ));
    }
}
