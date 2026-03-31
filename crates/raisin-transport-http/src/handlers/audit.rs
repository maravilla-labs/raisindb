use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_audit::AuditRepository;
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, state::AppState};

pub async fn audit_get_by_id(
    State(state): State<AppState>,
    Path((_repo, _branch, _ws, id)): Path<(String, String, String, String)>,
) -> Result<Json<Vec<raisin_models::nodes::audit_log::AuditLog>>, ApiError> {
    let logs = state.audit.get_logs_by_node_id(&id).await?;
    Ok(Json(logs))
}

pub async fn audit_get_by_path(
    State(state): State<AppState>,
    Path((repo, branch, ws, node_path)): Path<(String, String, String, String)>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Vec<raisin_models::nodes::audit_log::AuditLog>>, ApiError> {
    let tenant_id = "default"; // TODO: Extract from middleware/auth
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    let path = if node_path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", node_path.trim_start_matches('/'))
    };
    let node = nodes_svc
        .get_by_path(&path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(&path))?;
    let logs = state.audit.get_logs_by_node_id(&node.id).await?;
    Ok(Json(logs))
}
