// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

use super::handler::WAIT_REASON_PARALLEL;
use super::ParallelHandler;
use crate::types::{ChildFlowStatus, FlowContext, FlowError, FlowNode, StepResult, StepType};
use serde_json::Value;
use std::collections::HashMap;

fn create_test_context() -> FlowContext {
    FlowContext::new(
        "test-instance".to_string(),
        serde_json::json!({
            "data": "test-value"
        }),
    )
}

fn create_parallel_node() -> FlowNode {
    let mut properties = HashMap::new();

    let branches = vec![
        serde_json::json!({
            "id": "branch-1",
            "flow_definition": {
                "nodes": []
            }
        }),
        serde_json::json!({
            "id": "branch-2",
            "flow_definition": {
                "nodes": []
            }
        }),
    ];

    properties.insert("branches".to_string(), Value::Array(branches));
    properties.insert(
        "merge_strategy".to_string(),
        Value::String("merge_all".to_string()),
    );

    FlowNode {
        id: "parallel-1".to_string(),
        step_type: StepType::Parallel,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    }
}

// Tests for pure functions that don't require callbacks

#[test]
fn test_parallel_merge_all_strategy() {
    let handler = ParallelHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert(
        "merge_strategy".to_string(),
        Value::String("merge_all".to_string()),
    );

    let node = FlowNode {
        id: "parallel-1".to_string(),
        step_type: StepType::Parallel,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let child_statuses = vec![
        ChildFlowStatus {
            branch_id: "branch-1".to_string(),
            instance_id: "child-1".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"result": "success"})),
            error: None,
        },
        ChildFlowStatus {
            branch_id: "branch-2".to_string(),
            instance_id: "child-2".to_string(),
            status: "failed".to_string(),
            output: None,
            error: Some("Test error".to_string()),
        },
    ];

    let result = handler.merge_all_outputs(&node, &mut context, &child_statuses);
    assert!(result.is_ok());

    match result.unwrap() {
        StepResult::Continue { output, .. } => {
            let output_obj = output.as_object().unwrap();
            assert!(output_obj.contains_key("branch_0"));
            assert!(output_obj.contains_key("branch_1"));
        }
        _ => panic!("Expected Continue result"),
    }
}

#[test]
fn test_parallel_first_success_strategy() {
    let handler = ParallelHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert(
        "merge_strategy".to_string(),
        Value::String("first_success".to_string()),
    );

    let node = FlowNode {
        id: "parallel-1".to_string(),
        step_type: StepType::Parallel,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let child_statuses = vec![
        ChildFlowStatus {
            branch_id: "branch-1".to_string(),
            instance_id: "child-1".to_string(),
            status: "failed".to_string(),
            output: None,
            error: Some("Error 1".to_string()),
        },
        ChildFlowStatus {
            branch_id: "branch-2".to_string(),
            instance_id: "child-2".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"result": "success"})),
            error: None,
        },
    ];

    let result = handler.first_success_output(&node, &mut context, &child_statuses);
    assert!(result.is_ok());

    match result.unwrap() {
        StepResult::Continue { output, .. } => {
            assert_eq!(output["result"], "success");
        }
        _ => panic!("Expected Continue result"),
    }
}

#[test]
fn test_parallel_all_success_strategy_fails() {
    let handler = ParallelHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert(
        "merge_strategy".to_string(),
        Value::String("all_success".to_string()),
    );

    let node = FlowNode {
        id: "parallel-1".to_string(),
        step_type: StepType::Parallel,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let child_statuses = vec![
        ChildFlowStatus {
            branch_id: "branch-1".to_string(),
            instance_id: "child-1".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"result": "success"})),
            error: None,
        },
        ChildFlowStatus {
            branch_id: "branch-2".to_string(),
            instance_id: "child-2".to_string(),
            status: "failed".to_string(),
            output: None,
            error: Some("Test error".to_string()),
        },
    ];

    let result = handler.all_success_output(&node, &mut context, &child_statuses);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        FlowError::ParallelExecutionError(_)
    ));
}

#[test]
fn test_parallel_all_success_when_all_succeed() {
    let handler = ParallelHandler::new();
    let mut context = create_test_context();

    let mut properties = HashMap::new();
    properties.insert(
        "merge_strategy".to_string(),
        Value::String("all_success".to_string()),
    );

    let node = FlowNode {
        id: "parallel-1".to_string(),
        step_type: StepType::Parallel,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let child_statuses = vec![
        ChildFlowStatus {
            branch_id: "branch-1".to_string(),
            instance_id: "child-1".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"result": "success1"})),
            error: None,
        },
        ChildFlowStatus {
            branch_id: "branch-2".to_string(),
            instance_id: "child-2".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"result": "success2"})),
            error: None,
        },
    ];

    let result = handler.all_success_output(&node, &mut context, &child_statuses);
    assert!(result.is_ok());

    match result.unwrap() {
        StepResult::Continue { output, .. } => {
            let output_obj = output.as_object().unwrap();
            assert!(output_obj.contains_key("branch_0"));
            assert!(output_obj.contains_key("branch_1"));
        }
        _ => panic!("Expected Continue result"),
    }
}

#[test]
fn test_parallel_first_success_all_failed() {
    let handler = ParallelHandler::new();
    let mut context = create_test_context();

    let node = FlowNode {
        id: "parallel-1".to_string(),
        step_type: StepType::Parallel,
        properties: HashMap::new(),
        children: vec![],
        next_node: Some("next-step".to_string()),
    };

    let child_statuses = vec![
        ChildFlowStatus {
            branch_id: "branch-1".to_string(),
            instance_id: "child-1".to_string(),
            status: "failed".to_string(),
            output: None,
            error: Some("Error 1".to_string()),
        },
        ChildFlowStatus {
            branch_id: "branch-2".to_string(),
            instance_id: "child-2".to_string(),
            status: "failed".to_string(),
            output: None,
            error: Some("Error 2".to_string()),
        },
    ];

    let result = handler.first_success_output(&node, &mut context, &child_statuses);
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        FlowError::AllChildFlowsFailed
    ));
}

#[test]
fn test_join_with_statuses_not_all_completed() {
    let handler = ParallelHandler::new();
    let mut context = create_test_context();

    let node = create_parallel_node();

    let child_statuses = vec![
        ChildFlowStatus {
            branch_id: "branch-1".to_string(),
            instance_id: "child-1".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"result": "success"})),
            error: None,
        },
        ChildFlowStatus {
            branch_id: "branch-2".to_string(),
            instance_id: "child-2".to_string(),
            status: "running".to_string(),
            output: None,
            error: None,
        },
    ];

    let result = handler.join_with_statuses(
        &node,
        &mut context,
        vec!["child-1".to_string(), "child-2".to_string()],
        child_statuses,
    );
    assert!(result.is_ok());

    match result.unwrap() {
        StepResult::Wait { reason, metadata } => {
            assert_eq!(reason, WAIT_REASON_PARALLEL);
            assert_eq!(metadata["pending_count"], 1);
        }
        _ => panic!("Expected Wait result for incomplete branches"),
    }
}
