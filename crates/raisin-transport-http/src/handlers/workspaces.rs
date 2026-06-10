// SPDX-License-Identifier: BSL-1.1

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models as models;
use raisin_models::timestamp::StorageTimestamp;

use crate::{
    error::ApiError,
    middleware::TenantInfo,
    state::AppState,
    types::{Page, PageMeta, PageParams},
};

pub async fn list_workspaces(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    axum::extract::Query(p): axum::extract::Query<PageParams>,
) -> Result<Json<Page<models::workspace::Workspace>>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
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
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<models::workspace::Workspace>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let ws = state
        .ws_svc
        .get(tenant_id, &repo, &name)
        .await?
        .ok_or_else(|| ApiError::workspace_not_found(&name))?;
    Ok(Json(ws))
}

/// Require an operator (admin token / system context) for structural
/// workspace mutations. Workspace definitions control allowed node types and
/// RLS surface, so identity users and anonymous callers must not touch them.
fn require_operator(auth: &Option<Extension<models::auth::AuthContext>>) -> Result<(), ApiError> {
    let is_operator = auth.as_ref().is_some_and(|Extension(ctx)| {
        ctx.is_system || ctx.permissions().is_some_and(|p| p.is_system_admin)
    });
    if is_operator {
        Ok(())
    } else {
        Err(raisin_error::Error::PermissionDenied(
            "Workspace management requires operator privileges".to_string(),
        )
        .into())
    }
}

pub async fn put_workspace(
    State(state): State<AppState>,
    Path((repo, name)): Path<(String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<models::auth::AuthContext>>,
    Json(mut ws): Json<models::workspace::Workspace>,
) -> Result<StatusCode, ApiError> {
    require_operator(&auth)?;
    let tenant_id = tenant_info.tenant_id.as_str();
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
    Extension(tenant_info): Extension<TenantInfo>,
) -> Result<Json<models::workspace::WorkspaceConfig>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
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
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<models::auth::AuthContext>>,
    Json(config): Json<models::workspace::WorkspaceConfig>,
) -> Result<StatusCode, ApiError> {
    require_operator(&auth)?;
    let tenant_id = tenant_info.tenant_id.as_str();
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
