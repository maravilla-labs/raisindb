use super::*;
use crate::types::{FlowInstance, FlowMetrics, WaitInfo};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

// ---------------------------------------------------------------------------
// Mock callbacks
// ---------------------------------------------------------------------------

struct MockCallbacks {
    instance: std::sync::Mutex<FlowInstance>,
    save_called: std::sync::atomic::AtomicBool,
}

impl MockCallbacks {
    fn new(instance: FlowInstance) -> Self {
        Self {
            instance: std::sync::Mutex::new(instance),
            save_called: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn was_save_called(&self) -> bool {
        self.save_called.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl FlowCallbacks for MockCallbacks {
    async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
        Ok(self.instance.lock().unwrap().clone())
    }

    async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()> {
        *self.instance.lock().unwrap() = instance.clone();
        self.save_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn save_instance_with_version(
        &self,
        instance: &FlowInstance,
        _expected_version: i32,
    ) -> FlowResult<()> {
        *self.instance.lock().unwrap() = instance.clone();
        self.save_called
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn create_node(&self, _: &str, _: &str, _: Value) -> FlowResult<Value> {
        Ok(Value::Null)
    }
    async fn update_node(&self, _: &str, _: Value) -> FlowResult<Value> {
        Ok(Value::Null)
    }
    async fn get_node(&self, _: &str) -> FlowResult<Option<Value>> {
        Ok(None)
    }
    async fn queue_job(&self, _: &str, _: Value) -> FlowResult<String> {
        Ok("test-job-id".to_string())
    }
    async fn call_ai(
        &self,
        _: &str,
        _: &str,
        _: Vec<Value>,
        _: Option<Value>,
    ) -> FlowResult<Value> {
        Ok(Value::Null)
    }
    async fn execute_function(&self, _: &str, _: Value) -> FlowResult<Value> {
        Ok(Value::Null)
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn test_flow_definition() -> Value {
    json!({
        "name": "test-flow",
        "version": 1,
        "nodes": [
            { "id": "start", "step_type": "start", "properties": {}, "next_node": "end" },
            { "id": "end", "step_type": "end", "properties": {} }
        ],
        "edges": [{"from": "start", "to": "end"}]
    })
}

/// Build a minimal test FlowInstance. Caller can override fields after creation.
fn waiting_instance(node_id: &str, wait_info: Option<WaitInfo>) -> FlowInstance {
    FlowInstance {
        id: "test-instance".to_string(),
        version: 1,
        flow_ref: "/flows/test".to_string(),
        flow_version: 1,
        flow_definition_snapshot: test_flow_definition(),
        status: FlowStatus::Waiting,
        current_node_id: node_id.to_string(),
        wait_info,
        variables: Value::Object(serde_json::Map::new()),
        input: json!({"test":"input"}),
        output: None,
        compensation_stack: Vec::new(),
        error: None,
        retry_count: 0,
        started_at: Utc::now(),
        completed_at: None,
        parent_instance_ref: None,
        metrics: FlowMetrics::default(),
        test_config: None,
    }
}

fn simple_wait(wait_type: WaitType) -> Option<WaitInfo> {
    Some(WaitInfo {
        subscription_id: "sub-test".to_string(),
        wait_type,
        target_path: None,
        expected_event: None,
        timeout_at: None,
    })
}

// ---------------------------------------------------------------------------
// Tests: status idempotency
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resume_already_running_is_idempotent() {
    let mut inst = waiting_instance("start", None);
    inst.status = FlowStatus::Running;
    let cb = MockCallbacks::new(inst);
    assert!(resume_flow("test-instance", json!({}), &cb).await.is_ok());
}

#[tokio::test]
async fn test_resume_completed_is_idempotent() {
    let mut inst = waiting_instance("end", None);
    inst.status = FlowStatus::Completed;
    inst.output = Some(json!({"result": "done"}));
    inst.completed_at = Some(Utc::now());
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", json!({}), &cb).await;
    assert!(result.is_ok());
    assert!(!cb.was_save_called());
}

#[tokio::test]
async fn test_resume_failed_is_idempotent() {
    let mut inst = waiting_instance("some-step", None);
    inst.status = FlowStatus::Failed;
    inst.error = Some("Something went wrong".to_string());
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", json!({}), &cb).await;
    assert!(result.is_ok());
    assert!(!cb.was_save_called());
}

#[tokio::test]
async fn test_resume_cancelled_is_idempotent() {
    let mut inst = waiting_instance("some-step", None);
    inst.status = FlowStatus::Cancelled;
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", json!({}), &cb).await;
    assert!(result.is_ok());
    assert!(!cb.was_save_called());
}

#[tokio::test]
async fn test_resume_pending_fails() {
    let mut inst = waiting_instance("start", None);
    inst.status = FlowStatus::Pending;
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", json!({}), &cb).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        FlowError::InvalidStateTransition { from, to } => {
            assert_eq!(from, "pending");
            assert_eq!(to, "running");
        }
        other => panic!("Expected InvalidStateTransition, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Tests: resume data routing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resume_with_tool_result() {
    let inst = waiting_instance("ai-step", simple_wait(WaitType::ToolCall));
    let tool_result = json!({"tool_name": "get_weather", "result": {"temperature": 72}});
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", tool_result.clone(), &cb).await;

    assert!(cb.was_save_called());
    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Running);
    assert!(updated.wait_info.is_none());
    assert_eq!(
        updated
            .variables
            .as_object()
            .unwrap()
            .get("__last_tool_result"),
        Some(&tool_result)
    );
}

#[tokio::test]
async fn test_resume_with_human_response() {
    let inst = waiting_instance("human-task", simple_wait(WaitType::HumanTask));
    let response = json!({"approved": true, "comments": "Looks good!"});
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", response.clone(), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(
        updated
            .variables
            .as_object()
            .unwrap()
            .get("__human_response"),
        Some(&response)
    );
}

#[tokio::test]
async fn test_resume_retry_preserves_count() {
    // retry_count must survive the retry resume so max_retries is actually
    // enforced on the next failure (it is reset when the step succeeds).
    let mut inst = waiting_instance("retry-step", simple_wait(WaitType::Retry));
    inst.retry_count = 3;
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", json!({}), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.retry_count, 3);
}

#[tokio::test]
async fn test_resume_chat_session_message_key() {
    let inst = waiting_instance("chat-step", simple_wait(WaitType::ChatSession));
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", json!({"message": "Hello from user"}), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(
        updated
            .variables
            .as_object()
            .unwrap()
            .get("__chat_user_message"),
        Some(&json!("Hello from user"))
    );
    assert_eq!(updated.status, FlowStatus::Running);
}

#[tokio::test]
async fn test_resume_chat_session_content_key() {
    let inst = waiting_instance("chat-step", simple_wait(WaitType::ChatSession));
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow(
        "test-instance",
        json!({"content": "User content via content key"}),
        &cb,
    )
    .await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(
        updated
            .variables
            .as_object()
            .unwrap()
            .get("__chat_user_message"),
        Some(&json!("User content via content key"))
    );
}

#[tokio::test]
async fn test_resume_chat_session_raw_string() {
    let inst = waiting_instance("chat-step", simple_wait(WaitType::ChatSession));
    let data = json!("Just a plain string message");
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", data.clone(), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(
        updated
            .variables
            .as_object()
            .unwrap()
            .get("__chat_user_message"),
        Some(&data)
    );
}

#[tokio::test]
async fn test_resume_waiting_without_wait_info() {
    let inst = waiting_instance("some-step", None);
    let data = json!({"key": "value"});
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", data.clone(), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(
        updated.variables.as_object().unwrap().get("__resume_data"),
        Some(&data)
    );
    assert_eq!(updated.status, FlowStatus::Running);
}

// ---------------------------------------------------------------------------
// Tests: function call failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resume_function_call_failure_routes_through_error_handling() {
    // A failed function result must NOT short-circuit the flow to Failed -
    // it re-executes the step so the failure flows through the full error
    // machinery (retry / error_edge / continue_on_fail / rollback).
    let flow_def = json!({
        "name": "fn-fail-flow",
        "version": 1,
        "nodes": [
            { "id": "start", "step_type": "start", "properties": {}, "next_node": "fn-step" },
            {
                "id": "fn-step",
                "step_type": "function_step",
                "properties": { "function_ref": "/lib/fails", "max_retries": 0 },
                "next_node": "end"
            },
            { "id": "end", "step_type": "end", "properties": {} }
        ]
    });

    let mut inst = waiting_instance("fn-step", simple_wait(WaitType::FunctionCall));
    inst.flow_definition_snapshot = flow_def;
    let data = json!({"success": false, "error": "Function execution timeout"});
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", data, &cb).await;

    // With max_retries=0, no error_edge, and nothing to compensate, the
    // flow ends up Failed and the error is reported.
    assert!(result.is_err());
    assert!(cb.was_save_called());
    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Failed);
    assert!(updated
        .error
        .as_deref()
        .unwrap_or("")
        .contains("Function execution timeout"));
}

