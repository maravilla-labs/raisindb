// SPDX-License-Identifier: BSL-1.1

//! Connection CRUD.
//!
//! Typed endpoints rather than the generic node API — which is how
//! `raisin:Integration` is edited — for one reason: a connection URL must be
//! validated against the operator's egress policy, and the slug against the
//! path grammar, BEFORE the node is written. Generic node CRUD runs neither
//! check, so a connection created that way could point at
//! `http://169.254.169.254/` and would only fail later, from a job, where
//! nobody is watching.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use raisin_mcp::client::{validate_slug, McpConnectionDescriptor};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use super::{
    connection_path, connection_service, egress_policy, load_connection, remote_error,
    require_admin, set_prop, CONNECTIONS_FOLDER, CONNECTION_NODE_TYPE,
};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connections";

/// Body for creating a connection.
#[derive(Debug, Deserialize)]
pub struct CreateConnectionRequest {
    /// URL-safe slug; immutable once set.
    pub slug: String,
    /// Human-readable title.
    #[serde(default)]
    pub title: Option<String>,
    /// Streamable HTTP endpoint.
    pub url: String,
    /// Whether the connection is active. Defaults to false: a connection is not
    /// useful until it has a credential, and an enabled one would start being
    /// dialled by discovery immediately.
    #[serde(default)]
    pub enabled: bool,
    /// `none` | `static` | `oauth`.
    #[serde(default)]
    pub auth_kind: Option<String>,
    /// Non-secret static-auth shape.
    #[serde(default)]
    pub static_auth: Option<Value>,
    /// Tool allow/deny lists.
    #[serde(default)]
    pub tool_filter: Option<Value>,
    /// Discovery schedule.
    #[serde(default)]
    pub refresh_policy: Option<Value>,
}

/// Body for updating a connection. Absent fields are left untouched.
#[derive(Debug, Deserialize)]
pub struct UpdateConnectionRequest {
    /// New title.
    #[serde(default)]
    pub title: Option<String>,
    /// New endpoint; re-validated against the egress policy.
    #[serde(default)]
    pub url: Option<String>,
    /// Enable or disable.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Change the auth mode.
    #[serde(default)]
    pub auth_kind: Option<String>,
    /// Replace the static-auth shape.
    #[serde(default)]
    pub static_auth: Option<Value>,
    /// Replace the tool filter.
    #[serde(default)]
    pub tool_filter: Option<Value>,
    /// Replace the refresh policy.
    #[serde(default)]
    pub refresh_policy: Option<Value>,
}

/// A connection as returned to the console — always redacted.
#[derive(Debug, Serialize)]
pub struct ConnectionResponse {
    /// Redacted connection view.
    #[serde(flatten)]
    pub connection: Value,
}

/// `GET /api/mcp-connections/{repo}` — list connections.
pub async fn list_connections(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let svc = connection_service(&state, &tenant.tenant_id, &repo, ACTOR);
    let nodes = svc.list_by_type(CONNECTION_NODE_TYPE).await?;

    // A connection that fails to parse is reported, not skipped and not fatal:
    // hiding it would leave an operator staring at a console that does not show
    // the thing they just broke.
    let connections: Vec<Value> = nodes
        .iter()
        .map(|node| match McpConnectionDescriptor::from_node(node) {
            Ok(descriptor) => descriptor.redacted(),
            Err(err) => json!({
                "slug": node.name,
                "path": node.path,
                "invalid": true,
                "error": err.to_string(),
            }),
        })
        .collect();

    Ok(Json(json!({ "connections": connections })))
}

/// `GET /api/mcp-connections/{repo}/{slug}` — read one connection.
pub async fn get_connection(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;
    let (_, descriptor) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;
    Ok(Json(descriptor.redacted()))
}

/// `POST /api/mcp-connections/{repo}` — create a connection.
pub async fn create_connection(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<CreateConnectionRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    validate_slug(&req.slug).map_err(remote_error)?;
    let url = parse_and_check_url(&state, &req.url)?;

    let svc = connection_service(&state, &tenant.tenant_id, &repo, ACTOR);
    let path = connection_path(&req.slug);
    if svc.get_by_path(&path).await?.is_some() {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "CONNECTION_EXISTS",
            format!("an MCP connection with slug `{}` already exists", req.slug),
        ));
    }

    let mut node = new_connection_node(&req.slug, &path);

    set_prop(&mut node, "slug", json!(req.slug))?;
    set_prop(
        &mut node,
        "title",
        json!(req.title.unwrap_or_else(|| req.slug.clone())),
    )?;
    set_prop(&mut node, "url", json!(url.as_str()))?;
    set_prop(&mut node, "enabled", json!(req.enabled))?;
    set_prop(
        &mut node,
        "auth_kind",
        json!(req.auth_kind.unwrap_or_else(|| "none".to_string())),
    )?;
    for (key, value) in [
        ("static_auth", req.static_auth),
        ("tool_filter", req.tool_filter),
        ("refresh_policy", req.refresh_policy),
    ] {
        if let Some(value) = value {
            set_prop(&mut node, key, value)?;
        }
    }

    // Parse what we are about to write, so a request that would produce an
    // unreadable connection fails here rather than on first use.
    let descriptor = McpConnectionDescriptor::from_node(&node).map_err(remote_error)?;

    svc.create(node).await?;
    Ok(Json(descriptor.redacted()))
}

