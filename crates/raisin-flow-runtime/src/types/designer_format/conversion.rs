// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Conversion from designer format to runtime format
//!
//! Flattens the tree-based designer structure into the flat graph format
//! expected by the runtime, injecting implicit Start/End nodes and edges.

use serde_json::Value;
use std::collections::HashMap;

use super::config_types::{DesignerAiConfig, DesignerToolMode};
use super::types::{
    DesignerContainerType, DesignerFlowDefinition, DesignerNode, DesignerStepProperties,
    DesignerStepType, ExecutionIdentityMode, StepErrorBehavior,
};
use crate::types::flow_definition::{
    AIContainerConfig, AiExecutionConfig, FlowDefinition, FlowEdge, FlowMetadata, FlowNode,
    StepType, ToolMode,
};

/// Is `id` used by any node in this tree, at any depth?
///
/// Ids share ONE namespace across nesting levels — `find_node` falls through to
/// children — so a child called `start` collides with the injected Start node
/// just as a top-level one does.
fn contains_id(nodes: &[FlowNode], id: &str) -> bool {
    nodes
        .iter()
        .any(|n| n.id == id || contains_id(&n.children, id))
}

impl DesignerFlowDefinition {
    /// Convert designer format to runtime format.
    ///
    /// This flattens the tree structure and generates edges for navigation.
    /// Automatically injects implicit Start and End nodes since the designer
    /// UI shows these visually but doesn't save them to workflow_data.
    pub fn to_runtime_format(self) -> FlowDefinition {
        let mut runtime_nodes = Vec::new();
        let mut edges = Vec::new();

        // Lower the designer tree into a flat runtime graph. Containers are
        // lowered into executable constructs:
        // - AND  -> pass-through + sequential chain of children
        // - OR   -> REL decision cascade routing to exactly one child
        // - Parallel -> Parallel step with per-child inline branch flows
        // - AI Sequence -> AIContainer step (children kept as tool steps)
        let entry = self.lower_nodes(&self.nodes, "end", &mut runtime_nodes, &mut edges);

        // Inject implicit Start/End nodes (the designer shows these visually but
        // doesn't save them). The canonical ids "start"/"end" match the
        // fallbacks used throughout the runtime handlers.
        //
        // ONLY IF THE AUTHOR HAS NOT USED THOSE IDS. A flow whose own first step
        // is called `start` used to get a second node with the same id, whose
        // `next_node` pointed at `start` — i.e. at itself, since that is what
        // `entry` resolved to. It survived only because `build_index` is
        // last-wins, so the lookup happened to land on the author's node and the
        // injected one was shadowed into harmlessness. On the linear fallback
        // path `find_node` returns the FIRST match instead — the injected Start —
        // and the flow spins on itself forever. Behaviour that depends on
        // whether an index was built is not behaviour; when the author has
        // already named a node `start`, that node IS the entry.
        let start_exists = contains_id(&runtime_nodes, "start");
        let end_exists = contains_id(&runtime_nodes, "end");

        let start_node = (!start_exists).then(|| FlowNode {
            id: "start".to_string(),
            step_type: StepType::Start,
            properties: HashMap::new(),
            children: Vec::new(),
            next_node: Some(entry.clone()),
        });
        if !start_exists {
            edges.insert(
                0,
                FlowEdge {
                    from: "start".to_string(),
                    to: entry,
                    label: None,
                    condition: None,
                },
            );
        }

        let end_node = (!end_exists).then(|| FlowNode {
            id: "end".to_string(),
            step_type: StepType::End,
            properties: HashMap::new(),
            children: Vec::new(),
            next_node: None,
        });

        // Flow-level error_strategy "continue" lowers to continue_on_fail on
        // work steps that don't declare their own error handling.
        if self.error_strategy == super::types::DesignerErrorStrategy::Continue {
            for node in runtime_nodes.iter_mut() {
                let is_work_step = matches!(
                    node.step_type,
                    StepType::FunctionStep | StepType::AgentStep | StepType::AIContainer
                );
                let has_explicit_error_handling = node.properties.contains_key("continue_on_fail")
                    || node.properties.contains_key("error_edge");
                if is_work_step && !has_explicit_error_handling {
                    node.properties
                        .insert("continue_on_fail".to_string(), Value::Bool(true));
                }
            }
        }

        // Build final node list: Start + lowered nodes + End (each cap present
        // only when the author did not already claim its id).
        let mut all_nodes: Vec<FlowNode> = start_node.into_iter().collect();
        all_nodes.append(&mut runtime_nodes);
        all_nodes.extend(end_node);

        // Preserve flow-level settings in metadata
        let mut metadata = FlowMetadata::default();
        metadata.custom.insert(
            "error_strategy".to_string(),
            serde_json::to_value(self.error_strategy).unwrap_or(Value::Null),
        );
        if let Some(timeout) = self.timeout_ms {
            metadata
                .custom
                .insert("timeout_ms".to_string(), Value::Number(timeout.into()));
        }

        // Note: node_index is built by the caller (from_workflow_data)
        FlowDefinition {
            nodes: all_nodes,
            edges,
            metadata,
            node_index: None,
        }
    }

