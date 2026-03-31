// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use chrono::Utc;
use serde_json::{json, Value};

use super::*;
use crate::types::{FlowStatus, TriggerEventType, TriggerInfo};
use crate::FlowError;

#[test]
fn test_create_flow_instance_from_node_event() {
    let flow_definition = json!({
        "nodes": [
            {
                "id": "start",
                "step_type": "start",
                "properties": {}
            },
            {
                "id": "step-1",
                "step_type": "function_step",
                "properties": {}
            }
        ],
        "edges": [
            {
                "from": "start",
                "to": "step-1"
            }
        ],
        "metadata": {}
    });

    let trigger_event = FlowTriggerEvent::NodeEvent {
        event_type: "Created".to_string(),
        node_id: "node123".to_string(),
        node_type: "Article".to_string(),
        node_path: "/articles/test".to_string(),
        properties: json!({"title": "Test Article"}),
        timestamp: Utc::now(),
    };

    let input = json!({"title": "Test Article"});

    let result = create_flow_instance_from_trigger(
        "/flows/test-flow".to_string(),
        1,
        flow_definition.clone(),
        &trigger_event,
        input.clone(),
    );

    assert!(result.is_ok());
    let instance = result.unwrap();
    assert_eq!(instance.flow_ref, "/flows/test-flow");
    assert_eq!(instance.flow_version, 1);
    assert_eq!(instance.current_node_id, "start");
    assert_eq!(instance.input, input);
    assert_eq!(instance.status, FlowStatus::Pending);
}

#[test]
fn test_create_flow_instance_missing_start_node() {
    let flow_definition = json!({
        "nodes": [
            {
                "id": "step-1",
                "step_type": "function_step",
                "properties": {}
            }
        ],
        "edges": [],
        "metadata": {}
    });

    let trigger_event = FlowTriggerEvent::NodeEvent {
        event_type: "Created".to_string(),
        node_id: "node123".to_string(),
        node_type: "Article".to_string(),
        node_path: "/articles/test".to_string(),
        properties: json!({}),
        timestamp: Utc::now(),
    };

    let result = create_flow_instance_from_trigger(
        "/flows/test-flow".to_string(),
        1,
        flow_definition,
        &trigger_event,
        json!({}),
    );

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        FlowError::InvalidDefinition(_)
    ));
}

#[test]
fn test_build_trigger_info_from_node_event() {
    let event = FlowTriggerEvent::NodeEvent {
        event_type: "Created".to_string(),
        node_id: "node123".to_string(),
        node_type: "Article".to_string(),
        node_path: "/articles/test".to_string(),
        properties: json!({}),
        timestamp: Utc::now(),
    };

    let result = build_trigger_info_from_event(&event);
    assert!(result.is_ok());

    let trigger_info = result.unwrap();
    assert_eq!(trigger_info.event_type, TriggerEventType::Created);
    assert_eq!(trigger_info.node_id, "node123");
    assert_eq!(trigger_info.node_type, "Article");
    assert_eq!(trigger_info.node_path, Some("/articles/test".to_string()));
}

#[test]
fn test_build_trigger_info_from_scheduled_event() {
    let event = FlowTriggerEvent::ScheduledTime {
        schedule_id: "sched123".to_string(),
        scheduled_time: Utc::now(),
        actual_time: Utc::now(),
    };

    let result = build_trigger_info_from_event(&event);
    assert!(result.is_ok());

    let trigger_info = result.unwrap();
    assert_eq!(trigger_info.event_type, TriggerEventType::Scheduled);
    assert_eq!(trigger_info.node_id, "sched123");
    assert_eq!(trigger_info.node_type, "schedule");
}

#[test]
fn test_build_trigger_info_from_tool_result() {
    let event = FlowTriggerEvent::ToolResult {
        tool_call_id: "call_123".to_string(),
        tool_name: "search".to_string(),
        result: json!({"results": []}),
        success: true,
        error: None,
        timestamp: Utc::now(),
    };

    let result = build_trigger_info_from_event(&event);
    assert!(result.is_ok());

    let trigger_info = result.unwrap();
    assert_eq!(trigger_info.event_type, TriggerEventType::Resume);
    assert_eq!(trigger_info.node_id, "call_123");
    assert_eq!(trigger_info.node_type, "search");
}