/// A fresh connection node.
///
/// Extracted from the handler for one reason: the id below has no test coverage
/// anywhere else, and its absence is what broke the feature in the field.
fn new_connection_node(slug: &str, path: &str) -> Node {
    Node {
        // The CALLER mints the id. `NodeService::create` does not generate one —
        // it looks this value up verbatim and refuses if it is taken, so leaving
        // it at `Default`'s empty string stores the first connection under ""
        // and makes every later create collide with it. Same as the canonical
        // create path in `handlers/repo/post.rs`.
        id: nanoid::nanoid!(),
        node_type: CONNECTION_NODE_TYPE.to_string(),
        name: slug.to_string(),
        path: path.to_string(),
        // `parent` is the parent's NAME, not its path.
        parent: Some(CONNECTIONS_FOLDER.to_string()),
        ..Default::default()
    }
}

/// `PATCH /api/mcp-connections/{repo}/{slug}` — update a connection.
pub async fn update_connection(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<UpdateConnectionRequest>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (mut node, _) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    // `slug` is deliberately absent from the update body. It is interpolated
    // into every generated proxy's name AND path, so changing it would orphan
    // every proxy and silently break every agent referencing one.
    if let Some(title) = req.title {
        set_prop(&mut node, "title", json!(title))?;
    }
    if let Some(raw_url) = req.url {
        let url = parse_and_check_url(&state, &raw_url)?;
        set_prop(&mut node, "url", json!(url.as_str()))?;
    }
    if let Some(enabled) = req.enabled {
        set_prop(&mut node, "enabled", json!(enabled))?;
    }
    if let Some(auth_kind) = req.auth_kind {
        set_prop(&mut node, "auth_kind", json!(auth_kind))?;
    }
    for (key, value) in [
        ("static_auth", req.static_auth),
        ("tool_filter", req.tool_filter),
        ("refresh_policy", req.refresh_policy),
    ] {
        if let Some(value) = value {
            set_prop(&mut node, key, value)?;
        }
    }

    let descriptor = McpConnectionDescriptor::from_node(&node).map_err(remote_error)?;

    let svc = connection_service(&state, &tenant.tenant_id, &repo, ACTOR);
    svc.update_node(node).await?;
    Ok(Json(descriptor.redacted()))
}

/// `DELETE /api/mcp-connections/{repo}/{slug}` — delete a connection.
///
/// Refuses while proxies still exist unless `?force=true`. Deleting a
/// connection out from under its proxies leaves functions whose every call
/// fails, referenced by agents that give no indication why — so the default is
/// to say what is in the way.
pub async fn delete_connection(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<DeleteQuery>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;

    let (_, descriptor) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    if !query.force && !descriptor.discovered_tool_names.is_empty() {
        return Err(ApiError::new(
            axum::http::StatusCode::CONFLICT,
            "CONNECTION_HAS_TOOLS",
            format!(
                "connection `{slug}` still has {} discovered tool(s); agents may reference their \
                 proxies. Retry with ?force=true to delete anyway.",
                descriptor.discovered_tool_names.len()
            ),
        ));
    }

    let svc = connection_service(&state, &tenant.tenant_id, &repo, ACTOR);
    let deleted = svc.delete_by_path(&connection_path(&slug)).await?;
    Ok(Json(json!({ "ok": deleted, "slug": slug })))
}

/// Query parameters for delete.
#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    /// Delete even when proxies exist.
    #[serde(default)]
    pub force: bool,
}

/// Parse an endpoint URL and check it against the egress policy.
fn parse_and_check_url(state: &AppState, raw: &str) -> Result<Url, ApiError> {
    let url = Url::parse(raw)
        .map_err(|e| ApiError::validation_failed(format!("invalid endpoint url `{raw}`: {e}")))?;
    egress_policy(state)
        .validate_url(&url)
        .map_err(remote_error)?;
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE regression an operator hit: the first connection saved, and every
    /// one after it failed with `Node with ID '' already exists`, because
    /// `Node::default()` leaves the id empty and `NodeService::create` refuses
    /// a duplicate rather than minting one.
    #[test]
    fn every_connection_gets_its_own_non_empty_id() {
        let first = new_connection_node("github", "/mcp-connections/github");
        let second = new_connection_node("linear", "/mcp-connections/linear");

        assert!(
            !first.id.is_empty(),
            "an empty id collides on the second create"
        );
        assert!(!second.id.is_empty());
        assert_ne!(first.id, second.id);
    }

    /// Re-creating a slug after deleting it must not reuse the old identity.
    #[test]
    fn recreating_the_same_slug_mints_a_fresh_id() {
        let a = new_connection_node("github", "/mcp-connections/github");
        let b = new_connection_node("github", "/mcp-connections/github");
        assert_ne!(a.id, b.id);
        assert_eq!(a.path, b.path);
    }
}
