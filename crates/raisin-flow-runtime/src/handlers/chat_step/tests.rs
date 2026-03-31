// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Unit & integration tests for the chat step handler.

use super::*;
use crate::types::{
    FlowCallbacks, FlowContext, FlowExecutionEvent, FlowInstance, FlowNode, FlowResult, StepType,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Mock
// ---------------------------------------------------------------------------

struct MockChatCallbacks {
    fn_responses: Arc<Mutex<Vec<Value>>>,
}

impl MockChatCallbacks {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            fn_responses: Arc::new(Mutex::new(responses)),
        }
    }
}

#[async_trait]
impl FlowCallbacks for MockChatCallbacks {
    async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
        Err(crate::types::FlowError::Other(
            "not used in test".to_string(),
        ))
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
        Ok("job-id".to_string())
    }

    async fn call_ai(
        &self,
        _ws: &str,
        _ref_: &str,
        _msgs: Vec<Value>,
        _rf: Option<Value>,
    ) -> FlowResult<Value> {
        let mut responses = self
            .fn_responses
            .lock()
            .expect("MockChatCallbacks: poisoned mutex");
        if responses.is_empty() {
            return Ok(serde_json::json!({"content": "Default AI response"}));
        }
        Ok(responses.remove(0))
    }

    async fn execute_function(&self, _fn_ref: &str, _input: Value) -> FlowResult<Value> {
        Ok(serde_json::json!({}))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_chat_node() -> FlowNode {
    FlowNode {
        id: "chat-1".to_string(),
        step_type: StepType::Chat,
        properties: {
            let mut props = HashMap::new();
            props.insert(
                "agent_ref".to_string(),
                serde_json::json!("/agents/support"),
            );
            props.insert("max_turns".to_string(), serde_json::json!(5));
            props.insert(
                "termination".to_string(),
                serde_json::json!({
                    "allow_user_end": true,
                    "allow_ai_end": true,
                    "end_keywords": ["bye", "exit"]
                }),
            );
            props
        },
        children: vec![],
        next_node: Some("end".to_string()),
    }
}

/// Assert the result is `StepResult::Wait` and return its metadata.
fn expect_wait(result: FlowResult<StepResult>) -> Value {
    let step = result.expect("execute returned Err");
    match step {
        StepResult::Wait { metadata, .. } => metadata,
        other => {
            unreachable!("expected Wait, got {other:?}");
        }
    }
}

/// Assert the result is `StepResult::Continue` and return its output.
fn expect_continue(result: FlowResult<StepResult>) -> Value {
    let step = result.expect("execute returned Err");
    match step {
        StepResult::Continue { output, .. } => output,
        other => {
            unreachable!("expected Continue, got {other:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Config parsing (sync)
// ---------------------------------------------------------------------------

#[test]
fn test_chat_config_parsing() {
    let handler = ChatStepHandler::new();
    let step = FlowNode {
        id: "test-chat".to_string(),
        step_type: StepType::Chat,
        properties: {
            let mut props = HashMap::new();
            props.insert(
                "agent_ref".to_string(),
                serde_json::json!("/agents/support"),
            );
            props.insert("max_turns".to_string(), serde_json::json!(100));
            props.insert(
                "system_prompt".to_string(),
                serde_json::json!("You are helpful"),
            );
            props
        },
        children: Vec::new(),
        next_node: None,
    };

    let config = handler.get_chat_config(&step);
    assert_eq!(config.agent_ref, Some("/agents/support".to_string()));
    assert_eq!(config.max_turns, 100);
    assert_eq!(config.system_prompt, Some("You are helpful".to_string()));
}

#[test]
fn test_termination_keywords() {
    let handler = ChatStepHandler::new();
    let config = TerminationConfig {
        allow_user_end: true,
        allow_ai_end: true,
        end_keywords: vec!["goodbye".to_string(), "exit".to_string()],
    };

    assert!(handler.should_terminate("goodbye and thanks", &config));
    assert!(handler.should_terminate("I want to EXIT now", &config));
    assert!(!handler.should_terminate("hello there", &config));
}

#[test]
fn test_termination_disabled() {
    let handler = ChatStepHandler::new();
    let config = TerminationConfig {
        allow_user_end: false,
        allow_ai_end: true,
        end_keywords: vec!["goodbye".to_string()],
    };

    assert!(!handler.should_terminate("goodbye", &config));
}

// ---------------------------------------------------------------------------
// Async integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_execute_first_turn_waits_for_input() {
    let callbacks = MockChatCallbacks::new(vec![]);
    let handler = ChatStepHandler::new();
    let node = create_chat_node();
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    let metadata = expect_wait(handler.execute(&node, &mut context, &callbacks).await);

    assert!(metadata.get("session_id").is_some());
    assert_eq!(
        metadata.get("turn_count").and_then(|v| v.as_u64()),
        Some(0)
    );
}

#[tokio::test]
async fn test_execute_with_user_message_calls_ai() {
    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "I can help you with that!"
    })]);
    let handler = ChatStepHandler::new();
    let node = create_chat_node();
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    context.variables.insert(
        "__chat_user_message".to_string(),
        serde_json::json!("I need help with billing"),
    );

    let metadata = expect_wait(handler.execute(&node, &mut context, &callbacks).await);

    assert_eq!(
        metadata.get("turn_count").and_then(|v| v.as_u64()),
        Some(1)
    );
    assert!(!context.variables.contains_key("__chat_user_message"));
}

