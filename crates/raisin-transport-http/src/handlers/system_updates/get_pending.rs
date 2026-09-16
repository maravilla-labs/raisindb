//! Handler for checking pending system updates.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use raisin_core::system_updates::check_pending_updates;
use raisin_rocksdb::SystemUpdateRepositoryImpl;

use crate::{error::ApiError, state::AppState};

use super::types::{BranchQuery, PendingUpdatesResponse};

/// Check for pending system updates for a repository
///
/// # Endpoint
/// GET /api/management/repositories/{tenant_id}/{repo_id}/system-updates
///
/// # Response
/// Returns a summary of pending NodeType and Workspace updates, including
/// breaking change detection.
pub async fn get_pending_updates(
    State(state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
) -> Result<Json<PendingUpdatesResponse>, ApiError> {
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    // Create the system update repository
    let system_update_repo = SystemUpdateRepositoryImpl::new(rocksdb.db().clone());

    // Which branch to report on. The registry is per branch, so asking about
    // the repository alone is not a question with an answer.
    let branch = q.branch();

    // Check for pending updates against the live definition stack (embedded +
    // any overlay/registry layer above it).
    let definitions = state.definitions().await;
    let summary = check_pending_updates(
        state.storage().clone(),
        &system_update_repo,
        &definitions,
        &tenant_id,
        &repo_id,
        branch,
    )
    .await
    .map_err(|e| ApiError::internal(format!("Failed to check pending updates: {}", e)))?;

    Ok(Json(summary.into()))
}
