//! Flow execution result types and error definitions

use serde_json::Value;
use thiserror::Error;

/// Result of a step execution
#[derive(Debug, Clone)]
pub enum StepResult {
    /// Continue to next step synchronously
    Continue {
        /// ID of the next node to execute
        next_node_id: String,
        /// Output from this step
        output: Value,
    },
    /// Pause execution, waiting for external event
    Wait {
        /// Reason for waiting
        reason: String,
        /// Additional metadata about what we're waiting for
        metadata: Value,
    },
    /// Re-execute the same step (used for internal loops like AI agent iterations)
    SameStep {
        /// Metadata about the re-execution
        metadata: Value,
    },
    /// Flow completed successfully
    Complete {
        /// Final output of the flow
        output: Value,
    },
    /// Step execution failed
    Error {
        /// Error that occurred
        error: FlowError,
    },
}

/// Flow execution errors
#[derive(Debug, Error, Clone)]
pub enum FlowError {
    /// Flow definition is invalid or malformed
    #[error("Invalid flow definition: {0}")]
    InvalidDefinition(String),

    /// Step not found in flow definition
    #[error("Step not found: {0}")]
    StepNotFound(String),

    /// Node not found in database
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// Failed to evaluate condition
    #[error("Condition evaluation failed: {0}")]
    ConditionEvaluation(String),

    /// Function execution failed
    #[error("Function execution failed: {0}")]
    FunctionExecution(String),

    /// AI provider error
    #[error("AI provider error: {0}")]
    AIProvider(String),

    /// Maximum iterations exceeded
    #[error("Maximum iterations exceeded: {limit}")]
    MaxIterationsExceeded {
        /// The iteration limit that was exceeded
        limit: u32,
    },

    /// Timeout exceeded
    #[error("Timeout exceeded after {duration_ms}ms")]
    TimeoutExceeded {
        /// Duration in milliseconds before timeout
        duration_ms: u64,
    },

    /// Version conflict (optimistic concurrency control)
    #[error("Version conflict: instance was modified by another process")]
    VersionConflict,

    /// Compensation execution failed
    #[error("Compensation failed for step {step_id}: {error}")]
    CompensationFailed {
        /// ID of the step whose compensation failed
        step_id: String,
        /// Error message
        error: String,
    },

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Invalid state transition
    #[error("Invalid state transition from {from} to {to}")]
    InvalidStateTransition {
        /// Source state
        from: String,
        /// Target state
        to: String,
    },

    /// Flow instance is already completed or cancelled
    #[error("Flow instance is already {status}")]
    AlreadyTerminated {
        /// The terminal status
        status: String,
    },

    /// Required property missing
    #[error("Missing required property: {0}")]
    MissingProperty(String),

    /// Generic error
    #[error("Flow error: {0}")]
    Other(String),

    /// Child flow creation or execution error
    #[error("Child flow error: {0}")]
    ChildFlowError(String),

    /// All child flows failed
    #[error("All child flows failed")]
    AllChildFlowsFailed,

    /// Parallel execution error with details
    #[error("Parallel execution error: {0}")]
    ParallelExecutionError(String),

    /// Invalid node configuration
    #[error("Invalid node configuration: {0}")]
    InvalidNodeConfiguration(String),

    /// Feature not supported by this backend
    #[error("Not supported: {0}")]
    NotSupported(String),

    /// Merge conflict during isolated branch merge
    #[error("Merge conflict in isolated branch '{branch_name}': {details}")]
    MergeConflict {
        /// Name of the branch with conflicts
        branch_name: String,
        /// Details about the conflict
        details: String,
    },

    /// Branch operation failed
    #[error("Branch operation failed: {0}")]
    BranchOperationFailed(String),

    /// Permission denied for operation
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// Security policy violation
    #[error("Security policy violation: {0}")]
    SecurityViolation(String),
}

impl From<serde_json::Error> for FlowError {
    fn from(err: serde_json::Error) -> Self {
        FlowError::Serialization(err.to_string())
    }
}

/// Result type for flow operations
pub type FlowResult<T> = Result<T, FlowError>;
