// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI Container step handler
//!
//! Handles AI sequence containers that orchestrate AI agent interactions
//! with configurable tool execution modes.

mod agent_ref;
mod ai_call;
mod conversation;
mod execute;
pub mod types;

pub use types::*;

use crate::types::{
    AIContainerConfig, AiExecutionConfig, FlowContext, FlowError, FlowNode, FlowResult, ToolMode,
};
use serde_json::Value;

/// Handler for AI container steps
///
/// Manages AI agent execution loops with configurable tool handling.
pub struct AiContainerHandler;

impl AiContainerHandler {
    /// Create a new AI container handler
    pub fn new() -> Self {
        Self
    }

    fn get_config(&self, step: &FlowNode) -> FlowResult<AIContainerConfig> {
        // Get agent reference
        let agent_ref = step
            .get_string_property("agent_ref")
            .or_else(|| {
                // Support Reference object format: extract raisin:path or raisin:ref as string
                step.get_property("agent_ref")
                    .and_then(|v| v.as_object())
                    .and_then(|obj| {
                        obj.get("raisin:path")
                            .or_else(|| obj.get("raisin:ref"))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
            })
            .ok_or_else(|| {
                FlowError::MissingProperty(format!(
                    "AI container '{}' missing required property: agent_ref",
                    step.id
                ))
            })?;

        // Parse tool mode
        let tool_mode = step
            .get_string_property("tool_mode")
            .and_then(|s| serde_json::from_value(Value::String(s)).ok())
            .unwrap_or(ToolMode::Auto);

        // Get explicit tools list
        let explicit_tools = step
            .get_property("explicit_tools")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Get max iterations
        let max_iterations = step.get_u32_property("max_iterations").unwrap_or(10);

        // Get thinking enabled flag
        let thinking_enabled = step.get_bool_property("thinking_enabled").unwrap_or(false);

        // Get conversation reference
        let conversation_ref = step.get_string_property("conversation_ref");

        // Get timeout configuration
        let timeout_ms = step.get_u64_property("timeout_ms").or(Some(30000));
        let total_timeout_ms = step.get_u64_property("total_timeout_ms").or(Some(300000));

        Ok(AIContainerConfig {
            agent_ref,
            tool_mode,
            explicit_tools,
            max_iterations,
            conversation_ref,
            execution: AiExecutionConfig {
                max_retries: step.get_u32_property("max_retries").unwrap_or(2),
                retry_delay_ms: step.get_u64_property("retry_delay_ms").unwrap_or(1000),
                timeout_ms,
                thinking_enabled,
            },
            total_timeout_ms,
            response_format: step.get_string_property("response_format"),
            output_schema: step.get_property("output_schema").cloned(),
        })
    }

    /// Get or initialize container state from context
    fn get_state(&self, context: &FlowContext, step_id: &str) -> AiContainerState {
        let state_key = format!("{}_state", step_id);
        context
            .get_variable(&state_key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default()
    }

    /// Save container state to context
    fn save_state(
        &self,
        context: &mut FlowContext,
        step_id: &str,
        state: &AiContainerState,
    ) -> FlowResult<()> {
        let state_key = format!("{}_state", step_id);
        let state_value = serde_json::to_value(state)?;
        context.set_variable(state_key, state_value);
        Ok(())
    }

    /// Check if we have pending tool results to process
    fn has_pending_tool_results(&self, state: &AiContainerState) -> bool {
        !state.tool_results.is_empty()
    }

    /// Process tool calls based on tool mode
    fn process_tool_calls(
        &self,
        config: &AIContainerConfig,
        tool_calls: Vec<ToolCall>,
    ) -> ToolProcessingResult {
        if tool_calls.is_empty() {
            return ToolProcessingResult::NoTools;
        }

        match config.tool_mode {
            ToolMode::Auto => {
                // All tools execute automatically
                ToolProcessingResult::AutoExecute(tool_calls)
            }
            ToolMode::Explicit => {
                // All tools become explicit steps
                ToolProcessingResult::ExplicitWait(tool_calls)
            }
            ToolMode::Hybrid => {
                // Partition tools by explicit list
                let (explicit, auto): (Vec<_>, Vec<_>) = tool_calls
                    .into_iter()
                    .partition(|tc| config.explicit_tools.contains(&tc.name));

                if !explicit.is_empty() && !auto.is_empty() {
                    // Both types present
                    ToolProcessingResult::Mixed {
                        auto_tools: auto,
                        explicit_tools: explicit,
                    }
                } else if !explicit.is_empty() {
                    // Only explicit tools
                    ToolProcessingResult::ExplicitWait(explicit)
                } else if !auto.is_empty() {
                    // Only auto tools
                    ToolProcessingResult::AutoExecute(auto)
                } else {
                    ToolProcessingResult::NoTools
                }
            }
        }
    }
}

impl Default for AiContainerHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StepType;
    use raisin_ai::Role;
    use std::collections::HashMap;

    fn create_test_context() -> FlowContext {
        FlowContext::new(
            "test-instance".to_string(),
            serde_json::json!({
                "user_message": "Hello, I need help!"
            }),
        )
    }

    fn create_ai_container_node() -> FlowNode {
        let mut properties = HashMap::new();
        properties.insert(
            "agent_ref".to_string(),
            Value::String("/agents/customer-support".to_string()),
        );
        properties.insert("tool_mode".to_string(), Value::String("auto".to_string()));
        properties.insert("max_iterations".to_string(), Value::Number(5.into()));

        FlowNode {
            id: "ai-container-1".to_string(),
            step_type: StepType::AIContainer,
            properties,
            children: vec![],
            next_node: Some("end".to_string()),
        }
    }

    #[test]
    fn test_default_state() {
        let state = AiContainerState::default();
        assert_eq!(state.iteration, 0);
        assert!(state.pending_tool_calls.is_empty());
        assert!(!state.completed);
    }

    #[test]
    fn test_state_add_tool_result() {
        let mut state = AiContainerState::default();
        state.pending_tool_calls.push(ToolCall {
            id: "call_1".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "Zurich"}),
        });

        state.add_tool_result(ToolResult {
            tool_call_id: "call_1".to_string(),
            name: "get_weather".to_string(),
            result: serde_json::json!({"temp": 15}),
            error: None,
        });

        assert!(state.pending_tool_calls.is_empty());
        assert_eq!(state.tool_results.len(), 1);
    }

