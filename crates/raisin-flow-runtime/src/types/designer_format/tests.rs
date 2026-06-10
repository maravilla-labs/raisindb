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

    // Edges: Start->step_1, step_1->step_2, step_2->step_3, step_3->End
    // (order-insensitive - lowering emits them in reverse)
    assert_eq!(runtime.edges.len(), 4);
    let edge_pairs: std::collections::HashSet<(String, String)> = runtime
        .edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();
    for (from, to) in [
        ("start", "step_1"),
        ("step_1", "step_2"),
        ("step_2", "step_3"),
        ("step_3", "end"),
    ] {
        assert!(
            edge_pairs.contains(&(from.to_string(), to.to_string())),
            "missing edge {} -> {}",
            from,
            to
        );
    }

    // next_node chaining is what the executor follows
    let next = |id: &str| {
        runtime
            .nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.next_node.clone())
    };
    assert_eq!(next("step_1").as_deref(), Some("step_2"));
    assert_eq!(next("step_2").as_deref(), Some("step_3"));
    assert_eq!(next("step_3").as_deref(), Some("end"));
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
    assert_eq!(start.unwrap().id, "start");
    assert!(matches!(start.unwrap().step_type, StepType::Start));
}

#[test]
fn test_flow_level_continue_strategy_lowers_to_continue_on_fail() {
    let json = r#"{
        "version": 1,
        "error_strategy": "continue",
        "timeout_ms": 60000,
        "nodes": [
            { "id": "plain", "node_type": "raisin:FlowStep",
              "properties": { "function_ref": "/fn-a" } },
            { "id": "explicit", "node_type": "raisin:FlowStep",
              "properties": { "function_ref": "/fn-b", "error_edge": "plain" } }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    // Step without explicit error handling inherits continue_on_fail
    let plain = runtime.nodes.iter().find(|n| n.id == "plain").unwrap();
    assert_eq!(
        plain.properties.get("continue_on_fail"),
        Some(&serde_json::Value::Bool(true))
    );

    // Step with its own error_edge keeps its explicit handling
    let explicit = runtime.nodes.iter().find(|n| n.id == "explicit").unwrap();
    assert!(!explicit.properties.contains_key("continue_on_fail"));

    // Flow-level settings preserved in metadata
    assert_eq!(
        runtime.metadata.custom.get("error_strategy"),
        Some(&serde_json::json!("continue"))
    );
    assert_eq!(
        runtime.metadata.custom.get("timeout_ms"),
        Some(&serde_json::json!(60000))
    );
}

#[test]
fn test_chat_config_accepts_designer_ui_shape() {
    // The flow designer UI saves handoff targets with trigger_condition /
    // trigger_phrases and termination as {modes, termination_phrases};
    // both must survive into the lowered runtime properties.
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "chat_1", "node_type": "raisin:FlowStep",
              "properties": {
                "step_type": "chat",
                "chat_config": {
                    "agent_ref": "/agents/support",
                    "max_turns": 20,
                    "handoff_targets": [
                        { "agent_ref": "/agents/billing",
                          "trigger_condition": "input.topic == 'billing'",
                          "trigger_phrases": ["invoice", "refund"] }
                    ],
                    "termination": {
                        "modes": ["user_request", "max_turns"],
                        "termination_phrases": ["goodbye"],
                        "inactivity_timeout_ms": 300000
                    }
                }
              } }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();
    let chat = runtime.nodes.iter().find(|n| n.id == "chat_1").unwrap();

    let targets = chat
        .properties
        .get("handoff_targets")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(targets[0]["condition"], "input.topic == 'billing'");
    assert_eq!(targets[0]["trigger_phrases"][1], "refund");

    let termination = chat.properties.get("termination").unwrap();
    assert_eq!(termination["allow_user_end"], true);
    // "ai_decision" not in modes -> AI may not end the session
    assert_eq!(termination["allow_ai_end"], false);
    assert_eq!(termination["end_keywords"][0], "goodbye");
    assert_eq!(termination["inactivity_timeout_ms"], 300000);
}

#[test]
fn test_or_container_with_ai_router_lowering() {
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "route", "node_type": "raisin:FlowContainer", "container_type": "or",
              "rules": [ { "condition": "input.amount > 1000", "next_step": "review" } ],
              "router": {
                  "agent_ref": "/agents/dispatcher",
                  "min_confidence": 0.6,
                  "default_branch": "standard"
              },
              "children": [
                  { "id": "review", "node_type": "raisin:FlowStep",
                    "properties": { "action": "Manual review" } },
                  { "id": "standard", "node_type": "raisin:FlowStep",
                    "properties": { "action": "Standard processing" } }
              ] },
            { "id": "after", "node_type": "raisin:FlowStep", "properties": {} }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    // Rule cascade entry carries the container id and falls through to the router
    let rule_node = runtime.nodes.iter().find(|n| n.id == "route").unwrap();
    assert!(matches!(rule_node.step_type, StepType::Decision));
    assert_eq!(
        rule_node.properties.get("no_branch"),
        Some(&serde_json::json!("route__router"))
    );

    // Router node: AgentDecision with branches + fallback config, skip = successor
    let router = runtime
        .nodes
        .iter()
        .find(|n| n.id == "route__router")
        .unwrap();
    assert!(matches!(router.step_type, StepType::AgentDecision));
    assert_eq!(
        router.properties.get("agent_ref"),
        Some(&serde_json::json!("/agents/dispatcher"))
    );
    assert_eq!(
        router.properties.get("default_branch"),
        Some(&serde_json::json!("standard"))
    );
    let branches = router
        .properties
        .get("branches")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0]["id"], "review");
    assert_eq!(router.next_node.as_deref(), Some("after"));

    // Children exit to the container successor
    let review = runtime.nodes.iter().find(|n| n.id == "review").unwrap();
    assert_eq!(review.next_node.as_deref(), Some("after"));
}

