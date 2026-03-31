//! Flow execution context and runtime state
//!
//! The flow context follows the same pattern as function execution in RaisinDB.
//! When a flow is triggered by a node event:
//!
//! - `trigger_info` contains the event details (like `FunctionContext` in function execution)
//! - `input` contains the triggering node data (like `flow_input` in function execution)
//! - `step_outputs` contains outputs from completed steps (replaces raisin-rel templates)
//!
//! This allows steps to access:
//! - `context.input` - the node that triggered the flow
//! - `context.step_outputs["step-id"]` - output from a previous step

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::compensation::CompensationEntry;

/// Information about what triggered the flow
///
/// This mirrors `FunctionContext` from raisin-functions and contains
/// the event details that initiated flow execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerInfo {
    /// The type of event that triggered the flow
    pub event_type: TriggerEventType,

    /// The ID of the node that triggered the event
    pub node_id: String,

    /// The node type (e.g., "raisin:AIMessage")
    pub node_type: String,

    /// The workspace where the event occurred
    pub workspace: String,

    /// The path of the triggering node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,

    /// Tenant ID
    pub tenant_id: String,

    /// Repository ID
    pub repo_id: String,

    /// Branch name
    pub branch: String,
}

/// Type of event that triggered the flow
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerEventType {
    /// Node was created
    Created,
    /// Node was updated
    Updated,
    /// Node was deleted
    Deleted,
    /// Manual/API trigger
    Manual,
    /// Scheduled trigger
    Scheduled,
    /// Webhook trigger
    Webhook,
    /// Flow resumption (tool result, human task completion, etc.)
    Resume,
}

impl Default for TriggerEventType {
    fn default() -> Self {
        Self::Manual
    }
}

/// Runtime context for flow execution
///
/// Maintains the current state during flow execution, including variables,
/// input data, and current execution state.
///
/// # Data Access Pattern
///
/// Instead of raisin-rel template expressions like `{{ input.value }}`,
/// flows access data through the context:
///
/// ```text
/// context.input              -> The triggering node data
/// context.step_outputs       -> HashMap of step_id -> step output
/// context.variables          -> Mutable flow variables
/// ```
///
/// Each step receives a combined input similar to existing function flows:
/// ```json
/// {
///     "flow_input": <triggering node>,
///     "previous_results": { "step-1": <output>, "step-2": <output> }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowContext {
    /// Flow instance ID
    pub instance_id: String,

    /// Information about what triggered the flow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_info: Option<TriggerInfo>,

    /// Original input to the flow - the triggering node data (immutable)
    /// This is equivalent to `flow_input` in function execution
    pub input: Value,

    /// Outputs from completed steps, keyed by step ID
    /// This replaces raisin-rel template expressions for accessing previous results
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub step_outputs: HashMap<String, Value>,

    /// Flow-scoped variables (mutable)
    pub variables: HashMap<String, Value>,

    /// Current step output (transient)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_output: Option<Value>,

    /// Current error info (populated when following an error edge)
    /// Available as `$.error` in expressions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<FlowContextError>,

    /// Stack for nested contexts (e.g., parallel branches, sub-flows)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_stack: Vec<ContextFrame>,

    /// Compensation stack for saga pattern rollback
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compensation_stack: Vec<CompensationEntry>,
}

impl FlowContext {
    /// Create a new flow context with initial input
    pub fn new(instance_id: String, input: Value) -> Self {
        Self {
            instance_id,
            trigger_info: None,
            input,
            step_outputs: HashMap::new(),
            variables: HashMap::new(),
            current_output: None,
            error: None,
            context_stack: Vec::new(),
            compensation_stack: Vec::new(),
        }
    }

    /// Create a new flow context with trigger info
    pub fn with_trigger(instance_id: String, input: Value, trigger_info: TriggerInfo) -> Self {
        Self {
            instance_id,
            trigger_info: Some(trigger_info),
            input,
            step_outputs: HashMap::new(),
            variables: HashMap::new(),
            current_output: None,
            error: None,
            context_stack: Vec::new(),
            compensation_stack: Vec::new(),
        }
    }

    /// Record the output of a completed step
    pub fn record_step_output(&mut self, step_id: String, output: Value) {
        self.step_outputs.insert(step_id, output);
    }

    /// Get the output of a previous step
    pub fn get_step_output(&self, step_id: &str) -> Option<&Value> {
        self.step_outputs.get(step_id)
    }

    /// Build the input for a step (like existing function flow pattern)
    ///
    /// Returns a JSON object with:
    /// - `flow_input`: The triggering node data
    /// - `previous_results`: Outputs from completed steps
    pub fn build_step_input(&self) -> Value {
        serde_json::json!({
            "flow_input": self.input,
            "previous_results": self.step_outputs,
        })
    }

