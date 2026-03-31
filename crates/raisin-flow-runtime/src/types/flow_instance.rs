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

    /// Custom metrics
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub custom: Value,
}
