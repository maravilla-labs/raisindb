// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Step configuration types for AI, decision, function, and human task steps

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Shared AI execution configuration for resilience and observability.
///
/// Used by both AI Container and Chat Step handlers to configure
/// retries, timeouts, and extended thinking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiExecutionConfig {
    /// Maximum retries on transient AI failures (default: 2)
    #[serde(default = "default_ai_max_retries")]
    pub max_retries: u32,

    /// Base delay between retries in milliseconds (exponential backoff, default: 1000ms)
    #[serde(default = "default_ai_retry_delay_ms")]
    pub retry_delay_ms: u64,

    /// Per-call timeout in milliseconds (default: 30000ms = 30s)
    #[serde(
        default = "default_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_ms: Option<u64>,

    /// Enable thinking/reasoning output
    #[serde(default)]
    pub thinking_enabled: bool,
}

impl Default for AiExecutionConfig {
    fn default() -> Self {
        Self {
            max_retries: default_ai_max_retries(),
            retry_delay_ms: default_ai_retry_delay_ms(),
            timeout_ms: default_timeout_ms(),
            thinking_enabled: false,
        }
    }
}

/// Configuration for AI container steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIContainerConfig {
    /// Reference to the agent node
    pub agent_ref: String,

    /// Tool execution mode
    #[serde(default)]
    pub tool_mode: ToolMode,

    /// Tools to expose as explicit steps (hybrid mode)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explicit_tools: Vec<String>,

    /// Maximum iterations
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,

    /// Existing conversation to continue
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_ref: Option<String>,

    /// Response format: "text", "json_object", or "json_schema"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<String>,

    /// JSON schema for structured output (when response_format = "json_schema")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,

    /// Shared AI execution config (retries, timeout, thinking)
    #[serde(default, flatten)]
    pub execution: AiExecutionConfig,

    /// Total execution timeout in milliseconds (default: 300000ms = 5min)
    #[serde(
        default = "default_total_timeout_ms",
        skip_serializing_if = "Option::is_none"
    )]
    pub total_timeout_ms: Option<u64>,
}

fn default_max_iterations() -> u32 {
    10
}

fn default_ai_max_retries() -> u32 {
    2
}

fn default_ai_retry_delay_ms() -> u64 {
    1000
}

fn default_timeout_ms() -> Option<u64> {
    Some(30000) // 30 seconds
}

fn default_total_timeout_ms() -> Option<u64> {
    Some(300000) // 5 minutes
}

/// Tool execution mode for AI containers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ToolMode {
    /// AI handles all tool calls internally
    #[default]
    Auto,

    /// All tool calls exposed as explicit steps
    Explicit,

    /// Some tools internal, some explicit
    Hybrid,
}

/// Configuration for decision steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionConfig {
    /// Condition expression (raisin-rel)
    pub condition: String,

    /// Node ID to go to if condition is true
    pub yes_node: String,

    /// Node ID to go to if condition is false
    pub no_node: String,
}

/// Configuration for function steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionStepConfig {
    /// Reference to the function node
    pub function_ref: String,

    /// Input mapping (how to map context to function input)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub input_mapping: HashMap<String, String>,

    /// Output mapping (how to map function output to context)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub output_mapping: HashMap<String, String>,

    /// Compensation function reference (for saga rollback)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation_ref: Option<String>,

    /// Max retries for this step
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,

    /// Timeout in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

fn default_max_retries() -> u32 {
    3
}

/// Configuration for human task steps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanTaskConfig {
    /// Task type
    pub task_type: TaskType,

    /// Task title
    pub title: String,

    /// Task description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Assignee path (user or role)
    pub assignee: String,

    /// Response options (for approval tasks)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<TaskOption>,

    /// Input schema (for input tasks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,

    /// Due date offset in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_in_seconds: Option<i64>,

    /// Priority (1-5, 5 being highest)
    #[serde(default = "default_priority")]
    pub priority: u8,
}

fn default_priority() -> u8 {
    3
}

/// Task type for human tasks.
///
/// The four named variants are the types the runtime understands
/// semantically; any other slug is carried through verbatim as
/// [`TaskType::Custom`] so an application can define its own task
/// vocabulary without an engine change. The slug is what lands in the
/// task node's `task_type` property and what a task UI dispatches on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskType {
    /// Approval decision
    Approval,

    /// User input required
    Input,

    /// Review task
    Review,

    /// Generic action
    Action,

    /// Application-defined task type, carried through verbatim
    Custom(String),
}

// Serialized as its bare slug (not a tagged enum), so `task_type: approval`
// and `task_type: seat_assignment` are both just strings on the wire and in
// flow YAML.
impl Serialize for TaskType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TaskType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let slug = String::deserialize(deserializer)?;
        TaskType::parse(&slug).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "invalid task_type '{}': expected a slug of 1-64 chars \
                 matching [a-z][a-z0-9_-]*",
                slug
            ))
        })
    }
}

impl TaskType {
    /// The slug form written to the task node and matched on by task UIs
    pub fn as_str(&self) -> &str {
        match self {
            TaskType::Approval => "approval",
            TaskType::Input => "input",
            TaskType::Review => "review",
            TaskType::Action => "action",
            TaskType::Custom(slug) => slug,
        }
    }

    /// Parse a task-type slug.
    ///
    /// Known slugs map to their named variant; anything else that is a
    /// valid slug becomes [`TaskType::Custom`]. Returns `None` for a slug
    /// that is not usable as a task type (see [`is_valid_task_type_slug`]).
    pub fn parse(slug: &str) -> Option<Self> {
        match slug {
            "approval" => Some(TaskType::Approval),
            "input" => Some(TaskType::Input),
            "review" => Some(TaskType::Review),
            "action" => Some(TaskType::Action),
            other if is_valid_task_type_slug(other) => Some(TaskType::Custom(other.to_string())),
            _ => None,
        }
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a string is usable as a task-type slug.
///
/// Task types are indexed and matched on as identifiers, so the shape is
/// constrained (length, and `[a-z0-9_-]` after a leading letter) even
/// though the *set* of values is open.
pub fn is_valid_task_type_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug.starts_with(|c: char| c.is_ascii_lowercase())
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Response option for human tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOption {
    /// Option value
    pub value: String,

    /// Display label
    pub label: String,

    /// Optional color/style
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}