    /// Lower a sequence of sibling nodes so each routes to the next, with
    /// the final one routing to `successor`. Returns the entry node id
    /// (or `successor` when the sequence is empty).
    fn lower_nodes(
        &self,
        nodes: &[DesignerNode],
        successor: &str,
        output: &mut Vec<FlowNode>,
        edges: &mut Vec<FlowEdge>,
    ) -> String {
        let mut next = successor.to_string();
        for node in nodes.iter().rev() {
            next = self.lower_node(node, &next, output, edges);
        }
        next
    }

    /// Lower a sequence of designer nodes into a self-contained inline flow
    /// definition (`{ nodes: [start, …, end] }`) suitable for a parallel
    /// branch - a child flow instance, so it needs its own start/end.
    fn lower_children_to_inline_flow(&self, children: &[DesignerNode]) -> Value {
        let mut branch_nodes: Vec<FlowNode> = Vec::new();
        let mut branch_edges: Vec<FlowEdge> = Vec::new();
        let entry = self.lower_nodes(children, "end", &mut branch_nodes, &mut branch_edges);

        let mut nodes_json: Vec<Value> = vec![serde_json::json!({
            "id": "start",
            "step_type": "start",
            "next_node": entry,
        })];
        for node in &branch_nodes {
            nodes_json.push(serde_json::to_value(node).unwrap_or(Value::Null));
        }
        nodes_json.push(serde_json::json!({
            "id": "end",
            "step_type": "end",
        }));

        serde_json::json!({ "nodes": nodes_json })
    }

