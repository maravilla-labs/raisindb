// SPDX-License-Identifier: BSL-1.1

//! Data types for webhook and trigger invocation.

use serde::{Deserialize, Serialize};

pub(in crate::handlers::webhooks) const TENANT_ID: &str = "default";
pub(in crate::handlers::webhooks) const DEFAULT_BRANCH: &str = "main";
pub(in crate::handlers::webhooks) const FUNCTIONS_WORKSPACE: &str = "functions";

/// Query parameters for webhook/trigger invocation
#[derive(Debug, Deserialize, Default)]
pub struct InvokeQuery {
    /// If true, wait for execution to complete and return result
    /// If false (default), return immediately with job_id
    #[serde(default)]
    pub sync: Option<bool>,
}

/// Response from webhook/trigger invocation
#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    /// Unique execution ID
    pub execution_id: String,
    /// Execution status: "queued" (async) or "completed"/"failed" (sync)
    pub status: String,
    /// Result from function execution (only for sync mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error message if execution failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Job ID for tracking async execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Execution duration in milliseconds (only for sync mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Function logs (only for sync mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<Vec<String>>,
}

/// Enum for different trigger lookup methods
pub(super) enum TriggerLookup {
    /// Look up by auto-generated webhook_id (nanoid)
    ByWebhookId(String),
    /// Look up by unique trigger name
    ByName(String),
}
