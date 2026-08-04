// SPDX-License-Identifier: BSL-1.1

//! Deleting proxy functions for tools that are gone upstream.
//!
//! Discovery **disables** a tool that vanishes, never deletes it — a deleted
//! proxy makes the tool silently disappear from any agent holding its path
//! (`resolveToolsParallel` returns `null` and the model simply stops being
//! offered it, with no error anywhere). That is the right default, but it leaves
//! `missing` entries accumulating with no way to clear them.
//!
//! So pruning exists, and it is **explicit only**. There is no age-based
//! automatic prune: the whole reason disable-don't-delete is the default is that
//! nothing in the system can tell whether an agent still needs that path, and a
//! timer knows even less than an operator does.
//!
//! Both endpoints mirror the connection-delete contract: refuse with 409 naming
//! the agents that reference the paths, and require `?force=true` to proceed.

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{connection_service, load_connection, require_admin, set_prop};
use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

const ACTOR: &str = "mcp-connection-prune";
/// Workspace holding generated proxy functions.
const FUNCTIONS_WORKSPACE: &str = "functions";
/// Workspace holding agents.
const AGENTS_NODE_TYPE: &str = "raisin:AIAgent";

/// Query parameters for a prune.
#[derive(Debug, Deserialize)]
pub struct PruneQuery {
    /// Delete even when an agent still references the path.
    #[serde(default)]
    pub force: bool,
}

/// `POST /api/mcp-connections/{repo}/{slug}/prune-tools`
///
/// Delete every proxy whose state is `missing`.
pub async fn prune_tools(
    State(state): State<AppState>,
    Path((repo, slug)): Path<(String, String)>,
    Query(query): Query<PruneQuery>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;
    let (node, _) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    let targets: Vec<Value> = discovered(&node)
        .into_iter()
        .filter(|t| t.get("state").and_then(Value::as_str) == Some("missing"))
        .collect();

    if targets.is_empty() {
        return Ok(Json(json!({ "ok": true, "pruned": 0, "tools": [] })));
    }
    apply_prune(
        &state,
        &tenant.tenant_id,
        &repo,
        &slug,
        targets,
        query.force,
    )
    .await
}

/// `DELETE /api/mcp-connections/{repo}/{slug}/tools/{remote_name}`
///
/// Delete one proxy, whatever its state.
pub async fn delete_tool(
    State(state): State<AppState>,
    Path((repo, slug, remote_name)): Path<(String, String, String)>,
    Query(query): Query<PruneQuery>,
    Extension(tenant): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
) -> Result<Json<Value>, ApiError> {
    require_admin(auth.as_deref())?;
    let (node, _) = load_connection(&state, &tenant.tenant_id, &repo, &slug, ACTOR).await?;

    let targets: Vec<Value> = discovered(&node)
        .into_iter()
        .filter(|t| t.get("remote_name").and_then(Value::as_str) == Some(remote_name.as_str()))
        .collect();

    if targets.is_empty() {
        return Err(ApiError::new(
            axum::http::StatusCode::NOT_FOUND,
            "TOOL_NOT_FOUND",
            format!("connection `{slug}` has no discovered tool named `{remote_name}`"),
        ));
    }
    apply_prune(
        &state,
        &tenant.tenant_id,
        &repo,
        &slug,
        targets,
        query.force,
    )
    .await
}

/// Check references, delete the proxies, and drop them from `discovered_tools`.
async fn apply_prune(
    state: &AppState,
    tenant: &str,
    repo: &str,
    slug: &str,
    targets: Vec<Value>,
    force: bool,
) -> Result<Json<Value>, ApiError> {
    let paths: Vec<String> = targets
        .iter()
        .filter_map(|t| t.get("function_path").and_then(Value::as_str))
        .map(str::to_string)
        .collect();

    // The check that makes this safe to offer at all. An operator pruning a
    // tool an agent still lists gets told which agent, by path, rather than
    // discovering it when the agent quietly stops being able to do something.
    if !force {
        let referencing = agents_referencing(state, tenant, repo, &paths).await?;
        if !referencing.is_empty() {
            return Err(ApiError::new(
                axum::http::StatusCode::CONFLICT,
                "TOOLS_IN_USE",
                format!(
                    "{} agent(s) still reference these tools: {}. Remove the paths from their \
                     `tools` arrays, or retry with ?force=true.",
                    referencing.len(),
                    referencing.join(", ")
                ),
            ));
        }
    }

    let functions = state.node_service_for_context(
        tenant,
        repo,
        super::CONFIG_BRANCH,
        FUNCTIONS_WORKSPACE,
        Some(system_auth()),
    );

    let mut deleted = Vec::new();
    for path in &paths {
        match functions.delete_by_path(path).await {
            // Already gone is success: the goal is that it is not there.
            Ok(_) => deleted.push(path.clone()),
            Err(e) => {
                tracing::warn!(%path, error = %e, "could not delete an MCP proxy function");
            }
        }
    }

    // Drop the pruned entries from the connection's record, so the console does
    // not keep offering to prune something that is already gone.
    let (mut node, _) = load_connection(state, tenant, repo, slug, ACTOR).await?;
    let pruned: std::collections::HashSet<&str> = targets
        .iter()
        .filter_map(|t| t.get("remote_name").and_then(Value::as_str))
        .collect();
    let remaining: Vec<Value> = discovered(&node)
        .into_iter()
        .filter(|t| {
            !t.get("remote_name")
                .and_then(Value::as_str)
                .is_some_and(|n| pruned.contains(n))
        })
        .collect();
    set_prop(&mut node, "discovered_tools", Value::Array(remaining))?;
    connection_service(state, tenant, repo, ACTOR)
        .update_node(node)
        .await?;

    Ok(Json(json!({
        "ok": true,
        "pruned": deleted.len(),
        "tools": deleted,
    })))
}

/// Paths of agents whose `tools` array mentions any of `paths`.
async fn agents_referencing(
    state: &AppState,
    tenant: &str,
    repo: &str,
    paths: &[String],
) -> Result<Vec<String>, ApiError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let wanted: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();

    let agents = state.node_service_for_context(
        tenant,
        repo,
        super::CONFIG_BRANCH,
        FUNCTIONS_WORKSPACE,
        Some(system_auth()),
    );
    let nodes = agents.list_by_type(AGENTS_NODE_TYPE).await?;

    let mut referencing = Vec::new();
    for node in nodes {
        let tools = node
            .properties
            .get("tools")
            .and_then(|pv| serde_json::to_value(pv).ok())
            .unwrap_or(Value::Null);
        let Some(list) = tools.as_array() else {
            continue;
        };
        // An entry may be a bare path string or an object carrying one; both
        // forms are accepted by the resolvers, so both must be checked here.
        let hit = list.iter().any(|entry| {
            let path = entry
                .as_str()
                .or_else(|| entry.get("path").and_then(Value::as_str));
            path.is_some_and(|p| wanted.contains(p))
        });
        if hit {
            referencing.push(node.path.clone());
        }
    }
    Ok(referencing)
}

/// The connection's recorded tools as a JSON array.
fn discovered(node: &raisin_models::nodes::Node) -> Vec<Value> {
    node.properties
        .get("discovered_tools")
        .and_then(|pv| serde_json::to_value(pv).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

/// System auth for reading agents and deleting generated functions.
fn system_auth() -> AuthContext {
    let mut auth = AuthContext::system();
    auth.user_id = Some(ACTOR.to_string());
    auth
}
