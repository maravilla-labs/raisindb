// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

use super::handler::WAIT_REASON_HUMAN_TASK;
use super::HumanTaskHandler;
use crate::handlers::StepHandler;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowInstance, FlowNode, FlowResult, StepResult, StepType,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Mock implementation of FlowCallbacks for testing
struct MockCallbacks {
    created_nodes: Arc<Mutex<Vec<(String, String, Value)>>>,
    should_fail: bool,
}

impl MockCallbacks {
    fn new() -> Self {
        Self {
            created_nodes: Arc::new(Mutex::new(Vec::new())),
            should_fail: false,
        }
    }

    fn with_failure() -> Self {
        Self {
            created_nodes: Arc::new(Mutex::new(Vec::new())),
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
        node_type: &str,
        path: &str,
        properties: Value,
    ) -> FlowResult<Value> {
        if self.should_fail {
            return Err(FlowError::FunctionExecution(
                "Failed to create node".to_string(),
            ));
        }
        self.created_nodes.lock().await.push((
            node_type.to_string(),
            path.to_string(),
            properties.clone(),
        ));
        Ok(properties)
    }

    async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
        Ok(Value::Null)
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

fn create_approval_task_node() -> FlowNode {
    let mut properties = HashMap::new();
    properties.insert(
        "task_type".to_string(),
        Value::String("approval".to_string()),
    );
    properties.insert(
        "title".to_string(),
        Value::String("Approve Budget Request".to_string()),
    );
    properties.insert(
        "description".to_string(),
        Value::String("Please review and approve this budget request".to_string()),
    );
    properties.insert(
        "assignee".to_string(),
        Value::String("/users/manager".to_string()),
    );

    let options = vec![
        serde_json::json!({
            "value": "approve",
            "label": "Approve",
            "style": "success"
        }),
        serde_json::json!({
            "value": "reject",
            "label": "Reject",
            "style": "danger"
        }),
    ];
    properties.insert("options".to_string(), Value::Array(options));
    properties.insert("priority".to_string(), Value::Number(4.into()));
    properties.insert("due_in_seconds".to_string(), Value::Number(86400.into()));

    FlowNode {
        id: "approval-1".to_string(),
        step_type: StepType::HumanTask,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    }
}

#[tokio::test]
async fn test_human_task_creates_inbox_task() {
    let handler = HumanTaskHandler::new();
    let node = create_approval_task_node();
    let mut context = create_test_context();

    let callbacks = MockCallbacks::new();
    let created_nodes = callbacks.created_nodes.clone();

    let result = handler.execute(&node, &mut context, &callbacks).await;
    assert!(result.is_ok());

    match result.unwrap() {
        StepResult::Wait { reason, metadata } => {
            assert_eq!(reason, WAIT_REASON_HUMAN_TASK);
            assert_eq!(metadata["task_type"], "approval");
            assert_eq!(metadata["step_id"], "approval-1");
            assert_eq!(metadata["assignee"], "/users/manager");
            assert!(metadata["task_path"]
                .as_str()
                .unwrap()
                .starts_with("/users/manager/inbox/task-approval-1-"));
        }
        _ => panic!("Expected Wait result"),
    }

    // Verify node was created
    let nodes = created_nodes.lock().await;
    assert_eq!(nodes.len(), 1);
    let (node_type, path, properties) = &nodes[0];
    assert_eq!(node_type, "inbox_task");
    assert!(path.starts_with("/users/manager/inbox/task-approval-1-"));
    assert_eq!(properties["task_type"], "approval");
    assert_eq!(properties["title"], "Approve Budget Request");
    assert_eq!(properties["priority"], 4);
}

#[tokio::test]
async fn test_human_task_missing_task_type() {
    let handler = HumanTaskHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert("title".to_string(), Value::String("Test Task".to_string()));
    properties.insert(
        "assignee".to_string(),
        Value::String("/users/test".to_string()),
    );

    let node = FlowNode {
        id: "task-1".to_string(),
        step_type: StepType::HumanTask,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let callbacks = MockCallbacks::new();

    let result = handler.execute(&node, &mut context, &callbacks).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), FlowError::MissingProperty(_)));
}

#[tokio::test]
async fn test_human_task_missing_title() {
    let handler = HumanTaskHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert(
        "task_type".to_string(),
        Value::String("approval".to_string()),
    );
    properties.insert(
        "assignee".to_string(),
        Value::String("/users/test".to_string()),
    );

    let node = FlowNode {
        id: "task-1".to_string(),
        step_type: StepType::HumanTask,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let callbacks = MockCallbacks::new();

    let result = handler.execute(&node, &mut context, &callbacks).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), FlowError::MissingProperty(_)));
}

#[tokio::test]
async fn test_human_task_missing_assignee() {
    let handler = HumanTaskHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert(
        "task_type".to_string(),
        Value::String("approval".to_string()),
    );
    properties.insert("title".to_string(), Value::String("Test Task".to_string()));

    let node = FlowNode {
        id: "task-1".to_string(),
        step_type: StepType::HumanTask,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let callbacks = MockCallbacks::new();

    let result = handler.execute(&node, &mut context, &callbacks).await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), FlowError::MissingProperty(_)));
}

#[tokio::test]
async fn test_human_task_invalid_task_type() {
    let handler = HumanTaskHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert(
        "task_type".to_string(),
        Value::String("invalid".to_string()),
    );
    properties.insert("title".to_string(), Value::String("Test Task".to_string()));
    properties.insert(
        "assignee".to_string(),
        Value::String("/users/test".to_string()),
    );

    let node = FlowNode {
        id: "task-1".to_string(),
        step_type: StepType::HumanTask,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let callbacks = MockCallbacks::new();

    let result = handler.execute(&node, &mut context, &callbacks).await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        FlowError::InvalidNodeConfiguration(_)
    ));
}

#[tokio::test]
async fn test_human_task_input_type() {
    let handler = HumanTaskHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert("task_type".to_string(), Value::String("input".to_string()));
    properties.insert(
        "title".to_string(),
        Value::String("Enter Details".to_string()),
    );
    properties.insert(
        "assignee".to_string(),
        Value::String("/users/user1".to_string()),
    );
    properties.insert(
        "input_schema".to_string(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "email": { "type": "string" }
            }
        }),
    );

    let node = FlowNode {
        id: "input-1".to_string(),
        step_type: StepType::HumanTask,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let callbacks = MockCallbacks::new();
    let created_nodes = callbacks.created_nodes.clone();

    let result = handler.execute(&node, &mut context, &callbacks).await;
    assert!(result.is_ok());

    match result.unwrap() {
        StepResult::Wait { reason, metadata } => {
            assert_eq!(reason, WAIT_REASON_HUMAN_TASK);
            assert_eq!(metadata["task_type"], "input");
        }
        _ => panic!("Expected Wait result"),
    }

    // Verify input_schema was included
    let nodes = created_nodes.lock().await;
    assert_eq!(nodes.len(), 1);
    let (_, _, properties) = &nodes[0];
    assert!(properties.get("input_schema").is_some());
}

#[tokio::test]
async fn test_human_task_creation_failure() {
    let handler = HumanTaskHandler::new();
    let node = create_approval_task_node();
    let mut context = create_test_context();

    let callbacks = MockCallbacks::with_failure();

    let result = handler.execute(&node, &mut context, &callbacks).await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        FlowError::FunctionExecution(_)
    ));
}
