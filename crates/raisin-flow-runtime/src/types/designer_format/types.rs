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
        /// AI router: an agent that decides which child runs when no REL
        /// rule matched (or as the only routing mechanism). OR containers
        /// only.
        #[serde(skip_serializing_if = "Option::is_none")]
        router: Option<DesignerRouterConfig>,
        /// Referee for competition containers: judges the competitors'
        /// answers, may request refinement rounds, declares confidence
        #[serde(skip_serializing_if = "Option::is_none")]
        referee: Option<DesignerRefereeConfig>,
        /// Loop configuration (only for loop containers)
        #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
        loop_config: Option<DesignerLoopConfig>,
        /// Fan-out configuration (parallel containers only): run the
        /// container's child subgraph once per item of a runtime collection
        /// and join all of the runs
        #[serde(rename = "fan_out", skip_serializing_if = "Option::is_none")]
        fan_out: Option<DesignerFanOutConfig>,
        /// Merge strategy for parallel containers
        /// (`merge_all` | `first_success` | `all_success`)
        #[serde(skip_serializing_if = "Option::is_none")]
        merge_strategy: Option<String>,
        /// Shared task prompt for competition containers (template
        /// expressions supported); children may override with their own
        /// `prompt`
        #[serde(skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
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

    // Decision step (step_type = decision, or any step with a `condition`)
    /// Node id to run when `condition` is true. Defaults to the next
    /// sibling, so a decision that only names `no_branch` reads as
    /// "skip ahead unless".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yes_branch: Option<String>,

    /// Node id to run when `condition` is false. Defaults to the next sibling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_branch: Option<String>,

    // Wait step (step_type = wait)
    /// What the step waits for: `delay` | `until` | `event` | `cron`
    /// (defaults to `delay`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_type: Option<String>,

    /// Duration for a `delay` wait, e.g. `"30m"`, `"1h"` (templates allowed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,

    /// Absolute timestamp for an `until` wait (templates allowed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,

    /// Event type for an `event` wait
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,

    /// Optional deadline for an `event` wait
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,

    /// Cron expression for a `cron` wait
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,

    // Sub-flow step (step_type = sub_flow)
    /// The flow to invoke, as a path or reference
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_ref: Option<RaisinReference>,

    /// Input mapping for the child flow (template expressions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_mapping: Option<serde_json::Value>,

    /// Run the child flow asynchronously
    #[serde(rename = "async", skip_serializing_if = "Option::is_none")]
    pub async_mode: Option<bool>,

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

    /// Mapping for the compensation input (template expressions; the
    /// forward step's output is available as output.*)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation_input_mapping: Option<serde_json::Value>,

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

    /// Function arguments - values support template expressions like
    /// "{{ input.value }}" or "${steps.prev.field}"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Value>,

    /// Prompt for ai_agent steps (template expressions supported)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Response format for ai_agent steps: "text", "json_object", or
    /// "json_schema" (structured output lands in structured_output)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<serde_json::Value>,

    /// Workflow-context injection for ai_agent steps: "input" | "full" |
    /// true (= full). Appends the workflow context as a JSON block to
    /// the prompt - templates remain the precise mechanism.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_context: Option<serde_json::Value>,

    // === Human task properties (step_type = human_task) ===
    /// Type of human task: approval | input | review | action
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,

    /// Assignee path - a user ("/users/alice") or an AI agent
    /// ("/agents/support"); agents decide tasks automatically
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,

    /// Task description shown to the assignee
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_description: Option<String>,

    /// Options for approval tasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,

    /// JSON schema for input tasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,

    /// Arbitrary structured payload for the task UI (raisin:InboxTask `data`
    /// slot). Template expressions inside it are resolved when the human_task
    /// step builds the task node, so a flow-created task can carry the same
    /// renderable payload a standalone `raisin.tasks.create` task does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,

    /// Task due time in seconds from creation (wait deadline).
    ///
    /// A literal number, or a template expression resolved per run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_in_seconds: Option<TemplatableNumber>,

    /// Task priority (1-5, where 5 is highest).
    ///
    /// A literal number, or a template expression resolved per run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<TemplatableNumber>,

    /// Minimum confidence for an agent assignee's decision (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f64>,

    /// Human assignee to escalate to when an agent assignee fails
    #[serde(skip_serializing_if = "Option::is_none")]
    pub escalation_assignee: Option<String>,

    /// Target node ID when the wait deadline expires
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_edge: Option<String>,
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
    /// Human task step (approval / input / review / action)
    HumanTask,
    /// Pause for time, an event, or a cron point
    Wait,
    /// Invoke another deployed flow as a step
    SubFlow,
    /// Two-way branch on a REL condition
    Decision,
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
///
/// Deserializes from either the full reference object
/// (`{"raisin:ref": "...", "raisin:workspace": "..."}`) or a plain path
/// string (hand-authored YAML convenience; workspace defaults to
/// "functions").
#[derive(Debug, Clone, Serialize)]
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