#[tokio::test]
async fn test_execute_termination_keyword() {
    let callbacks = MockChatCallbacks::new(vec![]);
    let handler = ChatStepHandler::new();
    let node = create_chat_node();
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    context.variables.insert(
        "__chat_user_message".to_string(),
        serde_json::json!("bye"),
    );

    let output = expect_continue(handler.execute(&node, &mut context, &callbacks).await);

    assert_eq!(
        output.get("completion_reason").and_then(|v| v.as_str()),
        Some("user_terminated")
    );
}

#[tokio::test]
async fn test_execute_max_turns_reached() {
    let callbacks = MockChatCallbacks::new(vec![
        serde_json::json!({"content": "Response 1"}),
        serde_json::json!({"content": "Response 2"}),
    ]);
    let handler = ChatStepHandler::new();

    let node = FlowNode {
        id: "chat-limit".to_string(),
        step_type: StepType::Chat,
        properties: {
            let mut props = HashMap::new();
            props.insert(
                "agent_ref".to_string(),
                serde_json::json!("/agents/test"),
            );
            props.insert("max_turns".to_string(), serde_json::json!(2));
            props
        },
        children: vec![],
        next_node: Some("end".to_string()),
    };
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    // Turn 1
    context.variables.insert(
        "__chat_user_message".to_string(),
        serde_json::json!("msg1"),
    );
    let r1 = handler
        .execute(&node, &mut context, &callbacks)
        .await
        .expect("turn 1");
    assert!(matches!(r1, StepResult::Wait { .. }));

    // Turn 2
    context.variables.insert(
        "__chat_user_message".to_string(),
        serde_json::json!("msg2"),
    );
    let output = expect_continue(handler.execute(&node, &mut context, &callbacks).await);

    assert_eq!(
        output.get("completion_reason").and_then(|v| v.as_str()),
        Some("max_turns_reached")
    );
    assert_eq!(output.get("turn_count").and_then(|v| v.as_u64()), Some(2));
}

#[tokio::test]
async fn test_execute_ai_end_session_signal() {
    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "Glad I could help. Goodbye!",
        "end_session": true
    })]);
    let handler = ChatStepHandler::new();

    let node = FlowNode {
        id: "chat-ai-end".to_string(),
        step_type: StepType::Chat,
        properties: {
            let mut props = HashMap::new();
            props.insert(
                "agent_ref".to_string(),
                serde_json::json!("/agents/test"),
            );
            props.insert(
                "termination".to_string(),
                serde_json::json!({
                    "allow_user_end": true,
                    "allow_ai_end": true,
                    "end_keywords": []
                }),
            );
            props
        },
        children: vec![],
        next_node: Some("end".to_string()),
    };
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    context.variables.insert(
        "__chat_user_message".to_string(),
        serde_json::json!("Thanks!"),
    );

    let output = expect_continue(handler.execute(&node, &mut context, &callbacks).await);

    assert_eq!(
        output.get("completion_reason").and_then(|v| v.as_str()),
        Some("ai_terminated")
    );
}

#[tokio::test]
async fn test_agent_ref_fallback_from_flow_input() {
    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "Hello from the agent!"
    })]);
    let handler = ChatStepHandler::new();

    let node = FlowNode {
        id: "chat-no-agent".to_string(),
        step_type: StepType::Chat,
        properties: HashMap::new(),
        children: vec![],
        next_node: Some("end".to_string()),
    };

    let mut context = FlowContext::new(
        "test".to_string(),
        serde_json::json!({ "agent": "/agents/sample-assistant" }),
    );

    context.variables.insert(
        "__chat_user_message".to_string(),
        serde_json::json!("Hi there"),
    );

    let metadata = expect_wait(handler.execute(&node, &mut context, &callbacks).await);

    assert_eq!(
        metadata.get("turn_count").and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        metadata.get("current_agent").and_then(|v| v.as_str()),
        Some("/agents/sample-assistant")
    );
}

#[tokio::test]
async fn test_execute_with_tool_calls() {
    let callbacks = MockChatCallbacks::new(vec![
        serde_json::json!({
            "content": "",
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "create_plan",
                    "arguments": "{\"title\": \"Test Plan\", \"tasks\": [{\"title\": \"Task 1\"}]}"
                }
            }],
            "finish_reason": "tool_calls"
        }),
        serde_json::json!({
            "content": "I've created a plan with one task.",
            "finish_reason": "stop"
        }),
    ]);
    let handler = ChatStepHandler::new();
    let node = create_chat_node();
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    context.variables.insert(
        "__chat_user_message".to_string(),
        serde_json::json!("Make a plan for testing"),
    );

    let metadata = expect_wait(handler.execute(&node, &mut context, &callbacks).await);

    assert_eq!(
        metadata.get("turn_count").and_then(|v| v.as_u64()),
        Some(1)
    );
}
