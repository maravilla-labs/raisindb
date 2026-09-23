// SPDX-License-Identifier: BSL-1.1

//! The node-development surface over HTTP.
//!
//! One route per method, all thin adapters over
//! `raisin_core::services::node_dev::dispatch::call` — the same entry point
//! the function bindings use, so an external client (or an external agent
//! driving a run) gets exactly the shapes and rules an in-database tool gets.
//!
//! ```text
//! GET  /api/node-dev/{repo}                 the method list
//! POST /api/node-dev/{repo}/{method}?branch= one call; body = the request
//! ```
//!
//! Methods: stat, read, list, diff, watch · dry_run, propose, get_changeset,
//! list_changesets, commit, discard, apply · fork_branch, diff_branch,
//! merge_branch, discard_branch.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use raisin_core::services::node_dev::{dispatch, DevScope, NodeDevError};
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// `?branch=`.
#[derive(Debug, Deserialize, Default)]
pub struct BranchQuery {
    /// Branch (default: the body's `branch`, else `main`).
    pub branch: Option<String>,
}

fn map_err(e: NodeDevError) -> ApiError {
    let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    ApiError::new(status, e.code, e.message)
}

/// The method list.
pub async fn methods() -> Json<Value> {
    Json(json!({ "methods": dispatch::METHODS }))
}

/// One node-development call.
pub async fn call(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, method)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(args): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let Some(Extension(auth)) = auth else {
        return Err(ApiError::unauthorized("authentication required"));
    };
    if !auth.is_system && auth.is_anonymous_principal() {
        return Err(ApiError::unauthorized("authentication required"));
    }
    let branch = q
        .branch
        .or_else(|| {
            args.get("branch")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "main".to_string());
    let scope = DevScope::new(&tenant.tenant_id, &repo, &branch);
    // A client acting for an agent run is held to the run's own grant.
    let grant = match raisin_rocksdb::node_dev::tool_run_id(&args) {
        Some(run) => {
            raisin_rocksdb::node_dev::run_grant(&tenant.tenant_id, &repo, &branch, &run).await
        }
        None => None,
    };
    let svc = raisin_rocksdb::node_dev::node_dev_or_local(&state.storage);
    let out = dispatch::call(&svc, &scope, &auth, &method, args, grant.as_deref())
        .await
        .map_err(map_err)?;
    Ok(Json(out))
}
