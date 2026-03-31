// SPDX-License-Identifier: BSL-1.1

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use raisin_models as models;
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, state::AppState};

/// Get a node by its ID
#[allow(dead_code)]
pub async fn get_node(
    State(state): State<AppState>,
    Path((ws, id)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<models::nodes::Node>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let repo_id = "main"; // TODO
    let branch = "main"; // TODO
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, repo_id, branch, &ws, auth_context);

    let node = nodes_svc
        .get(&id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(&id))?;
    // Populate has_children field for frontend tree rendering
    Ok(Json(node))
}

/// Update a node by its ID
#[allow(dead_code)]
pub async fn put_node(
    State(state): State<AppState>,
    Path((ws, id)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(mut node): Json<models::nodes::Node>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let repo_id = "main"; // TODO
    let branch = "main"; // TODO
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, repo_id, branch, &ws, auth_context);

    // Ensure id and workspace alignment
    node.id = id;
    if node.workspace.as_deref() != Some(&ws) {
        node.workspace = Some(ws.clone());
    }
    nodes_svc.put(node).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a node by its ID
#[allow(dead_code)]
pub async fn delete_node(
    State(state): State<AppState>,
    Path((ws, id)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<bool>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let repo_id = "main"; // TODO
    let branch = "main"; // TODO
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, repo_id, branch, &ws, auth_context);

    let ok = nodes_svc.delete(&id).await?;
    Ok(Json(ok))
}