#[tokio::test]
async fn test_resume_function_call_failure_takes_error_edge() {
    // With an error_edge configured, a failed function routes to the
    // handler step instead of failing the flow.
    let flow_def = json!({
        "name": "fn-fail-edge-flow",
        "version": 1,
        "nodes": [
            { "id": "start", "step_type": "start", "properties": {}, "next_node": "fn-step" },
            {
                "id": "fn-step",
                "step_type": "function_step",
                "properties": {
                    "function_ref": "/lib/fails",
                    "max_retries": 0,
                    "error_edge": "error-handler"
                },
                "next_node": "end"
            },
            { "id": "error-handler", "step_type": "start", "properties": {}, "next_node": "end" },
            { "id": "end", "step_type": "end", "properties": {} }
        ]
    });

    let mut inst = waiting_instance("fn-step", simple_wait(WaitType::FunctionCall));
    inst.flow_definition_snapshot = flow_def;
    let data = json!({"success": false, "error": "boom"});
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", data, &cb).await;

    assert!(
        result.is_ok(),
        "error edge path should succeed, got {:?}",
        result
    );
    let updated = cb.instance.lock().unwrap();
    // Routed through error-handler -> end
    assert_eq!(updated.status, FlowStatus::Completed);
    let error_var = updated.variables.as_object().unwrap().get("error");
    assert!(error_var.is_some(), "$.error context must be populated");
}