#[test]
fn test_flow_instance_builder() {
    let flow_definition = json!({
        "nodes": [
            {
                "id": "start",
                "step_type": "start",
                "properties": {}
            }
        ],
        "edges": [],
        "metadata": {}
    });

    let trigger_event = FlowTriggerEvent::NodeEvent {
        event_type: "Created".to_string(),
        node_id: "node123".to_string(),
        node_type: "Article".to_string(),
        node_path: "/articles/test".to_string(),
        properties: json!({}),
        timestamp: Utc::now(),
    };

    let result = FlowInstanceBuilder::new(
        "/flows/test-flow".to_string(),
        1,
        flow_definition,
        trigger_event,
        json!({"title": "Test"}),
    )
    .tenant_id("tenant123".to_string())
    .repo_id("repo456".to_string())
    .branch("main".to_string())
    .workspace("default".to_string())
    .build();

    assert!(result.is_ok());
    let instance = result.unwrap();

    // Check that trigger info is stored in variables
    if let Value::Object(ref vars) = instance.variables {
        assert!(vars.contains_key("__trigger_info"));
        let trigger_info_value = vars.get("__trigger_info").unwrap();
        let trigger_info: TriggerInfo = serde_json::from_value(trigger_info_value.clone()).unwrap();
        assert_eq!(trigger_info.tenant_id, "tenant123");
        assert_eq!(trigger_info.repo_id, "repo456");
        assert_eq!(trigger_info.branch, "main");
        assert_eq!(trigger_info.workspace, "default");
    } else {
        panic!("Expected variables to be an object");
    }
}

#[test]
fn test_node_event_id() {
    let event = FlowTriggerEvent::NodeEvent {
        event_type: "Created".to_string(),
        node_id: "node123".to_string(),
        node_type: "Article".to_string(),
        node_path: "/articles/test".to_string(),
        properties: serde_json::json!({}),
        timestamp: Utc::now(),
    };

    let id = event.event_id();
    assert!(id.starts_with("node:node123:Created:"));
}

#[test]
fn test_tool_result_event_id() {
    let event = FlowTriggerEvent::ToolResult {
        tool_call_id: "call_abc123".to_string(),
        tool_name: "search".to_string(),
        result: serde_json::json!({"results": []}),
        success: true,
        error: None,
        timestamp: Utc::now(),
    };

    assert_eq!(event.event_id(), "tool:call_abc123");
}

#[test]
fn test_human_task_event_description() {
    let event = FlowTriggerEvent::HumanTaskCompleted {
        task_id: "task456".to_string(),
        task_type: "approval".to_string(),
        response: serde_json::json!({"approved": true}),
        completed_by: "alice@example.com".to_string(),
        timestamp: Utc::now(),
    };

    let desc = event.description();
    assert!(desc.contains("approval"));
    assert!(desc.contains("alice@example.com"));
}

#[test]
fn test_resume_reason_is_error_retry() {
    let retry_reason = FlowResumeReason::RetryAfterError {
        previous_error: "Network timeout".to_string(),
        retry_attempt: 2,
    };

    assert!(retry_reason.is_error_retry());
    assert_eq!(retry_reason.retry_attempt(), Some(2));

    let other_reason = FlowResumeReason::HumanTaskCompleted {
        task_id: "task123".to_string(),
        response: serde_json::json!({}),
    };

    assert!(!other_reason.is_error_retry());
    assert_eq!(other_reason.retry_attempt(), None);
}

#[test]
fn test_serialization() {
    let event = FlowTriggerEvent::ToolResult {
        tool_call_id: "call_123".to_string(),
        tool_name: "calculator".to_string(),
        result: serde_json::json!({"answer": 42}),
        success: true,
        error: None,
        timestamp: Utc::now(),
    };

    // Should serialize/deserialize correctly
    let json = serde_json::to_string(&event).unwrap();
    let deserialized: FlowTriggerEvent = serde_json::from_str(&json).unwrap();

    assert!(matches!(deserialized, FlowTriggerEvent::ToolResult { .. }));
}
