//! Saga compensation pattern for flow rollback

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Entry in the compensation stack for saga-based rollback
///
/// Each step that completes successfully can push a compensation entry to the stack.
/// If the flow fails, compensations are executed in reverse order (LIFO) to undo
/// side effects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompensationEntry {
    /// ID of the step that was completed
    pub step_id: String,

    /// When the step was completed
    pub completed_at: DateTime<Utc>,

    /// Function reference to call for compensation/undo
    pub compensation_fn: String,

    /// Input data needed for the compensation function
    pub compensation_input: Value,

    /// Status of the compensation execution
    pub compensation_status: CompensationStatus,
}

impl CompensationEntry {
    /// Create a new compensation entry
    pub fn new(step_id: String, compensation_fn: String, compensation_input: Value) -> Self {
        Self {
            step_id,
            completed_at: Utc::now(),
            compensation_fn,
            compensation_input,
            compensation_status: CompensationStatus::Pending,
        }
    }

    /// Mark compensation as executed successfully
    pub fn mark_executed(&mut self) {
        self.compensation_status = CompensationStatus::Executed;
    }

    /// Mark compensation as failed
    pub fn mark_failed(&mut self, error: String) {
        self.compensation_status = CompensationStatus::Failed(error);
    }

    /// Check if compensation is pending
    pub fn is_pending(&self) -> bool {
        matches!(self.compensation_status, CompensationStatus::Pending)
    }
}

/// Status of a compensation execution
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum CompensationStatus {
    /// Compensation not yet executed
    Pending,

    /// Compensation executed successfully
    Executed,

    /// Compensation execution failed
    Failed(String),
}

impl CompensationStatus {
    /// Check if the compensation was executed (successfully or not)
    pub fn is_executed(&self) -> bool {
        !matches!(self, CompensationStatus::Pending)
    }

    /// Get error message if failed
    pub fn error(&self) -> Option<&str> {
        match self {
            CompensationStatus::Failed(err) => Some(err),
            _ => None,
        }
    }
}
