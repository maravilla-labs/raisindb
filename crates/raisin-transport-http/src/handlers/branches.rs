// SPDX-License-Identifier: BSL-1.1

//! HTTP handlers for Branch management
//!
//! Provides REST API endpoints for:
//! - Creating branches
//! - Listing branches
//! - Getting branch information
//! - Deleting branches
//! - Getting/updating branch HEAD

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use raisin_context::{Branch, BranchDivergence, MergeResult, MergeStrategy};
use raisin_hlc::HLC;
use raisin_storage::{BranchRepository, Storage};

use crate::{error::ApiError, state::AppState};

/// Request body for creating a branch
#[derive(serde::Deserialize)]
pub struct CreateBranchRequest {
    /// Name for the new branch
    pub name: String,
    /// Optional revision to branch from (None = create from scratch)
    pub from_revision: Option<HLC>,
    /// Optional upstream branch for divergence comparison
    pub upstream_branch: Option<String>,
    /// Actor creating the branch
    pub created_by: Option<String>,
    /// Whether the branch is protected from deletion
    #[serde(default)]
    pub protected: bool,
    /// Whether to include revision history from the source branch (default: true)
    /// When enabled, revision metadata is copied in a background job
    #[serde(default = "default_true")]
    pub include_revision_history: bool,
}

fn default_true() -> bool {
    true
}

/// Request body for updating branch HEAD
#[derive(serde::Deserialize)]
pub struct UpdateBranchHeadRequest {
    /// New HEAD revision number
    pub revision: HLC,
}

/// Request body for merging branches
#[derive(serde::Deserialize)]
pub struct MergeBranchRequest {
    /// Name of the source branch to merge from
    pub source_branch: String,
    /// Merge strategy to use
    pub strategy: MergeStrategy,
    /// Commit message for the merge
    pub message: String,
    /// Actor performing the merge
    pub actor: String,
}

/// Request body for resolving merge conflicts
#[derive(serde::Deserialize)]
pub struct ResolveMergeRequest {
    /// Name of the source branch being merged from
    pub source_branch: String,
    /// User's resolutions for each conflict
    pub resolutions: Vec<raisin_context::ConflictResolution>,
    /// Commit message for the merge
    pub message: String,
    /// Actor performing the merge
    pub actor: String,
}

/// Response for getting branch HEAD
#[derive(serde::Serialize)]
pub struct HeadResponse {
    /// Current HEAD revision number
    pub revision: HLC,
}

/// Create a new branch
///
/// POST /api/management/repositories/{tenant_id}/{repo_id}/branches
#[axum::debug_handler]
pub async fn create_branch(
    State(state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    Json(req): Json<CreateBranchRequest>,
) -> Result<(StatusCode, Json<Branch>), ApiError> {
    let created_by = req.created_by.as_deref().unwrap_or("system");

    let branch = state
        .storage()
        .branches()
        .create_branch(
            &tenant_id,
            &repo_id,
            &req.name,
            created_by,
            req.from_revision,
            req.upstream_branch,
            req.protected,
            req.include_revision_history,
        )
        .await?;

    Ok((StatusCode::CREATED, Json(branch)))
}

/// List all branches in a repository
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/branches
pub async fn list_branches(
    State(state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
) -> Result<Json<Vec<Branch>>, ApiError> {
    let branches = state
        .storage()
        .branches()
        .list_branches(&tenant_id, &repo_id)
        .await?;

    Ok(Json(branches))
}

/// Get a specific branch
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}
pub async fn get_branch(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, name)): Path<(String, String, String)>,
) -> Result<Json<Branch>, ApiError> {
    let branch = state
        .storage()
        .branches()
        .get_branch(&tenant_id, &repo_id, &name)
        .await?;

    match branch {
        Some(b) => Ok(Json(b)),
        None => Err(ApiError::branch_not_found(&name)),
    }
}

/// Delete a branch
///
/// DELETE /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}
pub async fn delete_branch(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, name)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    state
        .storage()
        .branches()
        .delete_branch(&tenant_id, &repo_id, &name)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Get the HEAD revision of a branch
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/head
pub async fn get_branch_head(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, name)): Path<(String, String, String)>,
) -> Result<Json<HeadResponse>, ApiError> {
    let branch = state
        .storage()
        .branches()
        .get_branch(&tenant_id, &repo_id, &name)
        .await?;

    match branch {
        Some(b) => Ok(Json(HeadResponse { revision: b.head })),
        None => Err(ApiError::branch_not_found(&name)),
    }
}

