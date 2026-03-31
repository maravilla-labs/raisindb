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

impl DesignerFlowDefinition {
    /// Convert designer format to runtime format.
    ///
    /// This flattens the tree structure and generates edges for navigation.
    /// Automatically injects implicit Start and End nodes since the designer
    /// UI shows these visually but doesn't save them to workflow_data.
    pub fn to_runtime_format(self) -> FlowDefinition {
        let mut runtime_nodes = Vec::new();
        let mut edges = Vec::new();

        // Convert each root node and collect edges
        let first_node_id = self.convert_nodes(&self.nodes, &mut runtime_nodes, &mut edges, None);

        // Inject implicit Start node (designer UI shows this visually but doesn't save it)
        let start_node = FlowNode {
            id: "__implicit_start__".to_string(),
            step_type: StepType::Start,
            properties: HashMap::new(),
            children: Vec::new(),
            next_node: first_node_id.clone(),
        };

        // Inject implicit End node
        let end_node = FlowNode {
            id: "__implicit_end__".to_string(),
            step_type: StepType::End,
            properties: HashMap::new(),
            children: Vec::new(),
            next_node: None,
        };

        // Add edge from Start to first real node
        if let Some(ref first_id) = first_node_id {
            edges.insert(
                0,
                FlowEdge {
                    from: "__implicit_start__".to_string(),
                    to: first_id.clone(),
                    label: None,
                    condition: None,
                },
            );
        }

        // Add edge from last real node to End
        if let Some(last_node) = runtime_nodes.last() {
            edges.push(FlowEdge {
                from: last_node.id.clone(),
                to: "__implicit_end__".to_string(),
                label: None,
                condition: None,
            });
        }

        // Build final node list: Start + converted nodes + End
        let mut all_nodes = vec![start_node];
        all_nodes.append(&mut runtime_nodes);
        all_nodes.push(end_node);

        // Note: node_index is built by the caller (from_workflow_data)
        FlowDefinition {
            nodes: all_nodes,
            edges,
            metadata: FlowMetadata::default(),
            node_index: None,
        }
    }

    /// Recursively convert designer nodes to runtime nodes.
    ///
    /// Returns the ID of the first node in this sequence (for edge generation).
    fn convert_nodes(
        &self,
        nodes: &[DesignerNode],
        output: &mut Vec<FlowNode>,
        edges: &mut Vec<FlowEdge>,
        _parent_id: Option<&str>,
    ) -> Option<String> {
        if nodes.is_empty() {
            return None;
        }

        let first_id = nodes.first().map(|n| n.id().to_string());
        let mut prev_id: Option<String> = None;

        for node in nodes {
            let node_id = node.id().to_string();

            // Create edge from previous node to this one (sequential flow)
            if let Some(prev) = prev_id.take() {
                edges.push(FlowEdge {
                    from: prev,
                    to: node_id.clone(),
                    label: None,
                    condition: None,
                });
            }

            match node {
                DesignerNode::Step {
                    id,
                    properties,
                    on_error,
                    error_edge: node_error_edge,
                } => {
                    let runtime_node =
                        convert_step_node(id, properties, on_error, node_error_edge, self);
                    output.push(runtime_node);
                }

                DesignerNode::Container {
                    id,
                    container_type,
                    children,
                    ai_config,
                    timeout_ms,
                    ..
                } => {
                    let (container_node, child_edges) = convert_container_node(
                        id,
                        container_type,
                        children,
                        ai_config,
                        timeout_ms,
                        self,
                    );
                    output.push(container_node);
                    edges.extend(child_edges);
                }
            }

            prev_id = Some(node_id);
        }

        first_id
    }

    /// Determine the runtime StepType based on designer properties
    pub(crate) fn determine_step_type(&self, props: &DesignerStepProperties) -> StepType {
        if props.function_ref.is_some() {
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
    if let Some(lua) = &properties.lua_script {
        props.insert("lua_script".to_string(), Value::String(lua.clone()));
    }
    if let Some(cond) = &properties.condition {
        props.insert("condition".to_string(), Value::String(cond.clone()));
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
    if properties.continue_on_fail {
        props.insert("continue_on_fail".to_string(), Value::Bool(true));
    }
    if properties.isolated_branch {
        props.insert("isolated_branch".to_string(), Value::Bool(true));
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
                    })
                })
                .collect();
            props.insert("handoff_targets".to_string(), Value::Array(arr));
        }
        props.insert(
            "termination".to_string(),
            serde_json::json!({
                "allow_user_end": chat_cfg.termination.allow_user_end,
                "allow_ai_end": chat_cfg.termination.allow_ai_end,
                "end_keywords": chat_cfg.termination.end_keywords,
            }),
        );
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

    // Convert children recursively
    let mut child_nodes = Vec::new();
    let mut child_edges = Vec::new();
    def.convert_nodes(children, &mut child_nodes, &mut child_edges, Some(id));

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
            max_retries: 2,
            retry_delay_ms: 1000,
            timeout_ms: cfg.timeout_ms,
            thinking_enabled: cfg.thinking_enabled,
        },
        total_timeout_ms: cfg.total_timeout_ms,
        response_format: None,
        output_schema: None,
    }
}
