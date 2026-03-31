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
async fn test_resume_retry_resets_count() {
    let mut inst = waiting_instance("retry-step", simple_wait(WaitType::Retry));
    inst.retry_count = 3;
    let cb = MockCallbacks::new(inst);
    let _ = resume_flow("test-instance", json!({}), &cb).await;

    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.retry_count, 0);
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
async fn test_resume_function_call_failure_transitions_to_failed() {
    let inst = waiting_instance("function-step", simple_wait(WaitType::FunctionCall));
    let data = json!({"success": false, "error": "Function execution timeout"});
    let cb = MockCallbacks::new(inst);
    let result = resume_flow("test-instance", data, &cb).await;

    assert!(result.is_ok());
    assert!(cb.was_save_called());
    let updated = cb.instance.lock().unwrap();
    assert_eq!(updated.status, FlowStatus::Failed);
    assert_eq!(
        updated.error,
        Some("Function execution timeout".to_string())
    );
    assert!(updated.wait_info.is_none());
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
