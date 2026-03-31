//! Handler for checking pending system updates.

use axum::{
    extract::{Path, State},
    Json,
};
use raisin_core::system_updates::check_pending_updates;
use raisin_rocksdb::SystemUpdateRepositoryImpl;

use crate::{error::ApiError, state::AppState};

use super::types::PendingUpdatesResponse;

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
) -> Result<Json<PendingUpdatesResponse>, ApiError> {
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    // Create the system update repository
    let system_update_repo = SystemUpdateRepositoryImpl::new(rocksdb.db().clone());

    // Get the default branch for this repository
    let branch = "main"; // TODO: Get from repository config

    // Check for pending updates
    let summary = check_pending_updates(
        state.storage().clone(),
        &system_update_repo,
        &tenant_id,
        &repo_id,
        branch,
    )
    .await
    .map_err(|e| ApiError::internal(format!("Failed to check pending updates: {}", e)))?;

    Ok(Json(summary.into()))
}
