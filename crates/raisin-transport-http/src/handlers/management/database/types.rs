// SPDX-License-Identifier: BSL-1.1

//! Shared types for database management operations.

use axum::http::StatusCode;
use axum::response::Json;
use serde::{Deserialize, Serialize};

/// Query parameters for database operations.
#[derive(Debug, Deserialize)]
pub struct DatabaseOpQuery {
    /// Branch name (optional, defaults to default_branch from repo config)
    pub branch: Option<String>,

    /// Force regeneration even if dimensions match (default: false)
    #[serde(default)]
    pub force: bool,
}

/// Request body for reindex operation.
#[derive(Debug, Deserialize)]
pub struct ReindexRequest {
    /// Workspace to reindex
    pub workspace: String,

    /// Index types to rebuild: "all", "property", "reference", "child_order"
    pub index_types: Vec<String>,
}

/// Response containing a job ID.
#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub job_id: String,
    pub message: String,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Get branch name from query parameter or repository config.
#[cfg(feature = "storage-rocksdb")]
pub(super) async fn get_branch_name(
    state: &crate::state::AppState,
    tenant: &str,
    repo: &str,
    branch_param: Option<String>,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    // If branch is specified in query, use it
    if let Some(branch) = branch_param {
        return Ok(branch);
    }

    // Otherwise, try to get default_branch from repository config
    // For now, just use "main" as the default
    // TODO: Fetch from repository config when available
    tracing::debug!(
        "No branch specified for {}/{}, using default 'main'",
        tenant,
        repo
    );

    Ok("main".to_string())
}
