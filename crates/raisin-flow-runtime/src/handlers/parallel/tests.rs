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

// ---------------------------------------------------------------------------
// Dynamic fan-out (`for_each`)
// ---------------------------------------------------------------------------

/// Records the child-flow jobs a fork queued, so a test can assert on the
/// per-item payloads rather than on a child flow actually running.
struct RecordingCallbacks {
    queued: std::sync::Arc<tokio::sync::Mutex<Vec<(String, Value)>>>,
}

impl RecordingCallbacks {
    fn new() -> Self {
        Self {
            queued: std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }
}

#[async_trait::async_trait]
impl crate::types::FlowCallbacks for RecordingCallbacks {
    async fn load_instance(
        &self,
        _path: &str,
    ) -> crate::types::FlowResult<crate::types::FlowInstance> {
        unimplemented!()
    }

    async fn save_instance(
        &self,
        _instance: &crate::types::FlowInstance,
    ) -> crate::types::FlowResult<()> {
        Ok(())
    }

    async fn save_instance_with_version(
        &self,
        _instance: &crate::types::FlowInstance,
        _expected_version: i32,
    ) -> crate::types::FlowResult<()> {
        Ok(())
    }

    async fn create_node(
        &self,
        _node_type: &str,
        _path: &str,
        properties: Value,
    ) -> crate::types::FlowResult<Value> {
        Ok(properties)
    }

    async fn update_node(
        &self,
        _path: &str,
        _properties: Value,
    ) -> crate::types::FlowResult<Value> {
        Ok(Value::Null)
    }

    async fn get_node(&self, _path: &str) -> crate::types::FlowResult<Option<Value>> {
        Ok(None)
    }

    async fn queue_job(&self, job_type: &str, payload: Value) -> crate::types::FlowResult<String> {
        let mut queued = self.queued.lock().await;
        queued.push((job_type.to_string(), payload));
        Ok(format!("child-{}", queued.len()))
    }

    async fn call_ai(
        &self,
        _agent_workspace: &str,
        _agent_ref: &str,
        _messages: Vec<Value>,
        _response_format: Option<Value>,
    ) -> crate::types::FlowResult<Value> {
        Ok(Value::Null)
    }