    /// Set a variable
    pub fn set_variable(&mut self, key: String, value: Value) {
        self.variables.insert(key, value);
    }

    /// Get a variable
    pub fn get_variable(&self, key: &str) -> Option<&Value> {
        self.variables.get(key)
    }

    /// Merge output into variables
    pub fn merge_output(&mut self, output: Value) {
        if let Value::Object(map) = output {
            for (key, value) in map {
                self.variables.insert(key, value);
            }
        }
    }

    /// Push a new context frame (for nested execution)
    pub fn push_frame(&mut self, frame: ContextFrame) {
        self.context_stack.push(frame);
    }

    /// Pop the current context frame
    pub fn pop_frame(&mut self) -> Option<ContextFrame> {
        self.context_stack.pop()
    }

    /// Get the current context depth
    pub fn depth(&self) -> usize {
        self.context_stack.len()
    }

    /// Push a compensation entry onto the stack
    pub fn push_compensation(&mut self, entry: CompensationEntry) {
        self.compensation_stack.push(entry);
    }

    /// Pop a compensation entry from the stack
    pub fn pop_compensation(&mut self) -> Option<CompensationEntry> {
        self.compensation_stack.pop()
    }

    /// Convert context to JSON object for evaluation
    ///
    /// This creates a flat structure where step outputs can be accessed:
    /// - `input` - the triggering node data
    /// - `steps.{step_id}` - output from a specific step
    /// - `{variable_name}` - flow variables
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("input".to_string(), self.input.clone());

        // Add step outputs under "steps" namespace
        if !self.step_outputs.is_empty() {
            obj.insert(
                "steps".to_string(),
                Value::Object(
                    self.step_outputs
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
            );
        }

        // Add variables directly (for backward compatibility with raisin-rel evaluation)
        for (key, value) in &self.variables {
            obj.insert(key.clone(), value.clone());
        }

        if let Some(output) = &self.current_output {
            obj.insert("output".to_string(), output.clone());
        }

        // Add error info if present (available as $.error in expressions)
        if let Some(error) = &self.error {
            obj.insert(
                "error".to_string(),
                serde_json::to_value(error).unwrap_or(Value::Null),
            );
        }

        Value::Object(obj)
    }

    /// Set error info (when following an error edge)
    pub fn set_error(&mut self, error: FlowContextError) {
        self.error = Some(error);
    }

    /// Clear error info
    pub fn clear_error(&mut self) {
        self.error = None;
    }
}

impl Default for FlowContext {
    fn default() -> Self {
        Self::new(String::new(), Value::Null)
    }
}

/// A context frame for nested execution (sub-flows, parallel branches)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFrame {
    /// Type of frame
    pub frame_type: FrameType,

    /// Local variables for this frame
    pub local_variables: HashMap<String, Value>,

    /// Additional frame-specific data
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl ContextFrame {
    /// Create a new context frame
    pub fn new(frame_type: FrameType) -> Self {
        Self {
            frame_type,
            local_variables: HashMap::new(),
            metadata: Value::Null,
        }
    }

    /// Create a parallel branch frame
    pub fn parallel_branch(branch_id: String) -> Self {
        let mut frame = Self::new(FrameType::ParallelBranch);
        frame.metadata = serde_json::json!({ "branch_id": branch_id });
        frame
    }

    /// Create a sub-flow frame
    pub fn sub_flow(sub_flow_id: String) -> Self {
        let mut frame = Self::new(FrameType::SubFlow);
        frame.metadata = serde_json::json!({ "sub_flow_id": sub_flow_id });
        frame
    }

    /// Create a loop iteration frame
    pub fn loop_iteration(iteration: u32) -> Self {
        let mut frame = Self::new(FrameType::LoopIteration);
        frame.metadata = serde_json::json!({ "iteration": iteration });
        frame
    }
}

/// Type of context frame
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameType {
    /// Parallel branch execution
    ParallelBranch,

    /// Sub-flow execution
    SubFlow,

    /// Loop iteration
    LoopIteration,

    /// Conditional branch
    ConditionalBranch,
}

/// Error information available in flow context when following an error edge
///
/// This makes error data available as `$.error` in expressions for error handler nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowContextError {
    /// Error type/code (e.g., "timeout", "validation_error", "permission_denied")
    pub error_type: String,

    /// Human-readable error message
    pub message: String,

    /// ID of the step that failed
    pub step_id: String,

    /// Optional stack trace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack_trace: Option<String>,

    /// Number of retries attempted before failure
    #[serde(default)]
    pub retries_attempted: u32,

    /// Original input that caused the failure
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_input: Option<Value>,
}
