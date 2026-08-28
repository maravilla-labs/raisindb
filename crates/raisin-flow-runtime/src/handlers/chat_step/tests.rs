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
    /// Every node created, as `(path, node_type)`.
    created: Arc<Mutex<Vec<(String, String)>>>,
    /// The tool definitions offered on the most recent AI call.
    offered_tools: Arc<Mutex<Vec<Value>>>,
    /// How many times the model was called.
    ai_calls: Arc<Mutex<u32>>,
}

impl MockChatCallbacks {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            fn_responses: Arc::new(Mutex::new(responses)),
            created: Arc::new(Mutex::new(Vec::new())),
            offered_tools: Arc::new(Mutex::new(Vec::new())),
            ai_calls: Arc::new(Mutex::new(0)),
        }
    }

    fn offered(&self, name: &str) -> Option<Value> {
        self.offered_tools
            .lock()
            .unwrap()
            .iter()
            .find(|t| t["function"]["name"] == name)
            .cloned()
    }

    fn created_paths(&self) -> Vec<String> {
        self.created
            .lock()
            .unwrap()
            .iter()
            .map(|(p, _)| p.clone())
            .collect()
    }

    fn ai_call_count(&self) -> u32 {
        *self.ai_calls.lock().unwrap()
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

    // The chat step reaches the model through the *_with_tools pair. Overriding
    // both is what lets a test assert on the CONTROL TOOLS a step offered —
    // the trait defaults drop them, which is fine in production for a callback
    // with no provider behind it but would make these tests vacuous.
    async fn call_ai_with_tools(
        &self,
        ws: &str,
        ref_: &str,
        msgs: Vec<Value>,
        rf: Option<Value>,
        extra_tools: Vec<Value>,
    ) -> FlowResult<Value> {
        *self.offered_tools.lock().unwrap() = extra_tools;
        *self.ai_calls.lock().unwrap() += 1;
        self.call_ai(ws, ref_, msgs, rf).await
    }

    async fn call_ai_streaming_with_tools(
        &self,
        ws: &str,
        ref_: &str,
        msgs: Vec<Value>,
        rf: Option<Value>,
        extra_tools: Vec<Value>,
    ) -> FlowResult<tokio::sync::mpsc::Receiver<Value>> {
        let response = self
            .call_ai_with_tools(ws, ref_, msgs, rf, extra_tools)
            .await?;
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let _ = tx.send(response).await;
        Ok(rx)
    }

    async fn create_node_in_workspace(
        &self,
        _workspace: &str,
        node_type: &str,
        path: &str,
        _properties: Value,
    ) -> FlowResult<Value> {
        self.created
            .lock()
            .unwrap()
            .push((path.to_string(), node_type.to_string()));
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
    assert_eq!(metadata.get("turn_count").and_then(|v| v.as_u64()), Some(0));
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

    assert_eq!(metadata.get("turn_count").and_then(|v| v.as_u64()), Some(1));
    assert!(!context.variables.contains_key("__chat_user_message"));
}

#[tokio::test]
async fn test_execute_termination_keyword() {
    let callbacks = MockChatCallbacks::new(vec![]);
    let handler = ChatStepHandler::new();
    let node = create_chat_node();
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    context
        .variables
        .insert("__chat_user_message".to_string(), serde_json::json!("bye"));

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
            props.insert("agent_ref".to_string(), serde_json::json!("/agents/test"));
            props.insert("max_turns".to_string(), serde_json::json!(2));
            props
        },
        children: vec![],
        next_node: Some("end".to_string()),
    };
    let mut context = FlowContext::new("test".to_string(), serde_json::json!({}));

    // Turn 1
    context
        .variables
        .insert("__chat_user_message".to_string(), serde_json::json!("msg1"));
    let r1 = handler
        .execute(&node, &mut context, &callbacks)
        .await
        .expect("turn 1");
    assert!(matches!(r1, StepResult::Wait { .. }));

    // Turn 2
    context
        .variables
        .insert("__chat_user_message".to_string(), serde_json::json!("msg2"));
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
            props.insert("agent_ref".to_string(), serde_json::json!("/agents/test"));
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

    assert_eq!(metadata.get("turn_count").and_then(|v| v.as_u64()), Some(1));
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

    assert_eq!(metadata.get("turn_count").and_then(|v| v.as_u64()), Some(1));
}

// ---------------------------------------------------------------------------
// Control tools: ending with a result, and agent-initiated approval
//
// Everything below covers behaviour the chat step ADVERTISED but could not do.
// `termination.allow_ai_end` gated on an `end_session` flag that only a
// provider chunk could set, and no provider sets it — so an agent could never
// end a session, and `handoff_targets` were prose with no verb behind them.
// ---------------------------------------------------------------------------

