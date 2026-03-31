// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow execution result types

use serde::{Deserialize, Serialize};

use super::enums::{FlowStatus, StepStatus};

/// Result of executing a flow step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step ID
    pub step_id: String,

    /// Overall status
    pub status: StepStatus,

    /// Results from each function in the step
    pub function_results: Vec<FunctionResult>,

    /// Execution duration in milliseconds
    pub duration_ms: u64,

    /// Error message if step failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of executing a single function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResult {
    /// Function path
    pub function_path: String,

    /// Execution ID for this function call
    pub execution_id: String,

    /// Whether the function succeeded
    pub success: bool,

    /// Function return value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,

    /// Error message if function failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

/// Overall flow execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowExecutionResult {
    /// Unique flow execution ID
    pub flow_execution_id: String,

    /// Trigger path that started this flow
    pub trigger_path: String,

    /// Overall status
    pub status: FlowStatus,

    /// When execution started
    pub started_at: chrono::DateTime<chrono::Utc>,

    /// When execution completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,

    /// Total duration in milliseconds
    pub duration_ms: u64,

    /// Results from each step
    pub step_results: Vec<StepResult>,

    /// Final aggregated output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_output: Option<serde_json::Value>,

    /// Error message if flow failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
