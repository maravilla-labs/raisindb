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
fn test_loop_without_a_shape_is_rejected_by_validation() {
    // `over` is no longer required at PARSE time — a loop may instead be a
    // `while` or a `times` — so the "you must say which kind of loop this is"
    // check moved to validation, where it also covers hand-authored runtime
    // format rather than only the designer lowering.
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "each", "node_type": "raisin:FlowContainer", "container_type": "loop",
              "loop": { "item": "candidate" },
              "children": [] }
        ]
    }"#;

    let def = crate::types::FlowDefinition::from_workflow_data(
        serde_json::from_str::<serde_json::Value>(json).unwrap(),
    )
    .expect("a shapeless loop still parses; validation is what refuses it");

    let err = def.validate().unwrap_err().to_string();
    assert!(
        err.contains("'over'") && err.contains("'while'") && err.contains("'times'"),
        "the error must say what the author may choose from: {err}"
    );
}

/// All three loop shapes reach the runtime. `while` and `times` were previously
/// unreachable from the designer format, because `over` was mandatory and
/// `loop_type` was hardcoded to `for_each`.
#[test]
fn test_loop_shapes_lower_to_their_runtime_types() {
    let lower = |cfg: &str| {
        let json = format!(
            r#"{{ "version": 1, "nodes": [
                {{ "id": "each", "node_type": "raisin:FlowContainer", "container_type": "loop",
                   "loop": {cfg},
                   "children": [ {{ "id": "body", "node_type": "raisin:FlowStep",
                                    "properties": {{ "function_ref": "/lib/x" }} }} ] }}
            ] }}"#
        );
        let def = crate::types::FlowDefinition::from_workflow_data(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        )
        .unwrap();
        let node = def.find_node("each").expect("loop node").clone();
        (
            node.properties
                .get("loop_type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            node,
        )
    };

    let (kind, n) = lower(r#"{ "over": "${input.items}" }"#);
    assert_eq!(kind, "for_each");
    assert!(n.properties.contains_key("collection"));

    let (kind, n) = lower(r#"{ "while": "steps.critic.passed == false" }"#);
    assert_eq!(kind, "while", "a `while` loop must lower as one");
    assert_eq!(
        n.properties.get("condition").and_then(|v| v.as_str()),
        Some("steps.critic.passed == false")
    );

    let (kind, n) = lower(r#"{ "times": 3 }"#);
    assert_eq!(kind, "times");
    assert_eq!(n.properties.get("times").and_then(|v| v.as_u64()), Some(3));
}

/// Naming two shapes is refused rather than resolved by precedence.
#[test]
fn test_loop_with_two_shapes_is_rejected() {
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "each", "node_type": "raisin:FlowContainer", "container_type": "loop",
              "loop": { "over": "${input.items}", "times": 3 },
              "children": [] }
        ]
    }"#;
    let def = crate::types::FlowDefinition::from_workflow_data(
        serde_json::from_str::<serde_json::Value>(json).unwrap(),
    )
    .unwrap();
    assert!(def.validate().unwrap_err().to_string().contains("at once"));
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

/// A parallel container WITHOUT fan_out keeps its historical shape: one
/// static branch per child, default merge strategy.
#[test]
fn test_parallel_container_static_branches() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "fork",
                "node_type": "raisin:FlowContainer",
                "container_type": "parallel",
                "children": [
                    { "id": "a", "node_type": "raisin:FlowStep", "properties": {} },
                    { "id": "b", "node_type": "raisin:FlowStep", "properties": {} }
                ]
            }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    let parallel = runtime
        .nodes
        .iter()
        .find(|n| n.step_type == StepType::Parallel)
        .expect("parallel container lowers to a parallel step");

    let branches = parallel.get_array("branches").expect("static branches");
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[0]["id"], "a");
    assert_eq!(branches[1]["id"], "b");
    assert!(parallel.get_property("for_each").is_none());
    assert_eq!(
        parallel.get_string_property("merge_strategy").as_deref(),
        Some("merge_all")
    );
}

