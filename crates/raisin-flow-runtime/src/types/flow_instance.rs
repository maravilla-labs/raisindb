//! Flow instance types representing running workflow executions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::compensation::CompensationEntry;
use super::test_config::TestRunConfig;

/// A running instance of a flow definition
///
/// This represents the execution state of a workflow, stored as a node in RaisinDB.
/// Each instance maintains its own state, variables, and execution history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowInstance {
    /// Unique identifier for this instance
    pub id: String,

    /// Instance version for optimistic concurrency control (OCC)
    ///
    /// This is incremented on every save to prevent concurrent modifications.
    /// Used by `save_instance_with_version` to detect conflicts.
    #[serde(default)]
    pub version: i32,

    /// Reference to the original flow definition node
    pub flow_ref: String,

    /// Version of the flow definition at creation time
    pub flow_version: i32,

    /// Complete workflow definition snapshot (immutable)
    ///
    /// This ensures that in-flight flows continue using the same definition
    /// even if the flow is updated.
    pub flow_definition_snapshot: Value,

    /// Current execution status
    pub status: FlowStatus,

    /// Current position in the flow
    pub current_node_id: String,

    /// Information about what the flow is waiting for (if status is Waiting)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_info: Option<WaitInfo>,

    /// Flow-scoped variables (mutable during execution)
    pub variables: Value,

    /// Initial input provided to the flow
    pub input: Value,

    /// Final output (populated when status is Completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,

    /// Stack of completed steps with compensation info for rollback
    #[serde(default)]
    pub compensation_stack: Vec<CompensationEntry>,

    /// Error message (populated when status is Failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Number of retry attempts for the current step
    #[serde(default)]
    pub retry_count: u32,

    /// Recorded positions, oldest first, when checkpointing is enabled.
    ///
    /// Bounded by the flow's own `checkpoints` setting; the oldest is dropped
    /// once the cap is reached, because a debugger wants the recent past.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<FlowCheckpoint>,

    /// When the flow execution started
    pub started_at: DateTime<Utc>,

    /// When the flow execution completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,

    /// Parent flow instance (for sub-flows)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_instance_ref: Option<String>,

    /// Execution metrics for monitoring
    #[serde(default)]
    pub metrics: FlowMetrics,

    /// Test run configuration (for test mode)
    ///
    /// When set, the flow runs in test mode with optional function mocking
    /// and isolated branch execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_config: Option<TestRunConfig>,
}

