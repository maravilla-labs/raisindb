// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Core flow node types: FlowNode, FlowEdge, FlowMetadata, StepType

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Flow-level metadata
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlowMetadata {
    /// Flow name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Flow description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Flow version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// Custom metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom: HashMap<String, Value>,
}

/// A node in the flow definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    /// Unique node ID
    pub id: String,

    /// Type of step
    pub step_type: StepType,

    /// Node-specific properties
    #[serde(default)]
    pub properties: HashMap<String, Value>,

    /// Child nodes (for containers)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<FlowNode>,

    /// Next node ID (for simple sequential execution)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_node: Option<String>,
}

impl FlowNode {
    /// Get a property value
    pub fn get_property(&self, key: &str) -> Option<&Value> {
        self.properties.get(key)
    }

    /// Get a property as a string
    pub fn get_string_property(&self, key: &str) -> Option<String> {
        self.get_property(key)?.as_str().map(String::from)
    }

    /// Get a property as a string (alias for parallel handler)
    pub fn get_string(&self, key: &str) -> Option<String> {
        self.get_string_property(key)
    }

    /// Get a property as a boolean
    pub fn get_bool_property(&self, key: &str) -> Option<bool> {
        self.get_property(key)?.as_bool()
    }

    /// Get a property as an integer
    pub fn get_i64_property(&self, key: &str) -> Option<i64> {
        self.get_property(key)?.as_i64()
    }

    /// Get a property as an unsigned integer
    pub fn get_u32_property(&self, key: &str) -> Option<u32> {
        self.get_property(key)?
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
    }

    /// Get a property as an unsigned 64-bit integer
    pub fn get_u64_property(&self, key: &str) -> Option<u64> {
        self.get_property(key)?.as_u64()
    }

    /// Get a property as an array
    pub fn get_array(&self, key: &str) -> Option<&Vec<Value>> {
        self.get_property(key)?.as_array()
    }

    /// Get a property as an object/map
    pub fn get_object(&self, key: &str) -> Option<&serde_json::Map<String, Value>> {
        self.get_property(key)?.as_object()
    }
}

/// Edge connecting two nodes in the flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    /// Source node ID
    pub from: String,

    /// Target node ID
    pub to: String,

    /// Edge label (e.g., "yes", "no" for decision branches)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// Condition for this edge (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// Types of steps in a flow
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    /// Flow start node
    Start,

    /// Flow end node
    End,

    /// Decision/branching point
    Decision,

    /// Execute a raisin function
    FunctionStep,

    /// Single-shot AI agent call (no tool loop, no conversation persistence)
    #[serde(alias = "agent_step", alias = "ai_agent")]
    AgentStep,

    /// AI agent container (multi-iteration tool loop)
    #[serde(alias = "ai_container", alias = "ai_sequence")]
    AIContainer,

    /// Human task (approval, input, etc.)
    HumanTask,

    /// Parallel gateway (fork execution)
    Parallel,

    /// Join gateway (synchronize parallel branches)
    Join,

    /// Wait for time/event/condition
    Wait,

    /// Execute a sub-flow
    SubFlow,

    /// Interactive chat session with multi-turn conversation
    #[serde(alias = "chat_step", alias = "chat_session")]
    Chat,

    /// Generic container
    Container,

    /// Loop/iteration
    Loop,

    /// Custom step type
    Custom(String),
}

impl StepType {
    /// Check if this is a container type that can have children
    pub fn is_container(&self) -> bool {
        matches!(
            self,
            StepType::AIContainer | StepType::Parallel | StepType::Container | StepType::Loop
        )
    }

    /// Check if this is a branching type
    pub fn is_branching(&self) -> bool {
        matches!(self, StepType::Decision | StepType::Parallel)
    }
}