    /// Lower a single designer node into executable runtime node(s).
    /// Returns the entry id for the lowered construct (always the node's
    /// own id, so rules / error edges referencing it keep working).
    fn lower_node(
        &self,
        node: &DesignerNode,
        successor: &str,
        output: &mut Vec<FlowNode>,
        edges: &mut Vec<FlowEdge>,
    ) -> String {
        match node {
            DesignerNode::Step {
                id,
                properties,
                on_error,
                error_edge,
            } => {
                let mut runtime_node =
                    convert_step_node(id, properties, on_error, error_edge, self);
                fill_decision_branches(&mut runtime_node, successor);
                runtime_node.next_node = Some(successor.to_string());
                edges.push(FlowEdge {
                    from: id.clone(),
                    to: successor.to_string(),
                    label: None,
                    condition: None,
                });
                output.push(runtime_node);
                id.clone()
            }

            DesignerNode::Container {
                id,
                container_type,
                children,
                ai_config,
                rules,
                router,
                referee,
                loop_config,
                fan_out,
                merge_strategy,
                prompt,
                timeout_ms,
            } => match container_type {
                // AND: all children execute sequentially, then continue
                DesignerContainerType::And => {
                    let entry = self.lower_nodes(children, successor, output, edges);
                    let mut props = HashMap::new();
                    props.insert(
                        "container_type".to_string(),
                        Value::String("and".to_string()),
                    );
                    output.push(FlowNode {
                        id: id.clone(),
                        step_type: StepType::Start, // pass-through
                        properties: props,
                        children: Vec::new(),
                        next_node: Some(entry.clone()),
                    });
                    edges.push(FlowEdge {
                        from: id.clone(),
                        to: entry,
                        label: None,
                        condition: None,
                    });
                    id.clone()
                }

                // OR: REL rules route to exactly one child; that child then
                // exits the container. No matching rule skips the container.
                DesignerContainerType::Or => {
                    // Each child is one alternative branch exiting to the
                    // container successor
                    for child in children {
                        self.lower_node(child, successor, output, edges);
                    }

                    // Effective rules: explicit container rules, else each
                    // child's own `condition` property
                    let mut effective: Vec<(String, String)> = rules
                        .iter()
                        .map(|r| (r.condition.clone(), r.next_step.clone()))
                        .collect();
                    if effective.is_empty() {
                        for child in children {
                            if let DesignerNode::Step { id, properties, .. } = child {
                                if let Some(cond) = &properties.condition {
                                    effective.push((cond.clone(), id.clone()));
                                }
                            }
                        }
                    }

                    // AI router: the agent decides among the children when
                    // no REL rule matched. It becomes the cascade's final
                    // fallthrough (or the container node itself when there
                    // are no rules).
                    let router_node_id = router.as_ref().map(|router_cfg| {
                        let router_id = if effective.is_empty() {
                            id.clone()
                        } else {
                            format!("{}__router", id)
                        };
                        let branches: Vec<Value> = children
                            .iter()
                            .map(|c| {
                                let description = match c {
                                    DesignerNode::Step { properties, .. } => properties
                                        .action
                                        .clone()
                                        .or_else(|| properties.condition.clone())
                                        .unwrap_or_else(|| c.id().to_string()),
                                    DesignerNode::Container { .. } => c.id().to_string(),
                                };
                                serde_json::json!({ "id": c.id(), "description": description })
                            })
                            .collect();

                        let mut props = HashMap::new();
                        let agent_path = router_cfg
                            .agent_ref
                            .raisin_path
                            .clone()
                            .unwrap_or_else(|| router_cfg.agent_ref.raisin_ref.clone());
                        props.insert("agent_ref".to_string(), Value::String(agent_path));
                        props.insert(
                            "agent_workspace".to_string(),
                            Value::String(router_cfg.agent_ref.raisin_workspace.clone()),
                        );
                        props.insert("branches".to_string(), Value::Array(branches));
                        if let Some(prompt) = &router_cfg.prompt {
                            props.insert("prompt".to_string(), Value::String(prompt.clone()));
                        }
                        if let Some(min_confidence) = router_cfg.min_confidence {
                            if let Some(num) = serde_json::Number::from_f64(min_confidence) {
                                props.insert("min_confidence".to_string(), Value::Number(num));
                            }
                        }
                        if let Some(default_branch) = &router_cfg.default_branch {
                            props.insert(
                                "default_branch".to_string(),
                                Value::String(default_branch.clone()),
                            );
                        }
                        if let Some(include_context) = &router_cfg.include_context {
                            props.insert("include_context".to_string(), include_context.clone());
                        }

                        for child in children.iter() {
                            edges.push(FlowEdge {
                                from: router_id.clone(),
                                to: child.id().to_string(),
                                label: Some("ai".to_string()),
                                condition: None,
                            });
                        }
                        edges.push(FlowEdge {
                            from: router_id.clone(),
                            to: successor.to_string(),
                            label: Some("skip".to_string()),
                            condition: None,
                        });
                        output.push(FlowNode {
                            id: router_id.clone(),
                            step_type: StepType::AgentDecision,
                            properties: props,
                            children: Vec::new(),
                            next_node: Some(successor.to_string()),
                        });
                        router_id
                    });

                    if effective.is_empty() {
                        if let Some(router_id) = router_node_id {
                            // The router node carries the container id
                            return router_id;
                        }
                        // No routing information: pass through to the first
                        // child (sequential fallback)
                        let target = children
                            .first()
                            .map(|c| c.id().to_string())
                            .unwrap_or_else(|| successor.to_string());
                        output.push(FlowNode {
                            id: id.clone(),
                            step_type: StepType::Start,
                            properties: HashMap::new(),
                            children: Vec::new(),
                            next_node: Some(target.clone()),
                        });
                        edges.push(FlowEdge {
                            from: id.clone(),
                            to: target,
                            label: None,
                            condition: None,
                        });
                        return id.clone();
                    }

                    // Build the decision cascade in reverse: rule k falls
                    // through to rule k+1; the last falls through to the
                    // AI router (when configured) or the successor. The
                    // first decision carries the container id.
                    let mut fallthrough = router_node_id.unwrap_or_else(|| successor.to_string());
                    for (idx, (condition, target)) in effective.iter().enumerate().rev() {
                        let node_id = if idx == 0 {
                            id.clone()
                        } else {
                            format!("{}__rule{}", id, idx)
                        };
                        let mut props = HashMap::new();
                        props.insert("condition".to_string(), Value::String(condition.clone()));
                        props.insert("yes_branch".to_string(), Value::String(target.clone()));
                        props.insert("no_branch".to_string(), Value::String(fallthrough.clone()));
                        edges.push(FlowEdge {
                            from: node_id.clone(),
                            to: target.clone(),
                            label: Some("yes".to_string()),
                            condition: Some(condition.clone()),
                        });
                        edges.push(FlowEdge {
                            from: node_id.clone(),
                            to: fallthrough.clone(),
                            label: Some("no".to_string()),
                            condition: None,
                        });
                        output.push(FlowNode {
                            id: node_id.clone(),
                            step_type: StepType::Decision,
                            properties: props,
                            children: Vec::new(),
                            next_node: None,
                        });
                        fallthrough = node_id;
                    }
                    id.clone()
                }

                // Parallel: each child becomes an inline branch flow, or -
                // with a fan_out config - the children become ONE branch
                // subgraph run once per item of a runtime collection.
                DesignerContainerType::Parallel => {
                    let mut props = HashMap::new();

                    match fan_out {
                        Some(cfg) => {
                            // All children lower into a single branch
                            // subgraph; the fan-out width comes from the
                            // collection, not from the child count.
                            let branch_flow = self.lower_children_to_inline_flow(children);
                            props.insert("for_each".to_string(), Value::String(cfg.over.clone()));
                            if let Some(max) = cfg.max_branches {
                                props.insert("max_branches".to_string(), Value::Number(max.into()));
                            }
                            props.insert(
                                "branch".to_string(),
                                serde_json::json!({
                                    "id": format!("{}-${{index}}", id),
                                    "flow_definition": branch_flow,
                                    // `item` / `index` are bound by the
                                    // runtime; passing them explicitly keeps
                                    // the child input shape stable and
                                    // self-describing.
                                    "input_mapping": {
                                        "item": "${item}",
                                        "index": "${index}",
                                    },
                                }),
                            );
                        }
                        None => {
                            let mut branches: Vec<Value> = Vec::new();
                            for child in children {
                                branches.push(serde_json::json!({
                                    "id": child.id(),
                                    "flow_definition":
                                        self.lower_children_to_inline_flow(std::slice::from_ref(
                                            child,
                                        )),
                                }));
                            }
                            props.insert("branches".to_string(), Value::Array(branches));
                        }
                    }

                    props.insert(
                        "merge_strategy".to_string(),
                        Value::String(
                            merge_strategy
                                .clone()
                                .unwrap_or_else(|| "merge_all".to_string()),
                        ),
                    );
                    if let Some(timeout) = timeout_ms {
                        props.insert("timeout_ms".to_string(), Value::Number((*timeout).into()));
                    }
                    edges.push(FlowEdge {
                        from: id.clone(),
                        to: successor.to_string(),
                        label: None,
                        condition: None,
                    });
                    output.push(FlowNode {
                        id: id.clone(),
                        step_type: StepType::Parallel,
                        properties: props,
                        children: Vec::new(),
                        next_node: Some(successor.to_string()),
                    });
                    id.clone()
                }

                // AI Sequence: AIContainer step; children stay attached as
                // (potential) explicit tool steps
                DesignerContainerType::AiSequence => {
                    let (mut container_node, child_edges) = convert_container_node(
                        id,
                        container_type,
                        children,
                        ai_config,
                        timeout_ms,
                        self,
                    );
                    // Container-level task prompt + context injection
                    if let Some(p) = prompt {
                        container_node
                            .properties
                            .insert("prompt".to_string(), Value::String(p.clone()));
                    }
                    if let Some(cfg) = ai_config {
                        if let Some(ic) = &cfg.include_context {
                            container_node
                                .properties
                                .insert("include_context".to_string(), ic.clone());
                        }
                    }
                    container_node.next_node = Some(successor.to_string());
                    edges.push(FlowEdge {
                        from: id.clone(),
                        to: successor.to_string(),
                        label: None,
                        condition: None,
                    });
                    edges.extend(child_edges);
                    output.push(container_node);
                    id.clone()
                }

                // Competition: every child agent answers the same task, a
                // referee judges. Lowered to a single Competition step
                // carrying the competitor + referee config.
                DesignerContainerType::Competition => {
                    let competitors: Vec<Value> = children
                        .iter()
                        .filter_map(|c| match c {
                            DesignerNode::Step { id, properties, .. } => {
                                let agent_ref = properties.agent_ref.as_ref()?;
                                let path = agent_ref
                                    .raisin_path
                                    .clone()
                                    .unwrap_or_else(|| agent_ref.raisin_ref.clone());
                                let mut competitor = serde_json::json!({
                                    "id": id,
                                    "agent_ref": path,
                                    "agent_workspace": agent_ref.raisin_workspace,
                                });
                                if let Some(p) = &properties.prompt {
                                    competitor["prompt"] = Value::String(p.clone());
                                }
                                if let Some(a) = &properties.action {
                                    competitor["description"] = Value::String(a.clone());
                                }
                                Some(competitor)
                            }
                            DesignerNode::Container { .. } => None,
                        })
                        .collect();

                    let mut props = HashMap::new();
                    props.insert("competitors".to_string(), Value::Array(competitors));
                    if let Some(p) = prompt {
                        props.insert("prompt".to_string(), Value::String(p.clone()));
                    }
                    if let Some(referee_cfg) = referee {
                        let referee_path = referee_cfg
                            .agent_ref
                            .raisin_path
                            .clone()
                            .unwrap_or_else(|| referee_cfg.agent_ref.raisin_ref.clone());
                        props.insert("referee_agent_ref".to_string(), Value::String(referee_path));
                        props.insert(
                            "referee_agent_workspace".to_string(),
                            Value::String(referee_cfg.agent_ref.raisin_workspace.clone()),
                        );
                        if let Some(p) = &referee_cfg.prompt {
                            props.insert("referee_prompt".to_string(), Value::String(p.clone()));
                        }
                        if let Some(mc) = referee_cfg.min_confidence {
                            if let Some(num) = serde_json::Number::from_f64(mc) {
                                props.insert("min_confidence".to_string(), Value::Number(num));
                            }
                        }
                        if let Some(rounds) = referee_cfg.max_rounds {
                            props.insert("max_rounds".to_string(), Value::Number(rounds.into()));
                        }
                        if let Some(include_context) = &referee_cfg.include_context {
                            props.insert("include_context".to_string(), include_context.clone());
                        }
                    }
                    if let Some(timeout) = timeout_ms {
                        props.insert("timeout_ms".to_string(), Value::Number((*timeout).into()));
                    }

                    edges.push(FlowEdge {
                        from: id.clone(),
                        to: successor.to_string(),
                        label: None,
                        condition: None,
                    });
                    output.push(FlowNode {
                        id: id.clone(),
                        step_type: StepType::Competition,
                        properties: props,
                        children: Vec::new(),
                        next_node: Some(successor.to_string()),
                    });
                    id.clone()
                }

                // Loop: the children form the body and execute once per
                // item of the collection. The body chain's successor is
                // the loop node itself (back-edge): after each iteration
                // the Loop handler advances and re-enters the body, and
                // exits to the container successor when done.
                DesignerContainerType::Loop => {
                    let body_entry = self.lower_nodes(children, id, output, edges);

                    let mut props = HashMap::new();
                    // The SHAPE comes from the config, not a hardcoded default.
                    // `for_each` used to be written in unconditionally, which is
                    // what made the runtime's `while` and `times` arms
                    // unreachable from the designer format.
                    props.insert(
                        "loop_type".to_string(),
                        Value::String(
                            loop_config
                                .as_ref()
                                .map(|c| c.loop_type())
                                .unwrap_or("for_each")
                                .to_string(),
                        ),
                    );
                    if let Some(cfg) = loop_config {
                        if let Some(over) = &cfg.over {
                            props.insert("collection".to_string(), Value::String(over.clone()));
                        }
                        if let Some(cond) = &cfg.while_condition {
                            props.insert("condition".to_string(), Value::String(cond.clone()));
                        }
                        // Lowered whenever it is set, NOT only for a `while`.
                        // Dropping it on a for_each here would mean the author
                        // wrote a word that did nothing and never heard about
                        // it; carrying it through is what lets `validate` say so.
                        if cfg.unbounded {
                            props.insert("unbounded".to_string(), Value::Bool(true));
                        }
                        if let Some(times) = cfg.times {
                            props.insert("times".to_string(), Value::Number(times.into()));
                        }
                        props.insert("item_var".to_string(), Value::String(cfg.item.clone()));
                        if let Some(index) = &cfg.index {
                            props.insert("index_var".to_string(), Value::String(index.clone()));
                        }
                        if let Some(max) = cfg.max_iterations {
                            props.insert("max_iterations".to_string(), Value::Number(max.into()));
                        }
                        if let Some(until) = &cfg.until {
                            props.insert("until".to_string(), Value::String(until.clone()));
                        }
                    }
                    // Empty body degenerates to the loop node itself: the
                    // handler then just advances through the items.
                    props.insert("body_step".to_string(), Value::String(body_entry.clone()));
                    if let Some(timeout) = timeout_ms {
                        props.insert("timeout_ms".to_string(), Value::Number((*timeout).into()));
                    }

                    edges.push(FlowEdge {
                        from: id.clone(),
                        to: body_entry,
                        label: Some("each".to_string()),
                        condition: None,
                    });
                    edges.push(FlowEdge {
                        from: id.clone(),
                        to: successor.to_string(),
                        label: Some("done".to_string()),
                        condition: None,
                    });
                    output.push(FlowNode {
                        id: id.clone(),
                        step_type: StepType::Loop,
                        properties: props,
                        children: Vec::new(),
                        next_node: Some(successor.to_string()),
                    });
                    id.clone()
                }
            },
        }
    }