impl FlowInstance {
    /// The flow's checkpoint budget, from `metadata.custom.checkpoints`.
    ///
    /// `0` (the default, and anything unparseable) means checkpointing is off.
    /// Opt-in rather than always-on: a snapshot copies the whole variable bag,
    /// and a flow moving large payloads would pay that on every step.
    pub fn checkpoint_limit(definition: &Value) -> usize {
        definition
            .get("metadata")
            .and_then(|m| m.get("custom"))
            .and_then(|c| c.get("checkpoints"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize
    }

    /// Record the current position, dropping the oldest beyond `limit`.
    pub fn record_checkpoint(&mut self, limit: usize, visit: u64) {
        if limit == 0 {
            return;
        }
        let seq = self.checkpoints.last().map(|c| c.seq + 1).unwrap_or(0);
        self.checkpoints.push(FlowCheckpoint {
            seq,
            node_id: self.current_node_id.clone(),
            variables: self.variables.clone(),
            visit,
            wait_info: self.wait_info.clone(),
            recorded_at: Utc::now(),
        });
        while self.checkpoints.len() > limit {
            self.checkpoints.remove(0);
        }
    }

    /// Build a NEW instance positioned at one of this instance's checkpoints.
    ///
    /// **A fork, never a rewind.** The original is left exactly as it is,
    /// because its side effects already happened: an order was charged, mail
    /// went out, inbox tasks exist. Moving a live instance backwards would make
    /// the record disagree with the world and re-run every one of those on the
    /// way forward. Forking says the honest thing instead — here is a second
    /// run that begins from what the first knew at that moment — and leaves
    /// whether the side effects are safe to repeat as the caller's judgement,
    /// which is the only place that judgement can live.
    ///
    /// The fork carries the same definition snapshot, so it is the same flow;
    /// it starts `Pending` with a fresh version, no wait, no error and an empty
    /// compensation stack, since it has undone nothing and has nothing parked.
    ///
    /// **It does not inherit a parked inbox task, and must not.** A task is
    /// completed once, so two instances sharing one would race to consume a
    /// single human decision. Forking from a checkpoint that was waiting on a
    /// human therefore RE-ASKS: the fork re-executes that step and opens its own
    /// task, at its own path — which is safe because the path is derived from
    /// the whole instance id (see `generate_task_path`). Forking from a
    /// checkpoint taken AFTER the human answered inherits the answer instead,
    /// with no new task, because the response lives in the variable bag the
    /// checkpoint copied.
    pub fn fork_from_checkpoint(
        &self,
        seq: u32,
        new_id: impl Into<String>,
    ) -> Option<FlowInstance> {
        let checkpoint = self.checkpoints.iter().find(|c| c.seq == seq)?;
        Some(FlowInstance {
            id: new_id.into(),
            version: 1,
            flow_ref: self.flow_ref.clone(),
            flow_version: self.flow_version,
            flow_definition_snapshot: self.flow_definition_snapshot.clone(),
            status: FlowStatus::Pending,
            current_node_id: checkpoint.node_id.clone(),
            wait_info: None,
            variables: checkpoint.variables.clone(),
            input: self.input.clone(),
            output: None,
            compensation_stack: Vec::new(),
            error: None,
            retry_count: 0,
            checkpoints: Vec::new(),
            started_at: Utc::now(),
            completed_at: None,
            parent_instance_ref: None,
            metrics: Default::default(),
            test_config: self.test_config.clone(),
        })
    }

    /// Create a new flow instance
    pub fn new(
        flow_ref: String,
        flow_version: i32,
        flow_definition_snapshot: Value,
        input: Value,
        start_node_id: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            version: 1, // Start at version 1
            flow_ref,
            flow_version,
            flow_definition_snapshot,
            status: FlowStatus::Pending,
            current_node_id: start_node_id,
            wait_info: None,
            variables: Value::Object(Default::default()),
            input,
            output: None,
            compensation_stack: Vec::new(),
            error: None,
            retry_count: 0,
            checkpoints: Vec::new(),
            started_at: Utc::now(),
            completed_at: None,
            parent_instance_ref: None,
            metrics: FlowMetrics::default(),
            test_config: None,
        }
    }

    /// Create a new flow instance with test configuration
    pub fn new_test_run(
        flow_ref: String,
        flow_version: i32,
        flow_definition_snapshot: Value,
        input: Value,
        start_node_id: String,
        test_config: TestRunConfig,
    ) -> Self {
        let mut instance = Self::new(
            flow_ref,
            flow_version,
            flow_definition_snapshot,
            input,
            start_node_id,
        );
        instance.test_config = Some(test_config);
        instance
    }

    /// Check if this is a test run
    pub fn is_test_run(&self) -> bool {
        self.test_config
            .as_ref()
            .map(|c| c.is_test_run)
            .unwrap_or(false)
    }

    /// Get mock configuration for a function path
    pub fn get_function_mock(
        &self,
        function_path: &str,
    ) -> Option<&super::test_config::FunctionMock> {
        self.test_config.as_ref()?.get_mock(function_path)
    }

    /// Check if the flow is in a terminal state
    pub fn is_terminated(&self) -> bool {
        matches!(
            self.status,
            FlowStatus::Completed
                | FlowStatus::Failed
                | FlowStatus::Cancelled
                | FlowStatus::RolledBack
        )
    }

    /// Check if the flow can be resumed (not timed out)
    pub fn can_resume(&self) -> bool {
        if !matches!(self.status, FlowStatus::Waiting) {
            return false;
        }
        !self.is_timed_out()
    }

    /// Check if the flow's wait has timed out
    pub fn is_timed_out(&self) -> bool {
        self.wait_info
            .as_ref()
            .and_then(|w| w.timeout_at)
            .map(|timeout_at| Utc::now() > timeout_at)
            .unwrap_or(false)
    }
}

/// Execution status of a flow instance
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowStatus {
    /// Created but not yet started
    Pending,

    /// Currently executing
    Running,

    /// Paused, waiting for external event or input
    Waiting,

    /// Successfully completed
    Completed,

    /// Failed with error
    Failed,

    /// Manually cancelled
    Cancelled,

    /// Rolled back via saga compensation
    RolledBack,
}

impl FlowStatus {
    /// Get the status as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            FlowStatus::Pending => "pending",
            FlowStatus::Running => "running",
            FlowStatus::Waiting => "waiting",
            FlowStatus::Completed => "completed",
            FlowStatus::Failed => "failed",
            FlowStatus::Cancelled => "cancelled",
            FlowStatus::RolledBack => "rolled_back",
        }
    }
}