/// Update the HEAD revision of a branch
///
/// PUT /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/head
pub async fn update_branch_head(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, name)): Path<(String, String, String)>,
    Json(req): Json<UpdateBranchHeadRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .storage()
        .branches()
        .update_head(&tenant_id, &repo_id, &name, req.revision)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Compare two branches to calculate divergence (commits ahead/behind)
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/branches/{branch}/compare/{base_branch}
///
/// Returns the number of commits the branch is ahead and behind the base branch,
/// similar to GitHub's branch comparison feature.
#[cfg(feature = "storage-rocksdb")]
pub async fn compare_branches(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, branch, base_branch)): Path<(String, String, String, String)>,
) -> Result<Json<BranchDivergence>, ApiError> {
    // Access the concrete RocksDB storage to call calculate_divergence
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let divergence = rocksdb_storage
        .branches_impl()
        .calculate_divergence(&tenant_id, &repo_id, &branch, &base_branch)
        .await?;

    Ok(Json(divergence))
}

/// Merge a source branch into a target branch
///
/// POST /api/management/repositories/{tenant_id}/{repo_id}/branches/{target_branch}/merge
///
/// Performs a Git-like merge operation between two branches. Supports both fast-forward
/// and three-way merge strategies. Returns conflict information if the merge cannot be
/// completed automatically.
///
/// # Request Body
/// - `source_branch`: Name of the branch to merge from
/// - `strategy`: Merge strategy (FastForward or ThreeWay)
/// - `message`: Commit message for the merge
/// - `actor`: User performing the merge
///
/// # Response
/// Returns `MergeResult` containing:
/// - `success`: Whether the merge completed successfully
/// - `revision`: Revision number of the merge commit (if successful)
/// - `conflicts`: List of conflicts (if any)
/// - `fast_forward`: Whether this was a fast-forward merge
/// - `nodes_changed`: Number of nodes affected by the merge
#[cfg(feature = "storage-rocksdb")]
pub async fn merge_branches(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, target_branch)): Path<(String, String, String)>,
    Json(req): Json<MergeBranchRequest>,
) -> Result<Json<MergeResult>, ApiError> {
    // Access the concrete RocksDB storage to call merge_branches
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let result = rocksdb_storage
        .branches_impl()
        .merge_branches(
            &tenant_id,
            &repo_id,
            &target_branch,
            &req.source_branch,
            req.strategy,
            &req.message,
            &req.actor,
        )
        .await?;

    Ok(Json(result))
}

/// Resolve merge conflicts and complete the merge
///
/// POST /api/management/repositories/{tenant_id}/{repo_id}/branches/{target_branch}/resolve-merge
///
/// This endpoint is called after `merge_branches` has detected conflicts. The client
/// provides resolutions for all conflicts, and this endpoint creates the merge commit
/// with the resolved state.
///
/// # Request Body
/// - `source_branch`: Name of the branch being merged from
/// - `resolutions`: Array of conflict resolutions, each containing:
///   - `node_id`: ID of the conflicted node
///   - `resolution_type`: Type of resolution (KeepOurs, KeepTheirs, Manual)
///   - `resolved_properties`: The final properties for the node
/// - `message`: Commit message for the merge
/// - `actor`: User performing the merge resolution
///
/// # Response
/// Returns `MergeResult` containing:
/// - `success`: true (since conflicts were resolved)
/// - `revision`: Revision number of the merge commit
/// - `conflicts`: Empty array (all conflicts resolved)
/// - `fast_forward`: false (this is always a three-way merge)
/// - `nodes_changed`: Number of nodes affected by the merge
///
/// # Errors
/// - 404 if either branch doesn't exist
/// - 500 if the merge commit creation fails
#[cfg(feature = "storage-rocksdb")]
pub async fn resolve_merge_conflicts(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, target_branch)): Path<(String, String, String)>,
    Json(req): Json<ResolveMergeRequest>,
) -> Result<Json<MergeResult>, ApiError> {
    // Access the concrete RocksDB storage
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let result = rocksdb_storage
        .branches_impl()
        .resolve_merge_with_resolutions(
            &tenant_id,
            &repo_id,
            &target_branch,
            &req.source_branch,
            req.resolutions,
            &req.message,
            &req.actor,
        )
        .await?;

    Ok(Json(result))
}

/// Request body for setting upstream branch
#[derive(serde::Deserialize)]
pub struct SetUpstreamRequest {
    /// The upstream branch name, or null to clear
    pub upstream_branch: Option<String>,
}

/// Set the upstream branch for divergence comparison
///
/// PATCH /api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/upstream
///
/// Sets which branch this branch should be compared against for divergence calculation.
/// When upstream is None, the default branch (usually "main") is used for comparison.
#[cfg(feature = "storage-rocksdb")]
pub async fn set_upstream_branch(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, name)): Path<(String, String, String)>,
    Json(req): Json<SetUpstreamRequest>,
) -> Result<Json<Branch>, ApiError> {
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let branch = rocksdb_storage
        .branches_impl()
        .set_upstream_branch(&tenant_id, &repo_id, &name, req.upstream_branch.as_deref())
        .await?;

    Ok(Json(branch))
}