impl<'de> serde::Deserialize<'de> for RaisinReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct FullRef {
            #[serde(rename = "raisin:ref")]
            raisin_ref: String,
            #[serde(rename = "raisin:workspace", default = "default_ref_workspace")]
            raisin_workspace: String,
            #[serde(rename = "raisin:path", default)]
            raisin_path: Option<String>,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RefRepr {
            Path(String),
            Full(FullRef),
        }

        match RefRepr::deserialize(deserializer)? {
            RefRepr::Path(path) => Ok(RaisinReference {
                raisin_ref: path,
                raisin_workspace: default_ref_workspace(),
                raisin_path: None,
            }),
            RefRepr::Full(full) => Ok(RaisinReference {
                raisin_ref: full.raisin_ref,
                raisin_workspace: full.raisin_workspace,
                raisin_path: full.raisin_path,
            }),
        }
    }
}

fn default_ref_workspace() -> String {
    "functions".to_string()
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
    /// Competing agents: every child agent answers the same task, a
    /// referee picks the winner (with optional refinement rounds)
    Competition,
    /// Iterate the children once per item of a collection
    Loop,
}

/// Loop configuration for loop containers.
///
/// The container's children form the loop body and execute once per item
/// of the resolved collection. The current item is exposed as a flow
/// variable (default `item`), referenced in templates as `{{ item }}` /
/// `${item}` and in REL conditions as a bare identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// EXACTLY ONE of `over`, `while`, or `times` decides the loop's shape.
///
/// `over` was required, which made the runtime's `while` and `times` arms
/// unreachable from the format anyone actually authors — so "repeat until the
/// critic passes" could only be expressed by handing the loop an array of N
/// placeholder items and exiting early with `until`. All three are now
/// first-class; `validate` below refuses a config that names none or several,
/// because guessing which one an author meant is how a loop silently runs the
/// wrong number of times.
pub struct DesignerLoopConfig {
    /// Collection expression to iterate, e.g.
    /// `${steps.pick_candidates.candidates}` or `{{ input.items }}`.
    /// Must resolve to an array (objects iterate as
    /// `{key, value}` pairs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<String>,

    /// REL condition re-tested BEFORE each iteration; the loop runs while it
    /// holds, bounded by `max_iterations`.
    #[serde(rename = "while", default, skip_serializing_if = "Option::is_none")]
    pub while_condition: Option<String>,

    /// Fixed repeat count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<u32>,

    /// Variable name for the current item (default: `item`). Must be a
    /// REL identifier (`[A-Za-z0-9_]`, snake_case).
    #[serde(default = "default_loop_item")]
    pub item: String,

    /// Optional variable name for the current 0-based index
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,

    /// Safety cap: iterate at most this many items
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,

    /// `while` only: drop the iteration ceiling and run purely on the condition.
    ///
    /// Opt-in rather than the default for an absent `max_iterations`, because
    /// silently removing a safety limit is not something an author should get by
    /// omission. It is safe to offer at all only because the executor bounds a
    /// runaway itself, so a condition that never goes false costs a failed flow
    /// rather than a wedged worker.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unbounded: bool,

    /// Early-exit REL condition, evaluated after each completed
    /// iteration; when true the loop finishes with the results collected
    /// so far (e.g. `steps.ask.response == "accept"`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
}

