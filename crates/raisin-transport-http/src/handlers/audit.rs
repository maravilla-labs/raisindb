use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_audit::{AuditRepository, AuditScope};
use raisin_models::auth::AuthContext;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

pub async fn audit_get_by_id(
    State(state): State<AppState>,
    Path((repo, branch, ws, id)): Path<(String, String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Vec<raisin_models::nodes::audit_log::AuditLog>>, ApiError> {
    // Authorize the read through the RLS-aware NodeService: only return audit
    // logs for a node the caller can actually read. Without this, any node id
    // could be used to read another node's audit log (IDOR).
    let tenant_id = tenant_info.tenant_id.as_str();
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context);

    let node = nodes_svc
        .get(&id)
        .await?
        .ok_or_else(|| ApiError::node_not_found(&id))?;
    let scope = AuditScope::new(tenant_id, &repo, &branch, &ws);
    let logs = state.audit.get_logs_scoped(scope, &node.id, None).await?;
    Ok(Json(logs))
}

pub async fn audit_get_by_path(
    State(state): State<AppState>,
    Path((repo, branch, ws, node_path)): Path<(String, String, String, String)>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Vec<raisin_models::nodes::audit_log::AuditLog>>, ApiError> {
    let tenant_id = tenant_info.tenant_id.as_str();
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
    // Read by the resolved node id, never by path: a MOVE/RENAME changes the
    // path while the history stays attached to the node.
    let scope = AuditScope::new(tenant_id, &repo, &branch, &ws);
    let logs = state.audit.get_logs_scoped(scope, &node.id, None).await?;
    Ok(Json(logs))
}