// ---------------------------------------------------------------------------
// Tests: timeout enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resume_timed_out_transitions_to_failed() {
    let inst = waiting_instance(
        "human-task",
        Some(WaitInfo {
            subscription_id: "sub-timeout".to_string(),
            wait_type: WaitType::HumanTask,
            target_path: None,
            expected_event: None,
            timeout_at: Some(Utc::now() - chrono::Duration::seconds(60)),
        }),
    );

    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", json!({}), &cb).await;

    assert!(result.is_err());
    match result.unwrap_err() {
        FlowError::TimeoutExceeded { .. } => {}
        other => panic!("Expected TimeoutExceeded, got {:?}", other),
    }

    assert!(cb.was_save_called());
    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Failed);
    assert!(updated.error.as_ref().unwrap().contains("timed out"));
    assert!(updated.wait_info.is_none());
    assert!(updated.completed_at.is_some());
}

#[tokio::test]
async fn test_resume_not_timed_out_proceeds_normally() {
    let inst = waiting_instance(
        "human-task",
        Some(WaitInfo {
            subscription_id: "sub-future".to_string(),
            wait_type: WaitType::HumanTask,
            target_path: None,
            expected_event: None,
            timeout_at: Some(Utc::now() + chrono::Duration::hours(1)),
        }),
    );
    let response = json!({"approved": true});
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", response.clone(), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Running);
    assert!(updated.wait_info.is_none());
    assert_eq!(
        updated
            .variables
            .as_object()
            .unwrap()
            .get("__human_response"),
        Some(&response)
    );
}