/// A tool call in the shape a provider returns (arguments as a JSON STRING,
/// which is the form that actually arrives and the form `parse_tool_arguments`
/// exists to handle).
fn tool_call(id: &str, name: &str, args: Value) -> Value {
    serde_json::json!({
        "id": id,
        "type": "function",
        "function": { "name": name, "arguments": args.to_string() },
    })
}

/// A chat context carrying a first-turn user message.
fn context_with_message(msg: &str) -> FlowContext {
    FlowContext::new(
        "inst-ctl".to_string(),
        serde_json::json!({ "message": msg }),
    )
}

#[tokio::test]
async fn agent_ends_the_session_with_a_result() {
    let mut node = create_chat_node();
    node.properties.insert(
        "output_schema".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": { "verdict": { "type": "string" } },
        }),
    );

    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "All done.",
        "tool_calls": [tool_call(
            "tc1",
            "end_session",
            serde_json::json!({ "result": { "verdict": "refund" }, "reason": "resolved" }),
        )],
    })]);

    let mut context = context_with_message("I want a refund");
    let output = expect_continue(
        ChatStepHandler::new()
            .execute(&node, &mut context, &callbacks)
            .await,
    );

    // The RESULT is the point — a completion reason alone leaves the next step
    // with nothing to branch on.
    assert_eq!(output["result"]["verdict"], "refund");
    assert_eq!(output["completion_reason"], "ai_terminated");

    // And the tool the agent used was shaped by the step's output_schema.
    let end = callbacks
        .offered("end_session")
        .expect("end_session must be offered when allow_ai_end is on");
    assert_eq!(
        end["function"]["parameters"]["properties"]["result"]["properties"]["verdict"]["type"],
        "string"
    );
}

#[tokio::test]
async fn end_session_is_not_offered_when_ai_end_is_off() {
    let mut node = create_chat_node();
    node.properties.insert(
        "termination".to_string(),
        serde_json::json!({ "allow_user_end": true, "allow_ai_end": false }),
    );

    // The model calls it anyway (a hallucinated tool). It must be refused, not
    // acted on: routing it onward would end a session the author forbade
    // ending, and falling through to execute_function would invoke the tool
    // NAME as a function path.
    let callbacks = MockChatCallbacks::new(vec![
        serde_json::json!({
            "content": "",
            "tool_calls": [tool_call("tc1", "end_session", serde_json::json!({"result": {}}))],
        }),
        serde_json::json!({ "content": "Carrying on." }),
    ]);

    let mut context = context_with_message("hello");
    let result = ChatStepHandler::new()
        .execute(&node, &mut context, &callbacks)
        .await;

    assert!(callbacks.offered("end_session").is_none());
    // Still waiting for the user — the session did not end.
    let metadata = expect_wait(result);
    assert_eq!(metadata["turn_count"], 1);
}

#[tokio::test]
async fn agent_requested_approval_parks_the_flow() {
    let mut node = create_chat_node();
    node.properties.insert(
        "approval".to_string(),
        serde_json::json!({ "enabled": true, "assignee": "/users/editor" }),
    );

    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "Let me check with someone.",
        "tool_calls": [tool_call(
            "tc1",
            "request_approval",
            serde_json::json!({ "title": "Issue a refund?", "description": "€400" }),
        )],
    })]);

    let mut context = context_with_message("refund me");
    let metadata = expect_wait(
        ChatStepHandler::new()
            .execute(&node, &mut context, &callbacks)
            .await,
    );

    // Parked on the TASK, which is what WaitInfo records and what the timeout
    // path marks expired.
    let target = metadata["target_path"].as_str().expect("target_path");
    assert!(
        target.starts_with("/users/editor/inbox/task-"),
        "task filed at {target}"
    );
    assert_eq!(metadata["approval_title"], "Issue a refund?");

    let paths = callbacks.created_paths();
    assert!(paths.iter().any(|p| p == target), "inbox task was created");

    // The pending tool call must NOT have a result yet: a result in the history
    // is exactly what tells the model its question was already answered.
    assert!(
        !paths.iter().any(|p| p.ends_with("/tc1/result")),
        "no tool result may be written while the approval is pending: {paths:?}"
    );
}

