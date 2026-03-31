// SPDX-License-Identifier: BSL-1.1

//! Request/response types for function management and invocation.
//!
//! Defines the JSON-serializable structures used across all function
//! HTTP endpoints: listing, invocation, execution history, file
//! execution, and flow execution.

use serde::{Deserialize, Serialize};

/// Request to invoke a function.
#[derive(Debug, Deserialize)]
pub struct InvokeFunctionRequest {
    /// Input data passed to the function.
    #[serde(default)]
    pub input: serde_json::Value,
    /// Whether to wait for result (sync) or enqueue background job.
    #[serde(default)]
    pub sync: bool,
    /// Optional timeout override in milliseconds.
    pub timeout_ms: Option<u64>,
    /// For async invocations: wait for job completion before responding.
    #[serde(default)]
    pub wait_for_completion: bool,
    /// Optional max wait time for async wait mode (milliseconds).
    pub wait_timeout_ms: Option<u64>,
}

/// Response from function invocation.
#[derive(Debug, Serialize)]
pub struct InvokeFunctionResponse {
    pub execution_id: String,
    pub sync: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waited: Option<bool>,
}

/// Function summary for listing.
#[derive(Debug, Serialize)]
pub struct FunctionSummary {
    pub path: String,
    pub name: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub language: String,
    pub enabled: bool,
    pub execution_mode: String,
    pub has_http_trigger: bool,
    pub has_event_triggers: bool,
    pub has_schedule_triggers: bool,
}

/// Detailed function information.
#[derive(Debug, Serialize)]
pub struct FunctionDetails {
    pub path: String,
    pub name: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub language: String,
    pub enabled: bool,
    pub execution_mode: String,
    /// Entry file in format `filename:function` (e.g., `index.js:handler`).
    pub entry_file: String,
    /// Deprecated: Use `entry_file` instead. Kept for backward compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_limits: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_policy: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggers: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Execution history entry.
#[derive(Debug, Serialize)]
pub struct ExecutionRecord {
    pub execution_id: String,
    pub function_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_name: Option<String>,
    pub status: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Result of inline (synchronous) function execution.
#[cfg(feature = "storage-rocksdb")]
#[derive(Debug, Serialize)]
pub(crate) struct InlineFunctionResult {
    pub execution_id: String,
    pub success: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub logs: Vec<String>,
}

/// Query parameters for listing functions.
#[derive(Debug, Deserialize, Default)]
pub struct ListFunctionsQuery {
    pub language: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub include_disabled: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Query parameters for listing executions.
#[derive(Debug, Deserialize, Default)]
pub struct ListExecutionsQuery {
    pub status: Option<String>,
    pub trigger_name: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Query parameters for function details.
#[derive(Debug, Deserialize, Default)]
pub struct GetFunctionQuery {
    #[serde(default)]
    pub include_code: bool,
}

// ============================================================================
// Direct File Execution Types
// ============================================================================

/// Request to run a JavaScript file directly.
#[derive(Debug, Deserialize)]
pub struct RunFileRequest {
    /// Node ID of the `raisin:Asset` containing JS code (optional if code is provided).
    #[serde(default)]
    pub node_id: Option<String>,
    /// Inline code to execute (used when file is unsaved in editor).
    #[serde(default)]
    pub code: Option<String>,
    /// File name for inline code (e.g., `index.js`) - used for validation.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Path to the parent `raisin:Function` node (for network_policy lookup with unsaved code).
    #[serde(default)]
    pub function_path: Option<String>,
    /// Name of the exported function to call (e.g., `handler`, `main`, `process`).
    pub handler: String,
    /// JSON input data (mutually exclusive with `input_node_id`).
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    /// Node ID to use as input (loads node and passes as JSON).
    #[serde(default)]
    pub input_node_id: Option<String>,
    /// Workspace to look up `input_node_id` from (defaults to `content`).
    #[serde(default)]
    pub input_workspace: Option<String>,
    /// Optional timeout override in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// SSE event types for file execution streaming.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunFileEvent {
    /// Execution started.
    Started {
        execution_id: String,
        file_name: String,
        handler: String,
    },
    /// Log entry from console.log/error/warn.
    Log {
        level: String,
        message: String,
        timestamp: String,
    },
    /// Execution completed with result.
    Result {
        execution_id: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        duration_ms: u64,
    },
    /// Stream complete.
    Done,
}

// ============================================================================
// Flow Execution Types
// ============================================================================

/// Request to execute a flow.
#[derive(Debug, Deserialize)]
pub struct RunFlowRequest {
    /// Path to the `raisin:Flow` node containing `workflow_data`.
    pub flow_path: String,
    /// Input data passed to the flow.
    #[serde(default)]
    pub input: serde_json::Value,
}

/// Response from flow execution.
#[derive(Debug, Serialize)]
pub struct RunFlowResponse {
    /// The created flow instance ID.
    pub instance_id: String,
    /// Job ID for tracking execution.
    pub job_id: String,
    /// Status (always `queued` for async execution).
    pub status: String,
}

/// Request to resume a paused flow instance.
#[derive(Debug, Deserialize)]
pub struct ResumeFlowRequest {
    /// Data to pass to the waiting step when resuming.
    #[serde(default)]
    pub resume_data: serde_json::Value,
}

/// Request to execute a flow in test mode.
#[derive(Debug, Deserialize)]
pub struct RunFlowTestRequest {
    /// Path to the `raisin:Flow` node containing `workflow_data`.
    pub flow_path: String,
    /// Input data passed to the flow.
    #[serde(default)]
    pub input: serde_json::Value,
    /// Test run configuration.
    #[serde(default)]
    pub test_config: raisin_flow_runtime::types::TestRunConfig,
}

/// Response from cancelling a flow instance.
#[derive(Debug, Serialize)]
pub struct CancelFlowInstanceResponse {
    /// Flow instance ID.
    pub id: String,
    /// New status (always "cancelled").
    pub status: String,
}

/// Response from GET flow instance status.
#[derive(Debug, Serialize)]
pub struct FlowInstanceStatusResponse {
    /// Flow instance ID.
    pub id: String,
    /// Current status (pending, running, waiting, completed, failed, etc.).
    pub status: String,
    /// Flow-scoped variables (mutable during execution).
    pub variables: serde_json::Value,
    /// Path to the flow definition.
    pub flow_path: String,
    /// When the flow was started.
    pub started_at: String,
    /// Error message (if status is failed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
