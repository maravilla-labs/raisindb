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
use raisin_models::auth::AuthContext;

use crate::middleware::TenantInfo;
use crate::{error::ApiError, state::AppState};

/// Maximum lease a caller may request (anti-abuse: no indefinite holds).
/// Mirrors `MAX_LOCK_TTL_MS` in the SQL lock functions.
const MAX_LOCK_TTL_MS: i64 = 300_000;

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

/// ACL gate for lock / inventory operations, returning the resolved owner
/// identity derived from the auth context (never trusted from the request
/// body). Requires an authenticated, non-anonymous caller (system context
/// always allowed); a held lock is a denial-of-service vector, so anonymous /
/// unauthenticated requests are rejected with 403. Mirrors
/// `check_lock_permission` in the SQL lock functions.
fn require_owner(auth: &Option<Extension<AuthContext>>) -> Result<String, ApiError> {
    match auth.as_ref().map(|Extension(ctx)| ctx) {
        Some(ctx) if ctx.is_system || !ctx.is_anonymous => Ok(ctx.actor_id()),
        Some(_) => Err(ApiError::from(raisin_error::Error::Forbidden(
            "Anonymous users cannot perform lock operations".to_string(),
        ))),
        None => Err(ApiError::from(raisin_error::Error::Forbidden(
            "Lock operations require authentication".to_string(),
        ))),
    }
}

/// Validate & convert a requested TTL, rejecting non-positive and over-cap
/// values. Mirrors `validate_ttl` in the SQL lock functions.
fn validate_ttl(ttl_ms: i64) -> Result<Duration, ApiError> {
    if ttl_ms <= 0 {
        return Err(ApiError::from(raisin_error::Error::Validation(
            "ttl_ms must be positive".to_string(),
        )));
    }
    if ttl_ms > MAX_LOCK_TTL_MS {
        return Err(ApiError::from(raisin_error::Error::Validation(format!(
            "ttl_ms exceeds the maximum lease of {MAX_LOCK_TTL_MS} ms"
        ))));
    }
    Ok(Duration::from_millis(ttl_ms as u64))
}

#[derive(Deserialize)]
pub struct AcquireRequest {
    pub key: String,
    pub ttl_ms: i64,
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
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<AcquireRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let owner = require_owner(&auth)?;
    let mgr = manager(&state)?;
    let key = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.key);
    let ttl = validate_ttl(req.ttl_ms)?;

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
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<ReleaseRequest>,
) -> Result<Json<Value>, ApiError> {
    require_owner(&auth)?;
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
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<RenewRequest>,
) -> Result<Json<Value>, ApiError> {
    require_owner(&auth)?;
    let mgr = manager(&state)?;
    let key = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.key);
    let ttl = validate_ttl(req.ttl_ms)?;
    let renewed = mgr.renew(&key, req.token, ttl).await?;
    Ok(Json(json!({ "renewed": renewed })))
}

/// POST /api/{repo}/{branch}/inventory/claim
pub async fn claim_inventory(
    Path((repo, branch)): Path<(String, String)>,
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<ClaimRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    require_owner(&auth)?;
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
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<ReleaseClaimRequest>,
) -> Result<Json<Value>, ApiError> {
    require_owner(&auth)?;
    let mgr = manager(&state)?;
    let pool = scoped(tenant_info.tenant_id.as_str(), &repo, &branch, &req.pool);
    let remaining = mgr.release_claim(&pool, req.n).await?;
    Ok(Json(json!({ "remaining": remaining })))
}