#[tokio::test]
async fn an_answered_approval_resumes_the_same_turn() {
    let mut node = create_chat_node();
    node.properties.insert(
        "approval".to_string(),
        serde_json::json!({ "enabled": true, "assignee": "/users/editor" }),
    );

    let callbacks = MockChatCallbacks::new(vec![
        serde_json::json!({
            "content": "Checking.",
            "tool_calls": [tool_call(
                "tc1",
                "request_approval",
                serde_json::json!({ "title": "Issue a refund?" }),
            )],
        }),
        serde_json::json!({ "content": "Approved — refund issued." }),
    ]);

    let handler = ChatStepHandler::new();
    let mut context = context_with_message("refund me");
    expect_wait(handler.execute(&node, &mut context, &callbacks).await);
    assert_eq!(callbacks.ai_call_count(), 1);

    // The human answers. `resume_flow` delivers a HumanTask answer this way.
    context.set_variable(
        "__human_response".to_string(),
        serde_json::json!({ "value": "approve" }),
    );
    context.set_variable("__human_response_pending".to_string(), Value::Bool(true));

    let metadata = expect_wait(handler.execute(&node, &mut context, &callbacks).await);

    // The answer was written as the pending call's RESULT, so the next history
    // rebuild shows the model its own question answered.
    assert!(
        callbacks
            .created_paths()
            .iter()
            .any(|p| p.ends_with("/tc1/result")),
        "the human's answer must land as the tool call's result"
    );
    // And the agent got another turn without the user having to say anything.
    assert_eq!(callbacks.ai_call_count(), 2);
    assert_eq!(metadata["turn_count"], 1, "an approval is not a user turn");
}

#[tokio::test]
async fn a_redelivery_while_parked_does_not_re_ask() {
    let mut node = create_chat_node();
    node.properties.insert(
        "approval".to_string(),
        serde_json::json!({ "enabled": true, "assignee": "/users/editor" }),
    );

    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "Checking.",
        "tool_calls": [tool_call("tc1", "request_approval", serde_json::json!({"title": "Ok?"}))],
    })]);

    let handler = ChatStepHandler::new();
    let mut context = context_with_message("go");
    expect_wait(handler.execute(&node, &mut context, &callbacks).await);

    // Same step executed again with no answer delivered (a job redelivery, or a
    // timeout check). It must re-park, NOT call the model again — which would
    // ask the person a second time and orphan the first task.
    let metadata = expect_wait(handler.execute(&node, &mut context, &callbacks).await);
    assert_eq!(
        callbacks.ai_call_count(),
        1,
        "the model must not be re-asked"
    );
    assert!(
        metadata["target_path"].is_string(),
        "still parked on the task"
    );
}

#[tokio::test]
async fn approval_without_an_assignee_does_not_park() {
    // Nobody to ask. Parking would wait forever on a task that was never filed,
    // so the conversation must carry on instead.
    let mut node = create_chat_node();
    node.properties.insert(
        "approval".to_string(),
        serde_json::json!({ "enabled": true }),
    );

    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "Checking.",
        "tool_calls": [tool_call("tc1", "request_approval", serde_json::json!({"title": "Ok?"}))],
    })]);

    let mut context = context_with_message("go");
    let metadata = expect_wait(
        ChatStepHandler::new()
            .execute(&node, &mut context, &callbacks)
            .await,
    );

    // Waiting on the USER, not on a task.
    assert!(metadata["target_path"].is_null());
    assert!(!callbacks
        .created_paths()
        .iter()
        .any(|p| p.contains("/inbox/task-")));
}

#[tokio::test]
async fn handoff_targets_become_a_real_tool() {
    let mut node = create_chat_node();
    node.properties.insert(
        "handoff_targets".to_string(),
        serde_json::json!([
            { "agent_ref": "/agents/billing", "description": "Payments and refunds" },
        ]),
    );

    let callbacks = MockChatCallbacks::new(vec![serde_json::json!({
        "content": "",
        "tool_calls": [tool_call(
            "tc1",
            "handoff_to_agent",
            serde_json::json!({ "agent": "/agents/billing" }),
        )],
    })]);

    let mut context = context_with_message("I have a billing question");
    expect_wait(
        ChatStepHandler::new()
            .execute(&node, &mut context, &callbacks)
            .await,
    );

    let handoff = callbacks
        .offered("handoff_to_agent")
        .expect("a handoff target must produce a handoff TOOL, not just prose");
    assert_eq!(
        handoff["function"]["parameters"]["properties"]["agent"]["enum"][0],
        "/agents/billing"
    );

    // And the session is now that agent's.
    let session: Value = context.variables["__chat_session_chat-1"].clone();
    assert_eq!(session["current_agent"], "/agents/billing");
}
