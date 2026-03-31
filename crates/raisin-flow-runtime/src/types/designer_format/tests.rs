// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

use super::*;
use crate::types::flow_definition::StepType;

#[test]
fn test_parse_designer_step() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "step_1",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Test Step",
                    "disabled": false
                }
            }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    assert_eq!(def.version, 1);
    assert_eq!(def.nodes.len(), 1);

    let runtime = def.to_runtime_format();
    // 1 step + implicit Start + implicit End = 3 nodes
    assert_eq!(runtime.nodes.len(), 3);
    assert_eq!(runtime.nodes[0].step_type, StepType::Start);
    assert_eq!(runtime.nodes[1].id, "step_1");
    assert_eq!(runtime.nodes[2].step_type, StepType::End);
    // Edges: Start->step_1, step_1->End
    assert_eq!(runtime.edges.len(), 2);
}

#[test]
fn test_parse_designer_container() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "container_1",
                "node_type": "raisin:FlowContainer",
                "container_type": "ai_sequence",
                "ai_config": {
                    "tool_mode": "auto",
                    "max_iterations": 10,
                    "thinking_enabled": true,
                    "on_error": "stop"
                },
                "children": [
                    {
                        "id": "step_1",
                        "node_type": "raisin:FlowStep",
                        "properties": { "action": "Child Step" }
                    }
                ]
            }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    // 1 container + implicit Start + implicit End = 3 nodes
    assert_eq!(runtime.nodes.len(), 3);
    assert_eq!(runtime.nodes[0].step_type, StepType::Start);
    assert_eq!(runtime.nodes[1].step_type, StepType::AIContainer);
    assert_eq!(runtime.nodes[1].children.len(), 1);
    assert_eq!(runtime.nodes[2].step_type, StepType::End);
}

#[test]
fn test_sequential_edges() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            { "id": "step_1", "node_type": "raisin:FlowStep", "properties": {} },
            { "id": "step_2", "node_type": "raisin:FlowStep", "properties": {} },
            { "id": "step_3", "node_type": "raisin:FlowStep", "properties": {} }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    // 3 steps + implicit Start + implicit End = 5 nodes
    assert_eq!(runtime.nodes.len(), 5);
    assert_eq!(runtime.nodes[0].step_type, StepType::Start);
    assert_eq!(runtime.nodes[4].step_type, StepType::End);

    // Edges: Start->step_1, step_1->step_2, step_2->step_3, step_3->End = 4 edges
    assert_eq!(runtime.edges.len(), 4);
    assert_eq!(runtime.edges[0].from, "__implicit_start__");
    assert_eq!(runtime.edges[0].to, "step_1");
    assert_eq!(runtime.edges[1].from, "step_1");
    assert_eq!(runtime.edges[1].to, "step_2");
    assert_eq!(runtime.edges[2].from, "step_2");
    assert_eq!(runtime.edges[2].to, "step_3");
    assert_eq!(runtime.edges[3].from, "step_3");
    assert_eq!(runtime.edges[3].to, "__implicit_end__");
}

#[test]
fn test_implicit_start_node_present() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            { "id": "step_1", "node_type": "raisin:FlowStep", "properties": {} }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    // Verify start_node() can find the implicit start
    let start = runtime.start_node();
    assert!(start.is_some());
    assert_eq!(start.unwrap().id, "__implicit_start__");
    assert!(matches!(start.unwrap().step_type, StepType::Start));
}
