// SPDX-License-Identifier: BSL-1.1

//! Discovered-tool listing, per-tool enablement, and manual refresh.
//!
//! The proxy `raisin:Function` nodes themselves are written only by the
//! discovery job — never here. These endpoints record *intent* (which tools an
//! operator wants exposed) on the connection's `tool_filter`, and enqueue a
//! discovery run to make the proxies match. Writing a proxy directly from an
//! HTTP handler would put a second writer on nodes whose whole consistency
//! story is "one reconcile, under a lease".

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{connection_service, load_connection, require_admin, set_prop};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connection-tools";

/// `GET /api/mcp-connections/{repo}/{slug}/tools`
pub async fn list_tools(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (node, descriptor) =
        load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    let tools = node
        .properties
        .get("discovered_tools")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Array(Vec::new()));
    let health = node
        .properties
        .get("health")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .unwrap_or(Value::Null);

    Ok(Json(json!({
        "slug": descriptor.slug,
        "tools": tools,
        "tool_filter": descriptor.tool_filter,
        "health": health,
        "last_synced_at": node
            .properties
            .get("last_synced_at")
            .and_then(|pv| serde_json::to_value(pv).ok()),
    })))
}

/// Body for toggling one tool.
#[derive(Debug, Deserialize)]
pub struct SetToolEnabledRequest {
    /// Whether the tool should be exposed.
    pub enabled: bool,
}

/// `PATCH /api/mcp-connections/{repo}/{slug}/tools/{remote_name}`
///
/// Records the decision on `tool_filter` and enqueues a discovery run to bring
/// the proxy nodes in line.
pub async fn set_tool_enabled(
    State(state): State<AppState>,
    Path((repo, slug, remote_name)): Path<(String, String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SetToolEnabledRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (mut node, descriptor) =
        load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;
    let mut filter = descriptor.tool_filter.clone();

    if req.enabled {
        filter.deny.retain(|d| d != &remote_name);
        // An `allow` list is an explicit allowlist; enabling a tool that is not
        // on it would otherwise silently do nothing.
        if !filter.allow.is_empty() && !filter.allow.iter().any(|a| a == &remote_name) {
            filter.allow.push(remote_name.clone());
        }
    } else {
        filter.allow.retain(|a| a != &remote_name);
        if !filter.deny.iter().any(|d| d == &remote_name) {
            filter.deny.push(remote_name.clone());
        }
    }

    set_prop(&mut node, "tool_filter", serde_json::to_value(&filter)?)?;
    connection_service(&state, &tenant.tenant_id, &repo, ACTOR)
        .update_node(node)
        .await?;

    let job_id = super::refresh::enqueue_discovery(&state, &tenant.tenant_id, &repo, &slug).await?;

    Ok(Json(json!({
        "ok": true,
        "remote_name": remote_name,
        "enabled": req.enabled,
        "tool_filter": filter,
        "refresh_job_id": job_id,
    })))
}
