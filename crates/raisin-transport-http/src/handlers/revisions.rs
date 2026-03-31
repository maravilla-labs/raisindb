use axum::{
    extract::{Path, Query, State},
    Json,
};
use raisin_hlc::HLC;
use raisin_storage::{BranchRepository, RevisionMeta, RevisionRepository, Storage};
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, state::AppState};

/// Query parameters for listing revisions
#[derive(Debug, Deserialize)]
pub struct ListRevisionsQuery {
    /// Maximum number of revisions to return (default: 50)
    #[serde(default = "default_limit")]
    pub limit: usize,

    /// Number of revisions to skip for pagination (default: 0)
    #[serde(default)]
    pub offset: usize,

    /// Filter by branch name (optional - if not provided, shows all revisions)
    pub branch: Option<String>,

    /// Include system commits (default: false)
    #[serde(default)]
    pub include_system: bool,
}

fn default_limit() -> usize {
    50
}

/// Response for listing revisions
#[derive(Debug, Serialize)]
pub struct ListRevisionsResponse {
    pub revisions: Vec<RevisionMeta>,
    pub total: usize,
    pub has_more: bool,
}

/// List revisions for a repository (newest first)
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/revisions?limit=50&offset=0&branch=main&include_system=false
///
/// Returns paginated list of revisions with metadata.
/// If branch parameter is provided, only shows revisions from that branch (Git-like behavior).
pub async fn list_revisions(
    State(state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    Query(query): Query<ListRevisionsQuery>,
) -> Result<Json<ListRevisionsResponse>, ApiError> {
    let storage = state.connection().storage();
    let revisions_repo = storage.revisions();

    // Fetch one more than limit to check if there are more results
    let fetch_limit = query.limit + 1;

    let mut all_revisions = revisions_repo
        .list_revisions(&tenant_id, &repo_id, fetch_limit, query.offset)
        .await?;

    // Filter by branch if specified (Git-like: only show commits from this branch)
    if let Some(ref branch_name) = query.branch {
        all_revisions.retain(|r| &r.branch == branch_name);

        // Apply branch snapshot isolation:
        // For branches created from a revision, only show revisions up to the branch HEAD
        // This ensures branches show the correct snapshot they were created from
        if let Ok(Some(branch_info)) = storage
            .branches()
            .get_branch(&tenant_id, &repo_id, branch_name)
            .await
        {
            let max_revision = branch_info.head;
            all_revisions.retain(|r| r.revision <= max_revision);

            tracing::debug!(
                "Filtered revisions for branch '{}' to max_revision {} (branch HEAD)",
                branch_name,
                max_revision
            );
        }
    }

    // Filter out system commits if requested
    if !query.include_system {
        all_revisions.retain(|r| !r.is_system);
    }

    // Check if there are more results
    let has_more = all_revisions.len() > query.limit;

    // Trim to requested limit
    if all_revisions.len() > query.limit {
        all_revisions.truncate(query.limit);
    }

    Ok(Json(ListRevisionsResponse {
        total: all_revisions.len(),
        revisions: all_revisions,
        has_more,
    }))
}

/// Get single revision metadata
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/revisions/{revision}
///
/// Returns metadata for a specific revision.
pub async fn get_revision(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, revision_str)): Path<(String, String, String)>,
) -> Result<Json<RevisionMeta>, ApiError> {
    let storage = state.connection().storage();
    let revisions_repo = storage.revisions();

    let revision: HLC = revision_str
        .parse()
        .map_err(|e| ApiError::validation_failed(format!("Invalid revision: {}", e)))?;

    let meta = revisions_repo
        .get_revision_meta(&tenant_id, &repo_id, &revision)
        .await?
        .ok_or_else(|| ApiError::revision_not_found(&revision))?;

    Ok(Json(meta))
}

/// Get nodes changed in a specific revision
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/revisions/{revision}/changes
///
/// Returns list of node changes with operation types (added, modified, deleted).
pub async fn get_revision_changes(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, revision_str)): Path<(String, String, String)>,
) -> Result<Json<Vec<raisin_models::tree::NodeChange>>, ApiError> {
    let storage = state.connection().storage();
    let revisions_repo = storage.revisions();

    let revision: HLC = revision_str
        .parse()
        .map_err(|e| ApiError::validation_failed(format!("Invalid revision: {}", e)))?;

    let changed_nodes = revisions_repo
        .list_changed_nodes(&tenant_id, &repo_id, &revision)
        .await?;

    Ok(Json(changed_nodes))
}
