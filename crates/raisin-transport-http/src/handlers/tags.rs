// SPDX-License-Identifier: BSL-1.1

//! HTTP handlers for Tag management
//!
//! Provides REST API endpoints for:
//! - Creating tags
//! - Listing tags
//! - Getting tag information
//! - Deleting tags

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use raisin_context::Tag;
use raisin_hlc::HLC;
use raisin_storage::{Storage, TagRepository};

use crate::{error::ApiError, state::AppState};

/// Request body for creating a tag
#[derive(serde::Deserialize)]
pub struct CreateTagRequest {
    /// Name for the new tag (e.g., "v1.0.0")
    pub name: String,
    /// Revision number this tag points to
    pub revision: HLC,
    /// Actor who created the tag
    pub created_by: Option<String>,
    /// Optional annotation message
    pub message: Option<String>,
    /// Whether the tag is protected from deletion
    #[serde(default)]
    pub protected: bool,
}

/// Create a new tag
///
/// POST /api/management/repositories/{tenant_id}/{repo_id}/tags
#[axum::debug_handler]
pub async fn create_tag(
    State(state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    Json(req): Json<CreateTagRequest>,
) -> Result<(StatusCode, Json<Tag>), ApiError> {
    let created_by = req.created_by.as_deref().unwrap_or("system");

    let tag = state
        .storage()
        .tags()
        .create_tag(
            &tenant_id,
            &repo_id,
            &req.name,
            &req.revision,
            created_by,
            req.message,
            req.protected,
        )
        .await?;

    Ok((StatusCode::CREATED, Json(tag)))
}

/// List all tags in a repository
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/tags
pub async fn list_tags(
    State(state): State<AppState>,
    Path((tenant_id, repo_id)): Path<(String, String)>,
) -> Result<Json<Vec<Tag>>, ApiError> {
    let tags = state
        .storage()
        .tags()
        .list_tags(&tenant_id, &repo_id)
        .await?;

    Ok(Json(tags))
}

/// Get a specific tag
///
/// GET /api/management/repositories/{tenant_id}/{repo_id}/tags/{name}
pub async fn get_tag(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, name)): Path<(String, String, String)>,
) -> Result<Json<Tag>, ApiError> {
    let tag = state
        .storage()
        .tags()
        .get_tag(&tenant_id, &repo_id, &name)
        .await?
        .ok_or_else(|| ApiError::tag_not_found(&name))?;

    Ok(Json(tag))
}

/// Delete a tag
///
/// DELETE /api/management/repositories/{tenant_id}/{repo_id}/tags/{name}
pub async fn delete_tag(
    State(state): State<AppState>,
    Path((tenant_id, repo_id, name)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .storage()
        .tags()
        .delete_tag(&tenant_id, &repo_id, &name)
        .await?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::tag_not_found(&name))
    }
}