/// With fan_out, the children become ONE branch subgraph instantiated per
/// item of a runtime collection - this is what makes a per-row human task
/// (one task per item, then join) expressible from the designer format.
#[test]
fn test_parallel_container_fan_out_lowering() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "collect",
                "node_type": "raisin:FlowContainer",
                "container_type": "parallel",
                "fan_out": { "over": "${steps.plan.items}", "max_branches": 50 },
                "merge_strategy": "all_success",
                "children": [
                    { "id": "approve", "node_type": "raisin:FlowStep", "properties": {} }
                ]
            }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    let parallel = runtime
        .nodes
        .iter()
        .find(|n| n.step_type == StepType::Parallel)
        .expect("parallel container lowers to a parallel step");

    assert_eq!(
        parallel.get_string_property("for_each").as_deref(),
        Some("${steps.plan.items}")
    );
    assert_eq!(parallel.get_u32_property("max_branches"), Some(50));
    assert_eq!(
        parallel.get_string_property("merge_strategy").as_deref(),
        Some("all_success")
    );
    // A fan-out has a branch TEMPLATE, not a branch list
    assert!(parallel.get_property("branches").is_none());

    let branch = parallel.get_object("branch").expect("branch template");
    assert_eq!(branch["id"], "collect-${index}");
    assert_eq!(branch["input_mapping"]["item"], "${item}");
    // The child subgraph is a self-contained child flow
    let nodes = branch["flow_definition"]["nodes"].as_array().unwrap();
    assert_eq!(nodes[0]["id"], "start");
    assert_eq!(nodes[0]["next_node"], "approve");
    assert_eq!(nodes.last().unwrap()["id"], "end");
}

/// `wait`, `sub_flow`, and free-standing `decision` used to be runtime-format
/// only, so needing any one of them forced the WHOLE flow out of the
/// designer format (the two formats cannot be mixed in one definition).
#[test]
fn test_wait_and_sub_flow_steps_in_designer_format() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "cool_off",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Cool off",
                    "step_type": "wait",
                    "duration": "30m"
                }
            },
            {
                "id": "run_child",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Run child flow",
                    "step_type": "sub_flow",
                    "flow_ref": "/flows/settle-order",
                    "input_mapping": { "order_id": "${input.order_id}" },
                    "async": true
                }
            }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    let wait = runtime
        .nodes
        .iter()
        .find(|n| n.id == "cool_off")
        .expect("wait step lowers");
    assert_eq!(wait.step_type, StepType::Wait);
    // wait_type defaults to delay so the lowered node is self-describing
    assert_eq!(
        wait.get_string_property("wait_type").as_deref(),
        Some("delay")
    );
    assert_eq!(wait.get_string_property("duration").as_deref(), Some("30m"));

    let sub = runtime
        .nodes
        .iter()
        .find(|n| n.id == "run_child")
        .expect("sub_flow step lowers");
    assert_eq!(sub.step_type, StepType::SubFlow);
    assert_eq!(
        sub.get_string_property("flow_ref").as_deref(),
        Some("/flows/settle-order")
    );
    assert_eq!(sub.get_bool_property("async"), Some(true));
    assert_eq!(
        sub.get_object("input_mapping").unwrap()["order_id"],
        "${input.order_id}"
    );
}

/// A designer decision names only the arm that diverges; the other falls
/// through to the next sibling. Both must be present on the lowered node or
/// the decision handler rejects it at run time for a missing property.
#[test]
fn test_decision_step_defaults_unnamed_arm_to_successor() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "needs_review",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Large order?",
                    "step_type": "decision",
                    "condition": "input.amount > 500",
                    "yes_branch": "escalate"
                }
            },
            { "id": "ship", "node_type": "raisin:FlowStep", "properties": { "action": "Ship" } },
            { "id": "escalate", "node_type": "raisin:FlowStep", "properties": { "action": "Escalate" } }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    let decision = runtime
        .nodes
        .iter()
        .find(|n| n.id == "needs_review")
        .expect("decision step lowers");
    assert_eq!(decision.step_type, StepType::Decision);
    assert_eq!(
        decision.get_string_property("condition").as_deref(),
        Some("input.amount > 500")
    );
    assert_eq!(
        decision.get_string_property("yes_branch").as_deref(),
        Some("escalate")
    );
    // Unnamed arm falls through to the next sibling
    assert_eq!(
        decision.get_string_property("no_branch").as_deref(),
        Some("ship")
    );
}

/// A step carrying only a bare `condition` (no explicit step_type) is still
/// a decision — and must not lower to a node the handler rejects.
#[test]
fn test_bare_condition_step_gets_both_branches() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "gate",
                "node_type": "raisin:FlowStep",
                "properties": { "action": "Gate", "condition": "input.ok == true" }
            },
            { "id": "after", "node_type": "raisin:FlowStep", "properties": { "action": "After" } }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();

    let gate = runtime.nodes.iter().find(|n| n.id == "gate").unwrap();
    assert_eq!(gate.step_type, StepType::Decision);
    assert_eq!(
        gate.get_string_property("yes_branch").as_deref(),
        Some("after")
    );
    assert_eq!(
        gate.get_string_property("no_branch").as_deref(),
        Some("after")
    );
}