/// Information about what a flow instance is waiting for
///
/// This enables O(1) lookup when events occur (e.g., tool result arrives)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitInfo {
    /// Unique subscription ID for this wait
    pub subscription_id: String,

    /// Type of wait
    pub wait_type: WaitType,

    /// Path to the target node or resource being watched
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,

    /// Expected event type to match
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_event: Option<String>,

    /// When to automatically timeout if no response received
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_at: Option<DateTime<Utc>>,
}

/// Types of wait states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitType {
    /// Waiting for AI tool call results
    ToolCall,

    /// Waiting for human task completion
    HumanTask,

    /// Waiting for scheduled time
    Scheduled,

    /// Waiting for external event
    Event,

    /// Waiting for retry backoff
    Retry,

    /// Waiting for parallel branches to complete
    Join,

    /// Waiting for function execution to complete
    FunctionCall,

    /// Waiting for chat session user response
    ChatSession,
}

/// One recorded position in a flow's history: where it was, and what it knew.
///
/// This is the replay primitive. Before it, `current_node_id` plus a mutable
/// variable bag WAS the entire persisted position — there was no record of any
/// earlier state, so "what did the critic see on attempt 2" and "run this again
/// from before the bad decision" had no answer. Compensation is LIFO undo, which
/// unwinds side effects; it cannot put you back at a moment.
///
/// OFF BY DEFAULT, and opt-in per flow (`metadata.custom.checkpoints`). A
/// snapshot copies the whole variable bag, so a flow carrying large payloads
/// pays real bytes per step in a single persisted node. An agent graph being
/// debugged wants this; a publish flow moving 150 nodes does not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowCheckpoint {
    /// Monotonic sequence number within this instance, starting at 0.
    pub seq: u32,

    /// The node that was about to execute.
    pub node_id: String,

    /// The variable bag as it stood, including `step_outputs` and the `__*`
    /// bookkeeping, so a fork resumes with exactly what the original knew.
    pub variables: Value,

    /// Which visit of `node_id` this was (1 on the first) - the same counter
    /// `visits.<id>` exposes, denormalised so a checkpoint list reads as a
    /// history without resolving the bag.
    #[serde(default)]
    pub visit: u64,

    /// What the flow was waiting for at this point, if anything.
    ///
    /// Recorded so the history says WHY a position sat where it did — parked on
    /// whose inbox task, waiting for which event. Without it a checkpoint at a
    /// human task is indistinguishable from one at a function call, and the
    /// first question anyone asks of a stuck flow is what it was waiting on.
    ///
    /// A fork does NOT inherit it: see `fork_from_checkpoint`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_info: Option<WaitInfo>,

    pub recorded_at: DateTime<Utc>,
}

/// Execution metrics for monitoring and observability
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FlowMetrics {
    /// Total execution duration in milliseconds
    #[serde(default)]
    pub total_duration_ms: u64,

    /// Number of steps executed
    #[serde(default)]
    pub step_count: u32,

    /// Total number of retries across all steps
    #[serde(default)]
    pub retry_count: u32,

    /// Number of compensations executed
    #[serde(default)]
    pub compensation_count: u32,

    /// Number of AI iterations (for AI containers)
    #[serde(default)]
    pub ai_iteration_count: u32,

    /// Number of model calls whose usage was reported.
    ///
    /// Counted separately from the token totals because a provider that returns
    /// no `usage` block still costs a call: `ai_call_count > 0` with zero tokens
    /// means "the model ran and did not tell us", which is a different fact from
    /// "the model never ran" and the two used to be indistinguishable.
    #[serde(default)]
    pub ai_call_count: u32,

    /// Prompt tokens across every model call in this flow.
    #[serde(default)]
    pub total_input_tokens: u64,

    /// Completion tokens across every model call in this flow.
    #[serde(default)]
    pub total_output_tokens: u64,

    /// Custom metrics
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub custom: Value,
}