    /// Determine the runtime StepType based on designer properties
    pub(crate) fn determine_step_type(&self, props: &DesignerStepProperties) -> StepType {
        if matches!(props.step_type, Some(DesignerStepType::HumanTask)) || props.task_type.is_some()
        {
            // Explicit human task (approval / input / review / action)
            StepType::HumanTask
        } else if matches!(props.step_type, Some(DesignerStepType::Wait)) {
            StepType::Wait
        } else if matches!(props.step_type, Some(DesignerStepType::SubFlow)) {
            StepType::SubFlow
        } else if matches!(props.step_type, Some(DesignerStepType::Decision)) {
            StepType::Decision
        } else if props.function_ref.is_some() {
            StepType::FunctionStep
        } else if matches!(props.step_type, Some(DesignerStepType::AiAgent)) {
            // Explicit ai_agent step type → lightweight single-shot handler
            StepType::AgentStep
        } else if props.agent_ref.is_some() {
            // Has agent_ref but not explicitly ai_agent → full AI container (backward compat)
            StepType::AIContainer
        } else if matches!(props.step_type, Some(DesignerStepType::Chat)) {
            StepType::Chat
        } else if props.condition.is_some() {
            StepType::Decision
        } else {
            StepType::FunctionStep // Default to function step
        }
    }
}