#[tokio::test]
async fn test_resume_no_timeout_proceeds_normally() {
    let inst = waiting_instance("human-task", simple_wait(WaitType::HumanTask));
    let response = json!({"approved": true});
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", response.clone(), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Running);
    assert_eq!(
        updated
            .variables
            .as_object()
            .unwrap()
            .get("__human_response"),
        Some(&response)
    );
}

// ---------------------------------------------------------------------------
// Tests: check_flow_timeout (timeout_check jobs / scheduled wake-ups)
// ---------------------------------------------------------------------------

fn wait_with_timeout(wait_type: WaitType, timeout_at: chrono::DateTime<Utc>) -> Option<WaitInfo> {
    Some(WaitInfo {
        subscription_id: "sub-test".to_string(),
        wait_type,
        target_path: None,
        expected_event: None,
        timeout_at: Some(timeout_at),
    })
}

/// Regression test for the original bug: a timeout check firing while the
/// wait is NOT yet due must not resume the flow (it used to resume it
/// immediately with null data).
#[tokio::test]
async fn test_timeout_check_not_due_is_noop() {
    let future = Utc::now() + chrono::Duration::hours(1);
    let inst = waiting_instance("human-task", wait_with_timeout(WaitType::HumanTask, future));
    let cb = MockCallbacks::new(inst);

    let result = check_flow_timeout("test-instance", &cb).await;
    assert!(result.is_ok());

    let updated = cb.instance.lock().unwrap();
    assert_eq!(
        updated.status,
        FlowStatus::Waiting,
        "flow must stay waiting"
    );
    assert!(updated.wait_info.is_some(), "wait_info must be preserved");
    assert!(!cb.was_save_called(), "no state change should be persisted");
}

/// A scheduled (delay) wait whose wake-up time has NOT arrived is left alone.
#[tokio::test]
async fn test_timeout_check_scheduled_not_due_is_noop() {
    let future = Utc::now() + chrono::Duration::minutes(30);
    let inst = waiting_instance("start", wait_with_timeout(WaitType::Scheduled, future));
    let cb = MockCallbacks::new(inst);

    assert!(check_flow_timeout("test-instance", &cb).await.is_ok());

    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Waiting);
    assert!(!cb.was_save_called());
}

/// A scheduled wait whose wake-up time arrived resumes normally (and must
/// NOT be treated as a timeout failure).
#[tokio::test]
async fn test_timeout_check_scheduled_due_resumes() {
    let past = Utc::now() - chrono::Duration::seconds(5);
    let inst = waiting_instance("start", wait_with_timeout(WaitType::Scheduled, past));
    let cb = MockCallbacks::new(inst);

    let result = check_flow_timeout("test-instance", &cb).await;
    assert!(
        result.is_ok(),
        "due scheduled wait must resume, got {:?}",
        result
    );

    let updated = cb.instance.lock().unwrap();
    // Test flow def is start -> end, so the resumed flow runs to completion
    assert_eq!(updated.status, FlowStatus::Completed);
}

/// A retry wait whose backoff elapsed resumes and keeps its retry budget.
#[tokio::test]
async fn test_timeout_check_retry_due_resumes() {
    let past = Utc::now() - chrono::Duration::seconds(5);
    let mut inst = waiting_instance("start", wait_with_timeout(WaitType::Retry, past));
    inst.retry_count = 2;
    let cb = MockCallbacks::new(inst);

    assert!(check_flow_timeout("test-instance", &cb).await.is_ok());

    let updated = cb.instance.lock().unwrap();
    assert_ne!(updated.status, FlowStatus::Waiting);
}

