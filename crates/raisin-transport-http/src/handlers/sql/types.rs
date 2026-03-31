// SPDX-License-Identifier: BSL-1.1

//! Request and response types for SQL query endpoints.

use serde::{Deserialize, Serialize};

/// SQL query request
#[derive(Debug, Deserialize)]
pub struct SqlQueryRequest {
    /// SQL query to execute
    /// Workspace is specified in FROM clause: SELECT * FROM workspace_name
    /// Can include $1, $2, etc. placeholders for parameterized queries
    pub sql: String,

    /// Optional query parameters for parameterized queries
    /// Values will be safely substituted for $1, $2, etc. placeholders
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Vec<serde_json::Value>>,
}

/// SQL query response
///
/// For sync queries: contains data rows with actual results
/// For async bulk operations: contains single row with job_id, status, message columns
#[derive(Debug, Serialize)]
pub struct SqlQueryResponse {
    /// Column names in result set
    pub columns: Vec<String>,
    /// Result rows as JSON objects
    pub rows: Vec<serde_json::Value>,
    /// Total number of rows returned
    pub row_count: usize,
    /// Query execution time in milliseconds
    pub execution_time_ms: u64,
    /// Query plan (only present for EXPLAIN queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain_plan: Option<String>,
}