#[test]
fn test_loop_container_lowering() {
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "ask_each", "node_type": "raisin:FlowContainer", "container_type": "loop",
              "loop": {
                  "over": "${steps.pick.candidates}",
                  "item": "candidate",
                  "index": "candidate_index",
                  "max_iterations": 10,
                  "until": "steps.ask.response == \"accept\""
              },
              "children": [
                  { "id": "ask", "node_type": "raisin:FlowStep",
                    "properties": { "function_ref": "/lib/ask",
                                    "arguments": { "who": "${candidate}" } } },
                  { "id": "record", "node_type": "raisin:FlowStep",
                    "properties": { "function_ref": "/lib/record" } }
              ] },
            { "id": "after", "node_type": "raisin:FlowStep", "properties": {} }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    // Loop node carries the container id with the runtime loop properties
    let loop_node = runtime.nodes.iter().find(|n| n.id == "ask_each").unwrap();
    assert!(matches!(loop_node.step_type, StepType::Loop));
    let p = &loop_node.properties;
    assert_eq!(p.get("loop_type"), Some(&serde_json::json!("for_each")));
    assert_eq!(
        p.get("collection"),
        Some(&serde_json::json!("${steps.pick.candidates}"))
    );
    assert_eq!(p.get("item_var"), Some(&serde_json::json!("candidate")));
    assert_eq!(
        p.get("index_var"),
        Some(&serde_json::json!("candidate_index"))
    );
    assert_eq!(p.get("max_iterations"), Some(&serde_json::json!(10)));
    assert_eq!(
        p.get("until"),
        Some(&serde_json::json!("steps.ask.response == \"accept\""))
    );
    assert_eq!(p.get("body_step"), Some(&serde_json::json!("ask")));
    // Loop exits to the successor when iteration is done
    assert_eq!(loop_node.next_node.as_deref(), Some("after"));

    // Body chain: ask -> record -> back to the loop node (back-edge)
    let next = |id: &str| {
        runtime
            .nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.next_node.clone())
    };
    assert_eq!(next("ask").as_deref(), Some("record"));
    assert_eq!(next("record").as_deref(), Some("ask_each"));
}

#[test]
fn test_loop_config_defaults_item_var() {
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "each", "node_type": "raisin:FlowContainer", "container_type": "loop",
              "loop": { "over": "${input.items}" },
              "children": [
                  { "id": "body", "node_type": "raisin:FlowStep",
                    "properties": { "function_ref": "/lib/body" } }
              ] }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();
    let loop_node = runtime.nodes.iter().find(|n| n.id == "each").unwrap();
    assert_eq!(
        loop_node.properties.get("item_var"),
        Some(&serde_json::json!("item"))
    );
    assert!(!loop_node.properties.contains_key("until"));
    assert!(!loop_node.properties.contains_key("max_iterations"));
}

#[test]
fn test_loop_config_missing_over_is_a_conversion_error() {
    // `over` is required: a loop block without it must fail to parse, so
    // from_workflow_data surfaces an InvalidDefinition error.
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "each", "node_type": "raisin:FlowContainer", "container_type": "loop",
              "loop": { "item": "candidate" },
              "children": [] }
        ]
    }"#;

    let parsed: Result<DesignerFlowDefinition, _> = serde_json::from_str(json);
    assert!(parsed.is_err(), "loop config without 'over' must not parse");

    let err = crate::types::FlowDefinition::from_workflow_data(
        serde_json::from_str::<serde_json::Value>(json).unwrap(),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("over"),
        "error should mention the missing 'over' field: {}",
        err
    );
}

#[test]
fn test_or_container_router_only_carries_container_id() {
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "route", "node_type": "raisin:FlowContainer", "container_type": "or",
              "router": { "agent_ref": "/agents/dispatcher" },
              "children": [
                  { "id": "a", "node_type": "raisin:FlowStep", "properties": { "action": "A" } },
                  { "id": "b", "node_type": "raisin:FlowStep", "properties": { "action": "B" } }
              ] }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    // No rules: the container node itself IS the agent decision
    let router = runtime.nodes.iter().find(|n| n.id == "route").unwrap();
    assert!(matches!(router.step_type, StepType::AgentDecision));
    assert_eq!(router.next_node.as_deref(), Some("end"));
}