/// Default a decision step's unnamed arm to the sequential successor.
///
/// The runtime's decision handler requires BOTH `yes_branch` and
/// `no_branch`, but designer format is sequential: the useful spelling is to
/// name only the arm that diverges and let the other fall through to the
/// next sibling. Without this, a designer decision step lowered to a node
/// the handler rejects at run time for a missing property.
fn fill_decision_branches(node: &mut FlowNode, successor: &str) {
    if node.step_type != StepType::Decision {
        return;
    }
    for arm in ["yes_branch", "no_branch"] {
        node.properties
            .entry(arm.to_string())
            .or_insert_with(|| Value::String(successor.to_string()));
    }
}

/// Convert a designer step node to a runtime FlowNode
fn convert_step_node(
    id: &str,
    properties: &DesignerStepProperties,
    on_error: &Option<StepErrorBehavior>,
    node_error_edge: &Option<String>,
    def: &DesignerFlowDefinition,
) -> FlowNode {
    let step_type = def.determine_step_type(properties);
    let mut props = HashMap::new();

    // Copy relevant properties
    if let Some(action) = &properties.action {
        props.insert("action".to_string(), Value::String(action.clone()));
    }
    if let Some(func_ref) = &properties.function_ref {
        let path = func_ref
            .raisin_path
            .as_ref()
            .cloned()
            .unwrap_or_else(|| func_ref.raisin_ref.clone());
        props.insert("function_ref".to_string(), Value::String(path));
        props.insert(
            "function_workspace".to_string(),
            Value::String(func_ref.raisin_workspace.clone()),
        );
    }
    if let Some(agent_ref) = &properties.agent_ref {
        let path = agent_ref
            .raisin_path
            .as_ref()
            .cloned()
            .unwrap_or_else(|| agent_ref.raisin_ref.clone());
        props.insert("agent_ref".to_string(), Value::String(path));
        props.insert(
            "agent_workspace".to_string(),
            Value::String(agent_ref.raisin_workspace.clone()),
        );
    }
    if let Some(arguments) = &properties.arguments {
        props.insert("arguments".to_string(), arguments.clone());
    }
    if let Some(prompt) = &properties.prompt {
        props.insert("prompt".to_string(), Value::String(prompt.clone()));
    }
    if let Some(response_format) = &properties.response_format {
        props.insert("response_format".to_string(), response_format.clone());
    }
    if let Some(include_context) = &properties.include_context {
        props.insert("include_context".to_string(), include_context.clone());
    }
    if let Some(lua) = &properties.lua_script {
        props.insert("lua_script".to_string(), Value::String(lua.clone()));
    }
    if let Some(cond) = &properties.condition {
        props.insert("condition".to_string(), Value::String(cond.clone()));
    }

    // Decision branches. Whichever arm is not named falls through to the
    // next sibling; the caller fills that in once the successor is known
    // (`fill_decision_branches`), so a decision step needs only the arm
    // that actually diverges.
    if let Some(target) = &properties.yes_branch {
        props.insert("yes_branch".to_string(), Value::String(target.clone()));
    }
    if let Some(target) = &properties.no_branch {
        props.insert("no_branch".to_string(), Value::String(target.clone()));
    }

    // Wait step
    if let Some(wait_type) = &properties.wait_type {
        props.insert("wait_type".to_string(), Value::String(wait_type.clone()));
    } else if step_type == StepType::Wait {
        // `delay` is the overwhelmingly common case and the handler's own
        // default vocabulary; spelling it out keeps the lowered node
        // self-describing.
        props.insert("wait_type".to_string(), Value::String("delay".to_string()));
    }
    if let Some(duration) = &properties.duration {
        props.insert("duration".to_string(), Value::String(duration.clone()));
    }
    if let Some(until) = &properties.until {
        props.insert("until".to_string(), Value::String(until.clone()));
    }
    if let Some(event_type) = &properties.event_type {
        props.insert("event_type".to_string(), Value::String(event_type.clone()));
    }
    if let Some(timeout) = &properties.timeout {
        props.insert("timeout".to_string(), Value::String(timeout.clone()));
    }
    if let Some(cron) = &properties.cron {
        props.insert("cron".to_string(), Value::String(cron.clone()));
    }

    // Sub-flow step
    if let Some(flow_ref) = &properties.flow_ref {
        let path = flow_ref
            .raisin_path
            .as_ref()
            .cloned()
            .unwrap_or_else(|| flow_ref.raisin_ref.clone());
        props.insert("flow_ref".to_string(), Value::String(path));
        props.insert(
            "flow_workspace".to_string(),
            Value::String(flow_ref.raisin_workspace.clone()),
        );
    }
    if let Some(mapping) = &properties.input_mapping {
        props.insert("input_mapping".to_string(), mapping.clone());
    }
    if let Some(async_mode) = properties.async_mode {
        props.insert("async".to_string(), Value::Bool(async_mode));
    }

    if let Some(key) = &properties.payload_key {
        props.insert("payload_key".to_string(), Value::String(key.clone()));
    }
    if properties.disabled {
        props.insert("disabled".to_string(), Value::Bool(true));
    }
    if let Some(timeout) = properties.timeout_ms {
        props.insert("timeout_ms".to_string(), Value::Number(timeout.into()));
    }

    // Convert retry configuration to runtime properties
    if let Some(retry_strategy) = &properties.retry_strategy {
        props.insert(
            "retry_strategy".to_string(),
            Value::String(retry_strategy.clone()),
        );
        if retry_strategy == "none" {
            props.insert("max_retries".to_string(), Value::Number(0.into()));
        }
    }
    if let Some(retry) = &properties.retry {
        props.insert(
            "max_retries".to_string(),
            Value::Number(retry.max_retries.into()),
        );
        props.insert(
            "retry_base_delay_ms".to_string(),
            Value::Number(retry.base_delay_ms.into()),
        );
        props.insert(
            "retry_max_delay_ms".to_string(),
            Value::Number(retry.max_delay_ms.into()),
        );
    }

    // Error handling properties
    let effective_error_edge = node_error_edge.as_ref().or(properties.error_edge.as_ref());
    if let Some(error_edge) = effective_error_edge {
        props.insert("error_edge".to_string(), Value::String(error_edge.clone()));
    }
    if let Some(on_err) = on_error {
        let on_error_str = match on_err {
            StepErrorBehavior::Stop => "stop",
            StepErrorBehavior::Skip => "skip",
            StepErrorBehavior::Continue => "continue",
        };
        props.insert(
            "on_error".to_string(),
            Value::String(on_error_str.to_string()),
        );
    }
    if let Some(comp_ref) = &properties.compensation_ref {
        let path = comp_ref
            .raisin_path
            .as_ref()
            .cloned()
            .unwrap_or_else(|| comp_ref.raisin_ref.clone());
        props.insert("compensation_ref".to_string(), Value::String(path));
        props.insert(
            "compensation_workspace".to_string(),
            Value::String(comp_ref.raisin_workspace.clone()),
        );
    }
    if let Some(mapping) = &properties.compensation_input_mapping {
        props.insert("compensation_input_mapping".to_string(), mapping.clone());
    }
    if properties.continue_on_fail {
        props.insert("continue_on_fail".to_string(), Value::Bool(true));
    }
    if properties.isolated_branch {
        props.insert("isolated_branch".to_string(), Value::Bool(true));
    }

    // Human task properties. The runtime requires `title`; the designer's
    // step label (`action`) doubles as the task title.
    if let Some(task_type) = &properties.task_type {
        props.insert("task_type".to_string(), Value::String(task_type.clone()));
        if !props.contains_key("title") {
            let title = properties
                .action
                .clone()
                .unwrap_or_else(|| "Task".to_string());
            props.insert("title".to_string(), Value::String(title));
        }
    }
    if let Some(assignee) = &properties.assignee {
        props.insert("assignee".to_string(), Value::String(assignee.clone()));
    }
    if let Some(description) = &properties.task_description {
        props.insert(
            "description".to_string(),
            Value::String(description.clone()),
        );
    }
    if let Some(options) = &properties.options {
        props.insert("options".to_string(), options.clone());
    }
    if let Some(schema) = &properties.input_schema {
        props.insert("input_schema".to_string(), schema.clone());
    }
    if let Some(data) = &properties.data {
        props.insert("data".to_string(), data.clone());
    }
    // Emitted RAW (number or template string): the human task handler
    // resolves it with every other property and coerces afterwards.
    if let Some(due) = &properties.due_in_seconds {
        props.insert("due_in_seconds".to_string(), due.to_value());
    }
    if let Some(priority) = &properties.priority {
        props.insert("priority".to_string(), priority.to_value());
    }
    if let Some(min_confidence) = properties.min_confidence {
        if let Some(num) = serde_json::Number::from_f64(min_confidence) {
            props.insert("min_confidence".to_string(), Value::Number(num));
        }
    }
    if let Some(escalation) = &properties.escalation_assignee {
        props.insert(
            "escalation_assignee".to_string(),
            Value::String(escalation.clone()),
        );
    }
    if let Some(timeout_edge) = &properties.timeout_edge {
        props.insert(
            "timeout_edge".to_string(),
            Value::String(timeout_edge.clone()),
        );
    }
    if let Some(exec_identity) = &properties.execution_identity {
        let identity_str = match exec_identity {
            ExecutionIdentityMode::Agent => "agent",
            ExecutionIdentityMode::Caller => "caller",
            ExecutionIdentityMode::Function => "function",
        };
        props.insert(
            "execution_identity".to_string(),
            Value::String(identity_str.to_string()),
        );
    }

    // Chat step configuration
    if let Some(chat_cfg) = &properties.chat_config {
        if let Some(agent_ref) = &chat_cfg.agent_ref {
            let path = agent_ref
                .raisin_path
                .as_ref()
                .cloned()
                .unwrap_or_else(|| agent_ref.raisin_ref.clone());
            props.insert("agent_ref".to_string(), Value::String(path));
            props.insert(
                "agent_workspace".to_string(),
                Value::String(agent_ref.raisin_workspace.clone()),
            );
        }
        if let Some(system_prompt) = &chat_cfg.system_prompt {
            props.insert(
                "system_prompt".to_string(),
                Value::String(system_prompt.clone()),
            );
        }
        props.insert(
            "max_turns".to_string(),
            Value::Number(chat_cfg.max_turns.into()),
        );
        if let Some(timeout) = chat_cfg.session_timeout_ms {
            props.insert(
                "session_timeout_ms".to_string(),
                Value::Number(timeout.into()),
            );
        }
        if !chat_cfg.handoff_targets.is_empty() {
            let arr: Vec<Value> = chat_cfg
                .handoff_targets
                .iter()
                .map(|t| {
                    let path = t
                        .agent_ref
                        .raisin_path
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| t.agent_ref.raisin_ref.clone());
                    serde_json::json!({
                        "agent_ref": path,
                        "description": t.description,
                        "condition": t.condition,
                        "trigger_phrases": t.trigger_phrases,
                    })
                })
                .collect();
            props.insert("handoff_targets".to_string(), Value::Array(arr));
        }
        let mut termination = serde_json::json!({
            "allow_user_end": chat_cfg.termination.allow_user_end,
            "allow_ai_end": chat_cfg.termination.allow_ai_end,
            "end_keywords": chat_cfg.termination.end_keywords,
        });
        if let Some(inactivity) = chat_cfg.termination.inactivity_timeout_ms {
            termination["inactivity_timeout_ms"] = Value::Number(inactivity.into());
        }
        props.insert("termination".to_string(), termination);
    }

    FlowNode {
        id: id.to_string(),
        step_type,
        properties: props,
        children: Vec::new(),
        next_node: None,
    }
}

