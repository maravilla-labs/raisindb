//! Node revision-history handlers (git-style "file history").
//!
//! Lists the MVCC revisions of a node, newest first. Each entry carries the
//! `revision` (usable with the `rev/{revision}` time-travel reads to fetch the
//! full snapshot), plus when/who and whether the node was deleted at that
//! revision. Always available regardless of the NodeType `auditable` flag.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::NodeRevisionEntry;
use serde::Deserialize;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

#[derive(Debug, Default, Deserialize)]
pub struct HistoryQuery {
    /// Maximum number of revisions to return (newest first). Unbounded if omitted.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// List a node's revision history by ID.
///
/// GET /api/history/{repo}/{branch}/{ws}/by-id/{id}?limit=50
pub async fn history_get_by_id(
    State(state): State<AppState>,
    Path((repo, branch, ws, id)): Path<(String, String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<NodeRevisionEntry>>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    let entries = nodes_svc.history(&id, q.limit).await?;
    Ok(Json(entries))
}

/// List a node's revision history by path.
///
/// GET /api/history/{repo}/{branch}/{ws}/{*node_path}?limit=50
pub async fn history_get_by_path(
    State(state): State<AppState>,
    Path((repo, branch, ws, node_path)): Path<(String, String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<Vec<NodeRevisionEntry>>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    let path = if node_path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", node_path.trim_start_matches('/'))
    };

    let entries = nodes_svc.history_by_path(&path, q.limit).await?;
    Ok(Json(entries))
}
