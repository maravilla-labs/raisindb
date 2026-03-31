//! System updates HTTP handlers
//!
//! These endpoints manage built-in NodeType and Workspace updates for repositories.
//! They allow administrators to check for pending updates and apply them.

mod apply_nodetype;
mod apply_package;
mod apply_workspace;
mod get_pending;
mod types;

pub use get_pending::get_pending_updates;
pub use types::{
    ApplyUpdatesRequest, ApplyUpdatesResponse, PendingUpdateInfo, PendingUpdatesResponse,
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use raisin_core::nodetype_init::load_global_nodetypes_with_hashes;
use raisin_core::package_init::load_builtin_packages_with_hashes;
use raisin_core::system_updates::check_pending_updates;
use raisin_core::workspace_init::load_global_workspaces_with_hashes;
use raisin_rocksdb::SystemUpdateRepositoryImpl;
use raisin_storage::system_updates::ResourceType;

use crate::{error::ApiError, state::AppState};

/// Apply pending system updates to a repository
///
/// # Endpoint
/// POST /api/management/repositories/{tenant_id}/{repo_id}/system-updates/apply
///
/// # Request Body
/// ```json
/// {
///   "resources": ["raisin:Folder", "raisin:Page"],  // optional, empty = all
///   "force": false  // required true for breaking changes
/// }
/// ```
///
/// # Response
/// Returns the result of the apply operation. For large updates, this may
/// return a job ID for async tracking.
pub async fn apply_updates(
    State(state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    Json(req): Json<ApplyUpdatesRequest>,
) -> Result<(StatusCode, Json<ApplyUpdatesResponse>), ApiError> {
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let system_update_repo = SystemUpdateRepositoryImpl::new(rocksdb.db().clone());
    let branch = "main";

    // First, check for pending updates to identify what needs to be applied
    let summary = check_pending_updates(
        state.storage().clone(),
        &system_update_repo,
        &tenant_id,
        &repo_id,
        branch,
    )
    .await
    .map_err(|e| ApiError::internal(format!("Failed to check pending updates: {}", e)))?;

    // Filter updates based on request
    let updates_to_apply: Vec<_> = summary
        .updates
        .into_iter()
        .filter(|u| {
            if req.resources.is_empty() {
                true
            } else {
                req.resources.contains(&u.name)
            }
        })
        .collect();

    // Check for breaking changes if force is not set
    let has_breaking = updates_to_apply.iter().any(|u| u.is_breaking);
    if has_breaking && !req.force {
        return Err(ApiError::validation_failed(
            "Updates contain breaking changes. Set 'force: true' to apply anyway.",
        ));
    }

    let mut applied_count = 0;
    let mut skipped_count = 0;

    // Load all global definitions with hashes
    let nodetypes = load_global_nodetypes_with_hashes();
    let workspaces = load_global_workspaces_with_hashes();
    let packages = load_builtin_packages_with_hashes();

    // Apply each update
    for update in updates_to_apply {
        let applied = match update.resource_type {
            ResourceType::NodeType => {
                apply_nodetype::apply_nodetype_update(
                    &state,
                    &system_update_repo,
                    &tenant_id,
                    &repo_id,
                    branch,
                    &update,
                    &nodetypes,
                )
                .await?
            }
            ResourceType::Workspace => {
                apply_workspace::apply_workspace_update(
                    &state,
                    &system_update_repo,
                    &tenant_id,
                    &repo_id,
                    &update,
                    &workspaces,
                )
                .await?
            }
            ResourceType::Package => {
                apply_package::apply_package_update(
                    &state,
                    &system_update_repo,
                    rocksdb,
                    &tenant_id,
                    &repo_id,
                    branch,
                    &update,
                    &packages,
                )
                .await?
            }
        };
        if applied {
            applied_count += 1;
        } else {
            skipped_count += 1;
        }
    }

    let response = ApplyUpdatesResponse {
        job_id: None, // Synchronous for now
        message: format!(
            "Applied {} update(s), skipped {}",
            applied_count, skipped_count
        ),
        applied_count,
        skipped_count,
    };

    Ok((StatusCode::OK, Json(response)))
}