/// Convert a designer container node to a runtime FlowNode, returning child edges
fn convert_container_node(
    id: &str,
    container_type: &DesignerContainerType,
    children: &[DesignerNode],
    ai_config: &Option<DesignerAiConfig>,
    timeout_ms: &Option<u64>,
    def: &DesignerFlowDefinition,
) -> (FlowNode, Vec<FlowEdge>) {
    let step_type = match container_type {
        DesignerContainerType::And | DesignerContainerType::Or => StepType::Container,
        DesignerContainerType::Parallel => StepType::Parallel,
        DesignerContainerType::AiSequence => StepType::AIContainer,
        // Competition and Loop containers are lowered inline by lower_node
        // and never reach this generic conversion
        DesignerContainerType::Competition => StepType::Competition,
        DesignerContainerType::Loop => StepType::Loop,
    };

    let mut props = HashMap::new();
    props.insert(
        "container_type".to_string(),
        Value::String(format!("{:?}", container_type).to_lowercase()),
    );

    // Add AI config if present
    if let Some(ai_cfg) = ai_config {
        let runtime_config = convert_ai_config(ai_cfg);
        props.insert(
            "ai_config".to_string(),
            serde_json::to_value(&runtime_config).unwrap_or_default(),
        );

        // Also add individual properties for easier access
        if let Some(agent_ref) = &ai_cfg.agent_ref {
            props.insert(
                "agent_ref".to_string(),
                serde_json::to_value(agent_ref).unwrap_or_default(),
            );
        }
        props.insert(
            "tool_mode".to_string(),
            Value::String(format!("{:?}", ai_cfg.tool_mode).to_lowercase()),
        );
        props.insert(
            "max_iterations".to_string(),
            Value::Number(ai_cfg.max_iterations.into()),
        );
        props.insert(
            "thinking_enabled".to_string(),
            Value::Bool(ai_cfg.thinking_enabled),
        );
        if let Some(timeout) = ai_cfg.timeout_ms {
            props.insert("timeout_ms".to_string(), Value::Number(timeout.into()));
        }
        if let Some(total_timeout) = ai_cfg.total_timeout_ms {
            props.insert(
                "total_timeout_ms".to_string(),
                Value::Number(total_timeout.into()),
            );
        }
    }

    if let Some(timeout) = timeout_ms {
        props.insert("timeout_ms".to_string(), Value::Number((*timeout).into()));
    }

    // Convert children recursively (kept attached for AI containers;
    // their internal chaining ends at the container's own continuation)
    let mut child_nodes = Vec::new();
    let mut child_edges = Vec::new();
    def.lower_nodes(children, "end", &mut child_nodes, &mut child_edges);

    let node = FlowNode {
        id: id.to_string(),
        step_type,
        properties: props,
        children: child_nodes,
        next_node: None,
    };

    (node, child_edges)
}