    async fn execute_function(
        &self,
        _function_ref: &str,
        _input: Value,
    ) -> crate::types::FlowResult<Value> {
        Ok(Value::Null)
    }
}

fn create_fan_out_node(branch: Value, extra: Vec<(&str, Value)>) -> FlowNode {
    let mut properties = HashMap::new();
    properties.insert(
        "for_each".to_string(),
        Value::String("${input.items}".to_string()),
    );
    properties.insert("branch".to_string(), branch);
    for (key, value) in extra {
        properties.insert(key.to_string(), value);
    }

    FlowNode {
        id: "fan-out".to_string(),
        step_type: StepType::Parallel,
        properties,
        children: vec![],
        next_node: Some("next-step".to_string()),
    }
}

fn fan_out_context(items: Value) -> FlowContext {
    FlowContext::new(
        "test-instance".to_string(),
        serde_json::json!({ "items": items }),
    )
}

/// One child flow per runtime item, each referencing a DEPLOYED flow by
/// path, with `item` / `index` bound inside the branch template.
#[tokio::test]
async fn test_fan_out_forks_one_child_per_item() {
    let handler = ParallelHandler::new();
    let context = fan_out_context(serde_json::json!([
        {"id": "a", "owner": "/users/alice"},
        {"id": "b", "owner": "/users/bob"}
    ]));

    let node = create_fan_out_node(
        serde_json::json!({
            "id": "item-${index}",
            "flow_path": "/flows/approve-item",
            "input_mapping": {
                "item_id": "${item.id}",
                "assignee": "${item.owner}",
                "position": "${index}"
            }
        }),
        vec![],
    );

    let callbacks = RecordingCallbacks::new();
    let queued = callbacks.queued.clone();

    let result = handler
        .fork_branches(&node, &context, &callbacks)
        .await
        .expect("fan-out forks");

    match result {
        StepResult::Wait { reason, metadata } => {
            assert_eq!(reason, WAIT_REASON_PARALLEL);
            assert_eq!(metadata["total_branches"], 2);
            assert_eq!(metadata["branch_ids"][0], "item-0");
            assert_eq!(metadata["branch_ids"][1], "item-1");
            assert_eq!(metadata["child_flow_ids"].as_array().unwrap().len(), 2);
        }
        _ => panic!("Expected Wait result"),
    }

    let jobs = queued.lock().await;
    assert_eq!(jobs.len(), 2);

    // A path-referenced branch goes through flow_execution, and carries the
    // branch id so the join can correlate the result with its item.
    assert_eq!(jobs[0].0, "flow_execution");
    assert_eq!(jobs[0].1["flow_path"], "/flows/approve-item");
    assert_eq!(jobs[0].1["branch_id"], "item-0");
    assert_eq!(jobs[0].1["parent_step_id"], "fan-out");
    assert_eq!(jobs[0].1["input"]["item_id"], "a");
    assert_eq!(jobs[0].1["input"]["assignee"], "/users/alice");
    // A whole-value expression keeps its resolved JSON type
    assert_eq!(jobs[0].1["input"]["position"], 0);

    assert_eq!(jobs[1].1["branch_id"], "item-1");
    assert_eq!(jobs[1].1["input"]["item_id"], "b");
    assert_eq!(jobs[1].1["input"]["assignee"], "/users/bob");
}

/// With no `input_mapping`, the item itself is the child's input, and with
/// no `id` the branch id falls back to `{step_id}-{index}`.
#[tokio::test]
async fn test_fan_out_defaults_input_to_item() {
    let handler = ParallelHandler::new();
    let context = fan_out_context(serde_json::json!([{"row": 1}, {"row": 2}]));

    let node = create_fan_out_node(
        serde_json::json!({ "flow_definition": { "nodes": [] } }),
        vec![],
    );

    let callbacks = RecordingCallbacks::new();
    let queued = callbacks.queued.clone();

    handler
        .fork_branches(&node, &context, &callbacks)
        .await
        .expect("fan-out forks");

    let jobs = queued.lock().await;
    // An inline branch goes through create_child_flow instead.
    assert_eq!(jobs[0].0, "create_child_flow");
    assert_eq!(jobs[0].1["branch_id"], "fan-out-0");
    assert_eq!(jobs[0].1["parent_step_id"], "fan-out");
    assert_eq!(jobs[0].1["input"]["row"], 1);
    assert_eq!(jobs[1].1["branch_id"], "fan-out-1");
    assert_eq!(jobs[1].1["input"]["row"], 2);
}

/// An empty collection is a no-op fan-out, not a wait that never resolves.
#[tokio::test]
async fn test_fan_out_empty_collection_continues() {
    let handler = ParallelHandler::new();
    let context = fan_out_context(serde_json::json!([]));
    let node = create_fan_out_node(
        serde_json::json!({ "flow_path": "/flows/approve-item" }),
        vec![],
    );

    let callbacks = RecordingCallbacks::new();
    let result = handler
        .fork_branches(&node, &context, &callbacks)
        .await
        .expect("empty fan-out is a no-op");

    match result {
        StepResult::Continue { next_node_id, .. } => assert_eq!(next_node_id, "next-step"),
        _ => panic!("Expected Continue result"),
    }
    assert!(callbacks.queued.lock().await.is_empty());
}

/// Fan-out width is driven by runtime data, so it is capped.
#[tokio::test]
async fn test_fan_out_respects_max_branches() {
    let handler = ParallelHandler::new();
    let context = fan_out_context(serde_json::json!([1, 2, 3, 4, 5]));
    let node = create_fan_out_node(
        serde_json::json!({ "flow_path": "/flows/approve-item" }),
        vec![("max_branches", Value::from(2))],
    );

    let callbacks = RecordingCallbacks::new();
    handler
        .fork_branches(&node, &context, &callbacks)
        .await
        .expect("fan-out forks");

    assert_eq!(callbacks.queued.lock().await.len(), 2);
}

/// A `for_each` step without a `branch` template is an authoring error, not
/// a silent zero-branch fan-out.
#[tokio::test]
async fn test_fan_out_requires_branch_template() {
    let handler = ParallelHandler::new();
    let context = fan_out_context(serde_json::json!([1]));

    let mut properties = HashMap::new();
    properties.insert(
        "for_each".to_string(),
        Value::String("${input.items}".to_string()),
    );
    let node = FlowNode {
        id: "fan-out".to_string(),
        step_type: StepType::Parallel,
        properties,
        children: vec![],
        next_node: None,
    };

    let callbacks = RecordingCallbacks::new();
    let err = handler
        .fork_branches(&node, &context, &callbacks)
        .await
        .expect_err("a for_each without a branch template is an error");
    assert!(matches!(err, FlowError::MissingProperty(_)));
}

/// The join exposes results positionally AND as an ordered array tagged with
/// branch ids - for a fan-out the branch id is the only handle on WHICH item
/// produced what.
#[test]
fn test_merge_all_reports_branch_ids_in_order() {
    let handler = ParallelHandler::new();
    let mut context = create_test_context();
    let node = create_parallel_node();

    let child_statuses = vec![
        ChildFlowStatus {
            branch_id: "item-0".to_string(),
            instance_id: "child-1".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"decision": "approve"})),
            error: None,
        },
        ChildFlowStatus {
            branch_id: "item-1".to_string(),
            instance_id: "child-2".to_string(),
            status: "completed".to_string(),
            output: Some(serde_json::json!({"decision": "reject"})),
            error: None,
        },
    ];

    match handler
        .merge_all_outputs(&node, &mut context, &child_statuses)
        .expect("merge succeeds")
    {
        StepResult::Continue { output, .. } => {
            // Positional keys stay for existing consumers
            assert_eq!(output["branch_0"]["output"]["decision"], "approve");
            assert_eq!(output["branch_1"]["output"]["decision"], "reject");
            // The fan-out is iterable in order, each entry naming its branch
            let branches = output["branches"].as_array().unwrap();
            assert_eq!(branches.len(), 2);
            assert_eq!(branches[0]["branch_id"], "item-0");
            assert_eq!(branches[0]["output"]["decision"], "approve");
            assert_eq!(branches[1]["branch_id"], "item-1");
            assert_eq!(branches[1]["instance_id"], "child-2");
            // Author-chosen ids are NOT spliced into the top-level namespace
            assert!(output.get("item-0").is_none());
        }
        _ => panic!("Expected Continue result"),
    }
}
