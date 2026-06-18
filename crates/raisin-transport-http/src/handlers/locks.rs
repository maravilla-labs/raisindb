// SPDX-License-Identifier: BSL-1.1

//! HTTP handlers for the atomic lock / inventory subsystem.
//!
//! Exposes the lease-lock (`/locks/*`) and counting-reservation
//! (`/inventory/*`) primitives over REST. Keys are scoped by
//! `{tenant}\0{repo}\0{branch}\0{name}` so they never collide across scopes.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

use raisin_locks::{LockManager, LockManagerHandle};

use crate::middleware::TenantInfo;
use crate::{error::ApiError, state::AppState};

fn manager(state: &AppState) -> Result<LockManagerHandle, ApiError> {
    state.lock_manager.clone().ok_or_else(|| {
        ApiError::from(raisin_error::Error::Validation(
            "Locks subsystem disabled. Enable [locks] in server config.".to_string(),
        ))
    })
}

fn scoped(tenant: &str, repo: &str, branch: &str, name: &str) -> String {
    format!("{tenant}\0{repo}\0{branch}\0{name}")
}

#[derive(Deserialize)]
pub struct AcquireRequest {
    pub key: String,
    pub ttl_ms: i64,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Deserialize)]
pub struct ReleaseRequest {
    pub key: String,
    pub token: u64,
}

#[derive(Deserialize)]
pub struct RenewRequest {
    pub key: String,
    pub token: u64,
    pub ttl_ms: i64,
}

#[derive(Deserialize)]
pub struct ClaimRequest {
    pub pool: String,
    pub n: u64,
    pub capacity: u64,
}

#[derive(Deserialize)]
pub struct ReleaseClaimRequest {
    pub pool: String,
    pub n: u64,
}

/// POST /api/{repo}/{branch}/locks/acquire
pub async fn acquire_lock(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<AcquireRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mgr = manager(&state)?;
    let key = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.key);
    let owner = req.owner.unwrap_or_else(|| "anonymous".to_string());
    let ttl = Duration::from_millis(req.ttl_ms.max(0) as u64);

    match mgr.try_acquire(&key, &owner, ttl).await? {
        Some(g) => Ok((
            StatusCode::OK,
            Json(json!({
                "acquired": true,
                "key": req.key,
                "token": g.token,
                "expires_at_ms": g.expires_at_ms,
            })),
        )),
        // 409 Conflict: the resource is currently held (tie-breaker loss).
        None => Ok((StatusCode::CONFLICT, Json(json!({ "acquired": false })))),
    }
}

/// POST /api/{repo}/{branch}/locks/release
pub async fn release_lock(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<ReleaseRequest>,
) -> Result<Json<Value>, ApiError> {
    let mgr = manager(&state)?;
    let key = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.key);
    let released = mgr.release(&key, req.token).await?;
    Ok(Json(json!({ "released": released })))
}

/// POST /api/{repo}/{branch}/locks/renew
pub async fn renew_lock(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<RenewRequest>,
) -> Result<Json<Value>, ApiError> {
    let mgr = manager(&state)?;
    let key = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.key);
    let ttl = Duration::from_millis(req.ttl_ms.max(0) as u64);
    let renewed = mgr.renew(&key, req.token, ttl).await?;
    Ok(Json(json!({ "renewed": renewed })))
}

/// POST /api/{repo}/{branch}/inventory/claim
pub async fn claim_inventory(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<ClaimRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mgr = manager(&state)?;
    let pool = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.pool);
    match mgr.claim(&pool, req.n, req.capacity).await? {
        Some(remaining) => Ok((
            StatusCode::OK,
            Json(json!({ "claimed": true, "remaining": remaining })),
        )),
        // 409 Conflict: sold out.
        None => Ok((StatusCode::CONFLICT, Json(json!({ "claimed": false })))),
    }
}

/// POST /api/{repo}/{branch}/inventory/release
pub async fn release_inventory(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Json(req): Json<ReleaseClaimRequest>,
) -> Result<Json<Value>, ApiError> {
    let mgr = manager(&state)?;
    let pool = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.pool);
    let remaining = mgr.release_claim(&pool, req.n).await?;
    Ok(Json(json!({ "remaining": remaining })))
}
