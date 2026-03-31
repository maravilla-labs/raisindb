// SPDX-License-Identifier: BSL-1.1

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use raisin_models as models;
use raisin_models::timestamp::StorageTimestamp;

use crate::{
    error::ApiError,
    state::AppState,
    types::{Page, PageMeta, PageParams},
};

pub async fn list_workspaces(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    axum::extract::Query(p): axum::extract::Query<PageParams>,
) -> Result<Json<Page<models::workspace::Workspace>>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from auth/middleware
    let mut items = state.ws_svc.list(tenant_id, &repo).await?;
    // stable order for pagination: sort by name
    items.sort_by(|a, b| a.name.cmp(&b.name));
    let offset = p.offset.unwrap_or(0);
    let limit = p.limit.unwrap_or(usize::MAX);
    let start = offset.min(items.len());
    let end = (start.saturating_add(limit)).min(items.len());
    let total = items.len();
    let slice = items[start..end].to_vec();
    let next_offset = if end < total { Some(end) } else { None };
    Ok(Json(Page {
        items: slice,
        page: PageMeta {
            total,
            limit,
            offset,
            next_offset,
        },
    }))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
) -> Result<Json<models::workspace::Workspace>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from auth/middleware
    let ws = state
        .ws_svc
        .get(tenant_id, &repo, &name)
        .await?
        .ok_or_else(|| ApiError::workspace_not_found(&name))?;
    Ok(Json(ws))
}

pub async fn put_workspace(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
    Json(mut ws): Json<models::workspace::Workspace>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default"; // TODO: Extract from auth/middleware
    ws.name = name;
    state.ws_svc.put(tenant_id, &repo, ws).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get workspace configuration
///
/// GET /api/workspaces/{repo}/{name}/config
pub async fn get_workspace_config(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
) -> Result<Json<models::workspace::WorkspaceConfig>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from auth/middleware
    let ws = state
        .ws_svc
        .get(tenant_id, &repo, &name)
        .await?
        .ok_or_else(|| ApiError::workspace_not_found(&name))?;
    Ok(Json(ws.config))
}

/// Update workspace configuration
///
/// PUT /api/workspaces/{repo}/{name}/config
pub async fn update_workspace_config(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
    Json(config): Json<models::workspace::WorkspaceConfig>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default"; // TODO: Extract from auth/middleware
                               // Get existing workspace
    let mut ws = state
        .ws_svc
        .get(tenant_id, &repo, &name)
        .await?
        .ok_or_else(|| ApiError::workspace_not_found(&name))?;

    // Update config
    ws.config = config;
    ws.updated_at = Some(StorageTimestamp::now());

    // Save
    state.ws_svc.put(tenant_id, &repo, ws).await?;
    Ok(StatusCode::NO_CONTENT)
}
