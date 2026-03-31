//! Step execution records and status tracking

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Record of a step execution within a flow
///
/// Stored as a child node of the FlowInstance, providing a complete audit trail
/// of all steps executed during the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStepExecution {
    /// Unique identifier for this execution record
    pub id: String,

    /// ID of the node in the flow definition that this represents
    pub node_id: String,

    /// Current status of this step
    pub status: StepStatus,

    /// Input provided to this step
    pub input: Value,

    /// Output produced by this step (if completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,

    /// Error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// When step execution started
    pub started_at: DateTime<Utc>,

    /// When step execution completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,

    /// Iteration number (for loops)
    #[serde(default)]
    pub iteration: u32,
}

impl FlowStepExecution {
    /// Create a new step execution record
    pub fn new(node_id: String, input: Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            node_id,
            status: StepStatus::Pending,
            input,
            output: None,
            error: None,
            started_at: Utc::now(),
            completed_at: None,
            iteration: 0,
        }
    }

    /// Mark the step as running
    pub fn start(&mut self) {
        self.status = StepStatus::Running;
        self.started_at = Utc::now();
    }

    /// Mark the step as completed with output
    pub fn complete(&mut self, output: Value) {
        self.status = StepStatus::Completed;
        self.output = Some(output);
        self.completed_at = Some(Utc::now());
    }

    /// Mark the step as failed with error
    pub fn fail(&mut self, error: String) {
        self.status = StepStatus::Failed;
        self.error = Some(error);
        self.completed_at = Some(Utc::now());
    }

    /// Mark the step as skipped
    pub fn skip(&mut self) {
        self.status = StepStatus::Skipped;
        self.completed_at = Some(Utc::now());
    }

    /// Get duration in milliseconds if completed
    pub fn duration_ms(&self) -> Option<i64> {
        self.completed_at
            .map(|completed| (completed - self.started_at).num_milliseconds())
    }
}

/// Execution status of a step
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Step is queued but not yet started
    Pending,

    /// Step is currently executing
    Running,

    /// Step completed successfully
    Completed,

    /// Step failed with error
    Failed,

    /// Step was skipped (e.g., condition branch not taken)
    Skipped,
}

impl StepStatus {
    /// Check if the step is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            StepStatus::Completed | StepStatus::Failed | StepStatus::Skipped
        )
    }

    /// Get the status as a string
    pub fn as_str(&self) -> &'static str {
        match self {
            StepStatus::Pending => "pending",
            StepStatus::Running => "running",
            StepStatus::Completed => "completed",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
        }
    }
}
