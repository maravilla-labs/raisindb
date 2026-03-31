//! Types and callback definitions for trigger evaluation.

use raisin_error::Result;
use std::sync::Arc;

/// Trigger match result
#[derive(Debug, Clone)]
pub struct TriggerMatch {
    /// Path to the function to execute (for single-function triggers)
    pub function_path: Option<String>,
    /// Name of the trigger that matched
    pub trigger_name: String,
    /// Priority of the trigger (higher = execute first)
    pub priority: i32,
    /// Path to the trigger node (for standalone raisin:Trigger nodes)
    pub trigger_path: Option<String>,
    /// Workflow data from referenced raisin:Flow node
    /// When present, a FlowInstanceExecution job is created
    pub workflow_data: Option<serde_json::Value>,
    /// Maximum retry attempts on failure (0 = no retries, None = use default of 3)
    pub max_retries: Option<u32>,
}

/// Result of a single filter check during trigger evaluation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FilterCheckResult {
    /// Name of the filter (e.g., "node_types", "paths", "property_filters")
    pub filter_name: String,
    /// Whether the filter check passed
    pub passed: bool,
    /// Expected value from the trigger filter configuration
    pub expected: Option<serde_json::Value>,
    /// Actual value from the node/event being evaluated
    pub actual: Option<serde_json::Value>,
    /// Human-readable explanation of why the filter passed or failed
    pub reason: String,
}

/// Result of evaluating a single trigger against an event
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerEvaluationResult {
    /// Path to the trigger (for standalone triggers) or function path (for inline triggers)
    pub trigger_path: String,
    /// Name of the trigger
    pub trigger_name: String,
    /// Whether the trigger matched the event
    pub matched: bool,
    /// Detailed results of each filter check
    pub filter_checks: Vec<FilterCheckResult>,
    /// Job ID of the enqueued job if the trigger matched
    pub enqueued_job_id: Option<String>,
}

/// Complete report of trigger evaluation for a node event
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerEvaluationReport {
    /// Information about the event that triggered the evaluation
    pub event: TriggerEventInfo,
    /// Total number of triggers evaluated
    pub triggers_evaluated: usize,
    /// Number of triggers that matched
    pub triggers_matched: usize,
    /// Detailed results for each trigger evaluated
    pub trigger_results: Vec<TriggerEvaluationResult>,
    /// Time taken to evaluate all triggers in milliseconds
    pub duration_ms: u64,
}

/// Information about the node event being evaluated
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TriggerEventInfo {
    /// Type of event (Created, Updated, Deleted, Published)
    pub event_type: String,
    /// ID of the node that triggered the event
    pub node_id: String,
    /// Type of the node (e.g., "raisin:Message")
    pub node_type: String,
    /// Path of the node in the content tree
    pub node_path: String,
    /// Workspace where the event occurred
    pub workspace: String,
    /// Node properties (if available)
    pub node_properties: Option<serde_json::Value>,
}

/// Callback type for finding matching triggers
///
/// This callback is provided by the storage layer to find triggers matching node events.
/// Returns both the matching triggers and detailed evaluation results for debugging.
/// Arguments: (event_type, node_id, node_type, node_path, tenant_id, repo_id, branch, workspace, node_properties)
/// Returns: (List of matching triggers, List of all evaluation results)
pub type TriggerMatcherCallback = Arc<
    dyn Fn(
            String,                    // event_type (Created, Updated, Deleted, Published)
            String,                    // node_id
            String,                    // node_type
            String,                    // node_path
            String,                    // tenant_id
            String,                    // repo_id
            String,                    // branch
            String,                    // workspace
            Option<serde_json::Value>, // node_properties
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<(Vec<TriggerMatch>, Vec<TriggerEvaluationResult>)>,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

/// Callback type for fetching node data
///
/// This callback is provided by the transport layer to fetch node data for function context.
/// Arguments: (node_id, tenant_id, repo_id, branch, workspace)
/// Returns: Node as JSON value (or None if not found)
pub type NodeFetcherCallback = Arc<
    dyn Fn(
            String, // node_id
            String, // tenant_id
            String, // repo_id
            String, // branch
            String, // workspace
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<serde_json::Value>>> + Send>,
        > + Send
        + Sync,
>;