    #[test]
    fn test_state_complete() {
        let mut state = AiContainerState::default();
        state.complete("Done!".to_string());

        assert!(state.completed);
        assert_eq!(state.final_response, Some("Done!".to_string()));
    }

    #[test]
    fn test_message_role_conversion() {
        let role = MessageRole::Assistant;
        let ai_role: Role = role.into();
        assert_eq!(ai_role, Role::Assistant);

        let back: MessageRole = ai_role.into();
        assert_eq!(back, MessageRole::Assistant);
    }

    #[tokio::test]
    async fn test_handler_get_config() {
        let handler = AiContainerHandler::new();
        let node = create_ai_container_node();

        let config = handler.get_config(&node).unwrap();
        assert_eq!(config.agent_ref, "/agents/customer-support");
        assert_eq!(config.tool_mode, ToolMode::Auto);
        assert_eq!(config.max_iterations, 5);
    }

    #[tokio::test]
    async fn test_handler_missing_agent_ref() {
        let handler = AiContainerHandler::new();
        let node = FlowNode {
            id: "ai-1".to_string(),
            step_type: StepType::AIContainer,
            properties: HashMap::new(),
            children: vec![],
            next_node: None,
        };

        let result = handler.get_config(&node);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), FlowError::MissingProperty(_)));
    }

    #[test]
    fn test_process_tool_calls_auto() {
        let handler = AiContainerHandler::new();
        let config = AIContainerConfig {
            agent_ref: "/agents/test".to_string(),
            tool_mode: ToolMode::Auto,
            explicit_tools: vec![],
            max_iterations: 10,
            conversation_ref: None,
            execution: AiExecutionConfig {
                max_retries: 2,
                retry_delay_ms: 1000,
                timeout_ms: Some(30000),
                thinking_enabled: false,
            },
            total_timeout_ms: Some(300000),
            response_format: None,
            output_schema: None,
        };

        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "get_data".to_string(),
            arguments: serde_json::json!({}),
        }];

        let result = handler.process_tool_calls(&config, tool_calls);
        assert!(matches!(result, ToolProcessingResult::AutoExecute(_)));
    }

    #[test]
    fn test_process_tool_calls_explicit() {
        let handler = AiContainerHandler::new();
        let config = AIContainerConfig {
            agent_ref: "/agents/test".to_string(),
            tool_mode: ToolMode::Explicit,
            explicit_tools: vec![],
            max_iterations: 10,
            conversation_ref: None,
            execution: AiExecutionConfig {
                max_retries: 2,
                retry_delay_ms: 1000,
                timeout_ms: Some(30000),
                thinking_enabled: false,
            },
            total_timeout_ms: Some(300000),
            response_format: None,
            output_schema: None,
        };

        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "get_data".to_string(),
            arguments: serde_json::json!({}),
        }];

        let result = handler.process_tool_calls(&config, tool_calls);
        assert!(matches!(result, ToolProcessingResult::ExplicitWait(_)));
    }

    // ========== Mock FlowCallbacks for integration tests ==========

    use crate::types::{FlowCallbacks, FlowExecutionEvent, FlowInstance, FlowResult, StepResult};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// Mock FlowCallbacks that returns configurable AI responses
    struct MockCallbacks {
        /// AI responses to return in sequence (one per call_ai invocation)
        ai_responses: Arc<Mutex<Vec<Value>>>,
        /// Track function executions
        executed_functions: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl MockCallbacks {
        fn new(ai_responses: Vec<Value>) -> Self {
            Self {
                ai_responses: Arc::new(Mutex::new(ai_responses)),
                executed_functions: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn get_executed_functions(&self) -> Vec<(String, Value)> {
            self.executed_functions.lock().unwrap().clone()
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
            Ok(serde_json::json!({}))
        }
        async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
            Ok(serde_json::json!({}))
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
            let mut responses = self.ai_responses.lock().unwrap();
            if responses.is_empty() {
                return Ok(serde_json::json!({
                    "content": "Default response",
                    "finish_reason": "stop"
                }));
            }
            Ok(responses.remove(0))
        }
        async fn execute_function(
            &self,
            function_ref: &str,
            input: Value,
        ) -> FlowResult<Value> {
            self.executed_functions
                .lock()
                .unwrap()
                .push((function_ref.to_string(), input));
            Ok(serde_json::json!({"result": "ok"}))
        }
    }

    use crate::handlers::StepHandler;

    #[tokio::test]
    async fn test_execute_simple_ai_response() {
        let callbacks = MockCallbacks::new(vec![serde_json::json!({
            "content": "Hello! How can I help you today?",
            "finish_reason": "stop"
        })]);
        let handler = AiContainerHandler::new();
        let node = create_ai_container_node();
        let mut context = create_test_context();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());
        match result.unwrap() {
            StepResult::Continue { next_node_id, output } => {
                assert_eq!(next_node_id, "end");
                assert_eq!(
                    output.get("response").and_then(|v| v.as_str()),
                    Some("Hello! How can I help you today?")
                );
            }
            other => panic!("Expected Continue, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_with_auto_tool_calls() {
        // First call returns a tool call, second call completes
        let callbacks = MockCallbacks::new(vec![
            serde_json::json!({
                "content": "",
                "finish_reason": "tool_calls",
                "tool_calls": [{
                    "id": "call_1",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"city\": \"Zurich\"}"
                    }
                }]
            }),
        ]);
        let handler = AiContainerHandler::new();
        let node = create_ai_container_node();
        let mut context = create_test_context();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());
        // In auto mode with tool calls, should return SameStep to continue the loop
        match result.unwrap() {
            StepResult::SameStep { .. } => {
                // Verify the tool was executed
                let executed = callbacks.get_executed_functions();
                assert_eq!(executed.len(), 1);
                assert_eq!(executed[0].0, "get_weather");
            }
            other => panic!("Expected SameStep, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_execute_max_iterations_exceeded() {
        let callbacks = MockCallbacks::new(vec![]);
        let handler = AiContainerHandler::new();

        let mut properties = HashMap::new();
        properties.insert(
            "agent_ref".to_string(),
            Value::String("/agents/test".to_string()),
        );
        properties.insert("max_iterations".to_string(), Value::Number(1.into()));

        let node = FlowNode {
            id: "ai-1".to_string(),
            step_type: StepType::AIContainer,
            properties,
            children: vec![],
            next_node: Some("end".to_string()),
        };
        let mut context = create_test_context();

        // First call should succeed (iteration 0 -> 1, which equals max_iterations=1)
        // Actually, the check is iteration >= max_iterations BEFORE increment
        // Since iteration starts at 0, and max_iterations=1, the first call
        // should execute normally. Let's set it to 0 to force immediate failure.
        let mut properties2 = HashMap::new();
        properties2.insert(
            "agent_ref".to_string(),
            Value::String("/agents/test".to_string()),
        );
        properties2.insert("max_iterations".to_string(), Value::Number(0.into()));

        let node2 = FlowNode {
            id: "ai-1".to_string(),
            step_type: StepType::AIContainer,
            properties: properties2,
            children: vec![],
            next_node: Some("end".to_string()),
        };

        let result = handler.execute(&node2, &mut context, &callbacks).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            FlowError::MaxIterationsExceeded { limit: 0 }
        ));
    }

    #[tokio::test]
    async fn test_execute_explicit_tool_mode_returns_wait() {
        let callbacks = MockCallbacks::new(vec![serde_json::json!({
            "content": "",
            "finish_reason": "tool_calls",
            "tool_calls": [{
                "id": "call_1",
                "function": {
                    "name": "approve_order",
                    "arguments": "{\"order_id\": \"123\"}"
                }
            }]
        })]);
        let handler = AiContainerHandler::new();

        let mut properties = HashMap::new();
        properties.insert(
            "agent_ref".to_string(),
            Value::String("/agents/test".to_string()),
        );
        properties.insert(
            "tool_mode".to_string(),
            Value::String("explicit".to_string()),
        );

        let node = FlowNode {
            id: "ai-explicit".to_string(),
            step_type: StepType::AIContainer,
            properties,
            children: vec![],
            next_node: Some("end".to_string()),
        };
        let mut context = create_test_context();

        let result = handler.execute(&node, &mut context, &callbacks).await;
        assert!(result.is_ok());
        match result.unwrap() {
            StepResult::Wait { reason, metadata } => {
                assert_eq!(reason, "tool_call");
                assert!(metadata.get("tool_calls").is_some());
            }
            other => panic!("Expected Wait, got {:?}", other),
        }
    }

    #[test]
    fn test_process_tool_calls_hybrid() {
        let handler = AiContainerHandler::new();
        let config = AIContainerConfig {
            agent_ref: "/agents/test".to_string(),
            tool_mode: ToolMode::Hybrid,
            explicit_tools: vec!["human_approval".to_string()],
            max_iterations: 10,
            conversation_ref: None,
            execution: AiExecutionConfig {
                max_retries: 2,
                retry_delay_ms: 1000,
                timeout_ms: Some(30000),
                thinking_enabled: false,
            },
            total_timeout_ms: Some(300000),
            response_format: None,
            output_schema: None,
        };

        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "get_data".to_string(),
                arguments: serde_json::json!({}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "human_approval".to_string(),
                arguments: serde_json::json!({}),
            },
        ];

        let result = handler.process_tool_calls(&config, tool_calls);
        if let ToolProcessingResult::Mixed {
            auto_tools,
            explicit_tools,
        } = result
        {
            assert_eq!(auto_tools.len(), 1);
            assert_eq!(auto_tools[0].name, "get_data");
            assert_eq!(explicit_tools.len(), 1);
            assert_eq!(explicit_tools[0].name, "human_approval");
        } else {
            panic!("Expected Mixed result");
        }
    }
}