/// An expired human-task wait with no timeout_edge fails the flow.
#[tokio::test]
async fn test_timeout_check_expired_human_task_fails() {
    let past = Utc::now() - chrono::Duration::minutes(5);
    let inst = waiting_instance("human-task", wait_with_timeout(WaitType::HumanTask, past));
    let cb = MockCallbacks::new(inst);

    let result = check_flow_timeout("test-instance", &cb).await;
    assert!(result.is_err(), "expired wait should report timeout");

    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Failed);
    assert!(updated.error.as_deref().unwrap_or("").contains("timed out"));
}

/// An expired wait on a step with a timeout_edge routes to the handler node
/// instead of failing the flow.
#[tokio::test]
async fn test_timeout_check_expired_follows_timeout_edge() {
    let flow_def = json!({
        "name": "timeout-edge-flow",
        "version": 1,
        "nodes": [
            {
                "id": "wait-step",
                "step_type": "human_task",
                "properties": { "timeout_edge": "timeout-handler" },
                "next_node": "end"
            },
            { "id": "timeout-handler", "step_type": "start", "properties": {}, "next_node": "end" },
            { "id": "end", "step_type": "end", "properties": {} }
        ]
    });

    let past = Utc::now() - chrono::Duration::minutes(5);
    let mut inst = waiting_instance("wait-step", wait_with_timeout(WaitType::HumanTask, past));
    inst.flow_definition_snapshot = flow_def;
    let cb = MockCallbacks::new(inst);

    let result = check_flow_timeout("test-instance", &cb).await;
    assert!(
        result.is_ok(),
        "timeout_edge path should succeed, got {:?}",
        result
    );

    let updated = cb.instance.lock().unwrap();
    // Routed through timeout-handler -> end
    assert_eq!(updated.status, FlowStatus::Completed);
    let error_var = updated.variables.as_object().unwrap().get("error");
    assert!(
        error_var.is_some(),
        "error context must be set for the handler"
    );
    assert_eq!(
        error_var.unwrap().get("error_type"),
        Some(&json!("timeout"))
    );
}

/// A non-waiting flow is never touched by a timeout check.
#[tokio::test]
async fn test_timeout_check_non_waiting_is_noop() {
    let mut inst = waiting_instance("end", None);
    inst.status = FlowStatus::Completed;
    let cb = MockCallbacks::new(inst);

    assert!(check_flow_timeout("test-instance", &cb).await.is_ok());
    assert!(!cb.was_save_called());
}

/// A real resume arriving for a Scheduled wait past its wake-up time must
/// resume normally, not fail as "timed out" (Scheduled timeout_at is a
/// wake-up time, not a deadline).
#[tokio::test]
async fn test_resume_scheduled_past_due_does_not_fail() {
    let past = Utc::now() - chrono::Duration::minutes(5);
    let inst = waiting_instance("start", wait_with_timeout(WaitType::Scheduled, past));
    let cb = MockCallbacks::new(inst);

    let result = resume_flow("test-instance", json!(null), &cb).await;
    assert!(result.is_ok());

    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Completed);
}

#[tokio::test]
async fn test_resume_running_with_data_requests_redelivery() {
    // A fast function can finish before the flow persists Waiting; its
    // result delivery must be retried, not swallowed as "already running".
    let mut inst = waiting_instance("step-1", None);
    inst.status = FlowStatus::Running;
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", json!({"success": true, "result": 42}), &cb).await;
    assert!(matches!(result.unwrap_err(), FlowError::VersionConflict));
}

#[tokio::test]
async fn test_resume_pending_with_data_requests_redelivery() {
    let mut inst = waiting_instance("step-1", None);
    inst.status = FlowStatus::Pending;
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", json!({"success": true}), &cb).await;
    assert!(matches!(result.unwrap_err(), FlowError::VersionConflict));
}