/// Convert designer AI config to runtime format
fn convert_ai_config(cfg: &DesignerAiConfig) -> AIContainerConfig {
    AIContainerConfig {
        agent_ref: cfg
            .agent_ref
            .as_ref()
            .map(|r| {
                r.raisin_path
                    .clone()
                    .unwrap_or_else(|| r.raisin_ref.clone())
            })
            .unwrap_or_default(),
        tool_mode: match cfg.tool_mode {
            DesignerToolMode::Auto => ToolMode::Auto,
            DesignerToolMode::Explicit => ToolMode::Explicit,
            DesignerToolMode::Hybrid => ToolMode::Hybrid,
        },
        explicit_tools: cfg.explicit_tools.clone(),
        max_iterations: cfg.max_iterations,
        conversation_ref: cfg.conversation_ref.as_ref().map(|r| {
            r.raisin_path
                .clone()
                .unwrap_or_else(|| r.raisin_ref.clone())
        }),
        execution: AiExecutionConfig {
            max_retries: cfg.max_retries.unwrap_or(2),
            retry_delay_ms: cfg.retry_delay_ms.unwrap_or(1000),
            timeout_ms: cfg.timeout_ms,
            thinking_enabled: cfg.thinking_enabled,
        },
        total_timeout_ms: cfg.total_timeout_ms,
        response_format: cfg.response_format.clone(),
        output_schema: cfg.output_schema.clone(),
    }
}
