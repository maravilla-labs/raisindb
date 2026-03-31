// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::FlowExecutionHandler;
use crate::types::{FlowCallbacks, FlowError, FlowInstance, FlowResult, FlowStatus};

/// Mock implementation of FlowCallbacks for testing
struct MockCallbacks {
    instance: FlowInstance,
}

#[async_trait]
impl FlowCallbacks for MockCallbacks {
    async fn load_instance(&self, _path: &str) -> FlowResult<FlowInstance> {
        Ok(self.instance.clone())
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
        Err(FlowError::Other(
            "create_node not implemented in mock".to_string(),
        ))
    }

    async fn update_node(&self, _path: &str, _properties: Value) -> FlowResult<Value> {
        Err(FlowError::Other(
            "update_node not implemented in mock".to_string(),
        ))
    }

    async fn get_node(&self, _path: &str) -> FlowResult<Option<Value>> {
        Ok(None)
    }

    async fn queue_job(&self, _job_type: &str, _payload: Value) -> FlowResult<String> {
        Ok("test-job-id".to_string())
    }

    async fn call_ai(
        &self,
        _agent_workspace: &str,
        _agent_ref: &str,
        _messages: Vec<Value>,
        _response_format: Option<Value>,
    ) -> FlowResult<Value> {
        Err(FlowError::Other(
            "call_ai not implemented in mock".to_string(),
        ))
    }

    async fn execute_function(&self, _function_ref: &str, _input: Value) -> FlowResult<Value> {
        Err(FlowError::Other(
            "execute_function not implemented in mock".to_string(),
        ))
    }
}

/// Create a valid flow definition for testing
fn create_test_flow_definition() -> serde_json::Value {
    serde_json::json!({
        "name": "test-flow",
        "version": 1,
        "nodes": [
            {
                "id": "start",
                "step_type": "start",
                "properties": {},
                "children": [],
                "next_node": "end"
            },
            {
                "id": "end",
                "step_type": "end",
                "properties": {},
                "children": [],
                "next_node": null
            }
        ],
        "edges": [
            {"from": "start", "to": "end"}
        ]
    })
}

#[tokio::test]
async fn test_load_running_instance() {
    let instance = FlowInstance::new(
        "/flows/test-flow".to_string(),
        1,
        create_test_flow_definition(),
        serde_json::json!({"input": "test"}),
        "start".to_string(),
    );

    let callbacks = Arc::new(MockCallbacks {
        instance: instance.clone(),
    });
    let handler = FlowExecutionHandler::new(callbacks);

    let result = handler
        .handle(&instance.id, "tenant1", "repo1", "main")
        .await;

    // Should succeed - starts at "start" node and continues to "end"
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_cannot_resume_terminated_flow() {
    let mut instance = FlowInstance::new(
        "/flows/test-flow".to_string(),
        1,
        create_test_flow_definition(),
        serde_json::json!({"input": "test"}),
        "start".to_string(),
    );
    instance.status = FlowStatus::Completed;

    let callbacks = Arc::new(MockCallbacks {
        instance: instance.clone(),
    });
    let handler = FlowExecutionHandler::new(callbacks);

    let result = handler
        .handle(&instance.id, "tenant1", "repo1", "main")
        .await;

    // Should return Ok(None) for terminated instances
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), None);
}
