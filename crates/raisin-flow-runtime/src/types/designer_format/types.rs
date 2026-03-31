// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Core designer format types
//!
//! Type definitions for the flow designer UI format including nodes,
//! steps, containers, and their properties.

use serde::{Deserialize, Serialize};

/// Designer flow definition - the format produced by the raisin-flow-designer UI.
///
/// This is a tree-based structure where:
/// - Nodes use `node_type` field (not `step_type`)
/// - Containers have `children` arrays
/// - No explicit `edges` array (navigation is implicit in tree structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignerFlowDefinition {
    /// Schema version (currently 1)
    #[serde(default = "default_version")]
    pub version: u32,

    /// Error handling strategy
    #[serde(default)]
    pub error_strategy: DesignerErrorStrategy,

    /// Global timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// Root workflow nodes
    pub nodes: Vec<DesignerNode>,
}

fn default_version() -> u32 {
    1
}

/// Error strategy enum for designer format
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignerErrorStrategy {
    /// Stop workflow on first error
    #[default]
    FailFast,
    /// Continue execution even if a step fails
    Continue,
}

/// A node in the designer format - either a step or a container.
///
/// Uses serde's `tag` attribute to discriminate based on `node_type` field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "node_type")]
pub enum DesignerNode {
    /// A basic workflow step (function call, AI agent, etc.)
    #[serde(rename = "raisin:FlowStep")]
    Step {
        /// Unique node ID
        id: String,
        /// Step properties (action, function_ref, etc.)
        #[serde(default)]
        properties: DesignerStepProperties,
        /// Error handling behavior for this step
        #[serde(skip_serializing_if = "Option::is_none")]
        on_error: Option<StepErrorBehavior>,
        /// Target node ID for error flow (error edge at node level)
        #[serde(skip_serializing_if = "Option::is_none")]
        error_edge: Option<String>,
    },

    /// A container node (AND, OR, Parallel, AI Sequence)
    #[serde(rename = "raisin:FlowContainer")]
    Container {
        /// Unique node ID
        id: String,
        /// Type of container
        container_type: DesignerContainerType,
        /// Child nodes (nested)
        #[serde(default)]
        children: Vec<DesignerNode>,
        /// AI configuration (only for ai_sequence containers)
        #[serde(skip_serializing_if = "Option::is_none")]
        ai_config: Option<super::config_types::DesignerAiConfig>,
        /// Container rules for conditional routing
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rules: Vec<DesignerContainerRule>,
        /// Container timeout in milliseconds
        #[serde(skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
}

impl DesignerNode {
    /// Get the node ID
    pub fn id(&self) -> &str {
        match self {
            DesignerNode::Step { id, .. } => id,
            DesignerNode::Container { id, .. } => id,
        }
    }
}

/// Step properties in designer format
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DesignerStepProperties {
    /// Action name/label displayed in the designer
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,

    /// Reference to a RaisinDB function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_ref: Option<RaisinReference>,

    /// Reference to an AI agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_ref: Option<RaisinReference>,

    /// Lua script for evaluation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lua_script: Option<String>,

    /// Condition expression (REL)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,

    /// Key for payload data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_key: Option<String>,

    /// Whether step is disabled
    #[serde(default)]
    pub disabled: bool,

    /// Step type - distinguishes AI agent steps from regular steps
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_type: Option<DesignerStepType>,

    /// Retry configuration for this step
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryConfig>,

    /// Retry strategy preset
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_strategy: Option<String>,

    /// Step timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// Target node ID for error flow (error edge)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_edge: Option<String>,

    /// Reference to compensation function for saga rollback
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation_ref: Option<RaisinReference>,

    /// Continue workflow on step failure
    #[serde(default)]
    pub continue_on_fail: bool,

    /// Execute step in isolated git-like branch for safety
    #[serde(default)]
    pub isolated_branch: bool,

    /// Execution identity mode for permission handling (FR-028)
    /// - agent: Use AI agent's service account identity
    /// - caller: Use triggering user's identity for attribution
    /// - function: Use elevated function service account (delegation)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_identity: Option<ExecutionIdentityMode>,

    /// Chat step configuration (for chat step type)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_config: Option<super::config_types::ChatStepConfig>,
}

/// Execution identity mode for step execution (FR-028)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionIdentityMode {
    /// Use AI agent's service account identity
    #[default]
    Agent,
    /// Use triggering user's identity for attribution
    Caller,
    /// Use elevated function service account (delegation)
    Function,
}

/// Designer step type (different from runtime StepType)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignerStepType {
    /// Default function step
    Default,
    /// AI agent step
    AiAgent,
    /// Interactive chat session step
    Chat,
}

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Base delay in milliseconds for exponential backoff
    pub base_delay_ms: u64,
    /// Maximum delay cap in milliseconds
    pub max_delay_ms: u64,
}

/// RaisinDB reference type for cross-node references
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaisinReference {
    /// Node ID or path
    #[serde(rename = "raisin:ref")]
    pub raisin_ref: String,

    /// Workspace context
    #[serde(rename = "raisin:workspace")]
    pub raisin_workspace: String,

    /// Optional resolved path
    #[serde(rename = "raisin:path", skip_serializing_if = "Option::is_none")]
    pub raisin_path: Option<String>,
}

/// Container types matching the designer UI
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignerContainerType {
    /// All children must complete successfully
    And,
    /// Any child succeeding is sufficient
    Or,
    /// Execute children concurrently
    Parallel,
    /// AI-orchestrated execution with tool calls
    AiSequence,
}

/// Step error behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepErrorBehavior {
    /// Stop workflow on error
    Stop,
    /// Skip the step and continue
    Skip,
    /// Continue with next step
    Continue,
}

/// Container rule for conditional routing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignerContainerRule {
    /// Lua condition expression
    pub condition: String,
    /// ID of next step if condition matches
    pub next_step: String,
}