/// Regression: a templated deadline in DESIGNER format used to fail to
/// DESERIALIZE (`invalid type: string "${...}", expected i64`), so the whole
/// flow definition refused to load — the property could not be templated in
/// the canonical authoring format at all, and attempting it broke the flow.
#[test]
fn test_designer_human_task_accepts_templated_deadline_and_priority() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "approve",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Approve",
                    "step_type": "human_task",
                    "task_type": "approval",
                    "assignee": "/users/manager",
                    "due_in_seconds": "${input.due_secs}",
                    "priority": "{{ input.tier }}"
                }
            }
        ]
    }"#;

    let def: DesignerFlowDefinition =
        serde_json::from_str(json).expect("a templated deadline must not break the definition");
    let runtime = def.to_runtime_format();

    let step = runtime
        .nodes
        .iter()
        .find(|n| n.id == "approve")
        .expect("human task lowers");
    assert_eq!(step.step_type, StepType::HumanTask);
    // Passed through RAW: the handler resolves and coerces at run time.
    assert_eq!(
        step.get_string_property("due_in_seconds").as_deref(),
        Some("${input.due_secs}")
    );
    assert_eq!(
        step.get_string_property("priority").as_deref(),
        Some("{{ input.tier }}")
    );
}

/// Literal numbers must still lower as numbers, not as strings.
#[test]
fn test_designer_human_task_literal_deadline_stays_numeric() {
    let json = r#"{
        "version": 1,
        "error_strategy": "fail_fast",
        "nodes": [
            {
                "id": "approve",
                "node_type": "raisin:FlowStep",
                "properties": {
                    "action": "Approve",
                    "step_type": "human_task",
                    "task_type": "approval",
                    "assignee": "/users/manager",
                    "due_in_seconds": 86400,
                    "priority": 4
                }
            }
        ]
    }"#;

    let def: DesignerFlowDefinition = serde_json::from_str(json).unwrap();
    let runtime = def.to_runtime_format();
    let step = runtime.nodes.iter().find(|n| n.id == "approve").unwrap();
    assert_eq!(step.get_i64_property("due_in_seconds"), Some(86400));
    assert_eq!(step.get_u32_property("priority"), Some(4));
}

/// An unbounded `while` reaches the runtime, and the bounded default is
/// preserved when the flag is absent.
#[test]
fn test_unbounded_while_lowers_and_defaults_to_bounded() {
    let lower = |cfg: &str| {
        let json = format!(
            r#"{{ "version": 1, "nodes": [
                {{ "id": "spin", "node_type": "raisin:FlowContainer", "container_type": "loop",
                   "loop": {cfg},
                   "children": [ {{ "id": "body", "node_type": "raisin:FlowStep",
                                    "properties": {{ "function_ref": "/lib/x" }} }} ] }}
            ] }}"#
        );
        crate::types::FlowDefinition::from_workflow_data(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        )
        .unwrap()
    };

    let def = lower(r#"{ "while": "steps.critic.passed == false", "unbounded": true }"#);
    let node = def.find_node("spin").unwrap();
    assert_eq!(
        node.properties.get("unbounded").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(def.validate().is_ok(), "{:?}", def.validate());

    // Absent, the safety ceiling stays: silently dropping a limit is not
    // something an author should get by omission.
    let def = lower(r#"{ "while": "steps.critic.passed == false" }"#);
    assert!(def
        .find_node("spin")
        .unwrap()
        .properties
        .get("unbounded")
        .is_none());
}

/// `unbounded` is meaningless on a for_each or a times loop, so it is refused
/// rather than accepted as a word that does nothing.
#[test]
fn test_unbounded_is_rejected_on_bounded_loop_shapes() {
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "each", "node_type": "raisin:FlowContainer", "container_type": "loop",
              "loop": { "over": "${input.items}", "unbounded": true },
              "children": [] }
        ]
    }"#;
    let def = crate::types::FlowDefinition::from_workflow_data(
        serde_json::from_str::<serde_json::Value>(json).unwrap(),
    )
    .unwrap();
    let err = def.validate().unwrap_err().to_string();
    assert!(err.contains("unbounded"), "{err}");
}

/// Naming both a ceiling and no ceiling is a contradiction, not a precedence
/// puzzle.
#[test]
fn test_unbounded_with_max_iterations_is_rejected() {
    let json = r#"{
        "version": 1,
        "nodes": [
            { "id": "spin", "node_type": "raisin:FlowContainer", "container_type": "loop",
              "loop": { "while": "true", "unbounded": true, "max_iterations": 10 },
              "children": [] }
        ]
    }"#;
    let def = crate::types::FlowDefinition::from_workflow_data(
        serde_json::from_str::<serde_json::Value>(json).unwrap(),
    )
    .unwrap();
    assert!(def.validate().unwrap_err().to_string().contains("pick one"));
}