impl DesignerLoopConfig {
    /// Which runtime `loop_type` this config lowers to.
    ///
    /// Whether the config is COHERENT is checked by `FlowDefinition::validate`
    /// on the lowered shape, not here — one implementation, and it covers
    /// hand-authored runtime format as well as this one.
    pub fn loop_type(&self) -> &'static str {
        if self.while_condition.is_some() {
            "while"
        } else if self.times.is_some() {
            "times"
        } else {
            "for_each"
        }
    }
}

fn default_loop_item() -> String {
    "item".to_string()
}

/// A numeric step property that may instead be a template expression.
///
/// The runtime resolves every human-task property against the flow context
/// and coerces the result, so a deadline or priority can come from data
/// (`"${input.sla_seconds}"`) rather than being authored into the flow. This
/// type exists because a plain `Option<i64>` made such a value a
/// DESERIALIZATION error — the whole flow definition failed to load with
/// `invalid type: string, expected i64`, so the property could not be
/// templated in designer format at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TemplatableNumber {
    /// A literal value
    Number(i64),
    /// A template expression (`${...}` / `{{ ... }}`), resolved per run
    Expression(String),
}

impl TemplatableNumber {
    /// The value as it should be handed to the runtime: a JSON number for a
    /// literal, the raw string for an expression (resolved and coerced later).
    pub fn to_value(&self) -> serde_json::Value {
        match self {
            TemplatableNumber::Number(n) => serde_json::Value::Number((*n).into()),
            TemplatableNumber::Expression(expr) => serde_json::Value::String(expr.clone()),
        }
    }

    /// Whether a string variant actually looks like a template.
    ///
    /// A non-template string here is almost certainly an authoring mistake
    /// (`"3 days"`), and it would otherwise only surface at run time.
    pub fn is_resolvable(&self) -> bool {
        match self {
            TemplatableNumber::Number(_) => true,
            TemplatableNumber::Expression(expr) => {
                expr.contains("${") || expr.contains("{{") || expr.trim().parse::<i64>().is_ok()
            }
        }
    }
}

/// Fan-out configuration for a parallel container.
///
/// Without it, a parallel container's children ARE its branches - a fixed
/// set authored in the designer. With it, the container has exactly one
/// child subgraph that runs once per item of a runtime collection, and the
/// container joins all of those runs. That is the difference between "these
/// three things happen at once" and "one of these happens per row", and the
/// latter is what a per-row human task needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignerFanOutConfig {
    /// Collection expression to fan out over, e.g.
    /// `${steps.plan.items}` or `{{ input.rows }}`. Must resolve to an
    /// array (objects iterate as `{key, value}` pairs) - the same forms the
    /// loop container's `over` accepts.
    pub over: String,

    /// Safety cap on the number of branches created. Fan-out width comes
    /// from runtime data, so it is bounded; defaults to the runtime's own
    /// cap when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_branches: Option<u32>,
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

/// Referee configuration for competition containers.
///
/// Every child agent answers the same task (possibly with different
/// LLMs); the referee judges the answers and either accepts a winner or
/// sends feedback for another round. The declared confidence travels in
/// the step output so downstream rules can gate on it (e.g. route
/// low-confidence results to a human task).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignerRefereeConfig {
    /// The referee agent
    pub agent_ref: RaisinReference,

    /// Judging instructions (template expressions supported); defaults to
    /// a generic "pick the best answer" prompt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Confidence the referee must declare to accept without further
    /// rounds (0-1, default 0.7)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f64>,

    /// Maximum refinement rounds after the initial one (default 1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<u32>,

    /// Workflow-context injection for the shared task: "input" | "full" |
    /// true (= full)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_context: Option<serde_json::Value>,
}

/// AI router configuration for OR containers: the referenced agent picks
/// the child to run. Deterministic REL rules always run first; the agent
/// decides only when none matched (or when there are no rules at all).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignerRouterConfig {
    /// The routing agent
    pub agent_ref: RaisinReference,

    /// Routing instructions shown to the agent (template expressions
    /// supported); defaults to a prompt built from the flow input
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Minimum confidence (0-1) required to follow the agent's choice;
    /// below it the router falls back to `default_branch` / skip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_confidence: Option<f64>,

    /// Child id to run when the agent is not confident or returns an
    /// invalid branch; unset = skip the container
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,

    /// Workflow-context injection: "input" | "full" | true (= full)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_context: Option<serde_json::Value>,
}
