// SPDX-License-Identifier: BSL-1.1

//! Lock / inventory operation handlers
//!
//! Exposes the atomic acquire / tie-breaker primitive over the WebSocket node
//! API. Keys are scoped by `{tenant}\0{repo}\0{branch}\0{name}` so locks never
//! collide across tenants/repos/branches.

use parking_lot::RwLock;
use raisin_locks::{LockManager, LockManagerHandle};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope},
};

/// Maximum lease a caller may request (anti-abuse: no indefinite holds).
/// Mirrors `MAX_LOCK_TTL_MS` in the SQL lock functions.
const MAX_LOCK_TTL_MS: i64 = 300_000;

#[derive(Deserialize)]
struct AcquirePayload {
    key: String,
    ttl_ms: i64,
}

#[derive(Deserialize)]
struct ReleasePayload {
    key: String,
    token: u64,
}

#[derive(Deserialize)]
struct RenewPayload {
    key: String,
    token: u64,
    ttl_ms: i64,
}

#[derive(Deserialize)]
struct ClaimPayload {
    pool: String,
    n: u64,
    capacity: u64,
}

#[derive(Deserialize)]
struct ReleaseClaimPayload {
    pool: String,
    n: u64,
}

fn manager<S, B>(state: &Arc<WsState<S, B>>) -> Result<LockManagerHandle, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    state.lock_manager.clone().ok_or_else(|| {
        WsError::InvalidRequest(
            "Locks subsystem disabled. Enable [locks] in server config.".to_string(),
        )
    })
}

/// ACL gate for lock / inventory operations, returning the owner identity
/// derived from the connection's auth context (never trusted from the
/// payload). Requires an authenticated, non-anonymous caller (system context
/// always allowed). A held lock is a denial-of-service vector, so anonymous /
/// unauthenticated requests are rejected. Mirrors `check_lock_permission` in
/// the SQL lock functions.
fn require_owner(connection_state: &Arc<RwLock<ConnectionState>>) -> Result<String, WsError> {
    let conn = connection_state.read();
    match conn.auth_context() {
        Some(ctx) if ctx.is_system || !ctx.is_anonymous => Ok(ctx.actor_id()),
        Some(_) => Err(WsError::PermissionDenied),
        None => Err(WsError::NotAuthenticated),
    }
}

/// Validate & convert a requested TTL, rejecting non-positive and over-cap
/// values. Mirrors `validate_ttl` in the SQL lock functions.
fn validate_ttl(ttl_ms: i64) -> Result<Duration, WsError> {
    if ttl_ms <= 0 {
        return Err(WsError::InvalidRequest(
            "ttl_ms must be positive".to_string(),
        ));
    }
    if ttl_ms > MAX_LOCK_TTL_MS {
        return Err(WsError::InvalidRequest(format!(
            "ttl_ms exceeds the maximum lease of {MAX_LOCK_TTL_MS} ms"
        )));
    }
    Ok(Duration::from_millis(ttl_ms as u64))
}

fn scoped_key(request: &RequestEnvelope, name: &str) -> Result<String, WsError> {
    let tenant = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_deref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    Ok(format!("{tenant}\0{repo}\0{branch}\0{name}"))
}

/// Handle `locks.acquire`
pub async fn handle_locks_acquire<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let owner = require_owner(connection_state)?;
    let payload: AcquirePayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let key = scoped_key(&request, &payload.key)?;
    let ttl = validate_ttl(payload.ttl_ms)?;

    let guard = mgr.try_acquire(&key, &owner, ttl).await?;
    let body = match guard {
        Some(g) => serde_json::json!({
            "acquired": true,
            "key": payload.key,
            "token": g.token,
            "expires_at_ms": g.expires_at_ms,
        }),
        None => serde_json::json!({ "acquired": false }),
    };
    Ok(Some(ResponseEnvelope::success(request.request_id, body)))
}

/// Handle `locks.release`
pub async fn handle_locks_release<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    require_owner(connection_state)?;
    let payload: ReleasePayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let key = scoped_key(&request, &payload.key)?;
    let released = mgr.release(&key, payload.token).await?;
    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "released": released }),
    )))
}

/// Handle `locks.renew`
pub async fn handle_locks_renew<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    require_owner(connection_state)?;
    let payload: RenewPayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let key = scoped_key(&request, &payload.key)?;
    let ttl = validate_ttl(payload.ttl_ms)?;
    let renewed = mgr.renew(&key, payload.token, ttl).await?;
    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "renewed": renewed }),
    )))
}

/// Handle `inventory.claim`
pub async fn handle_inventory_claim<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    require_owner(connection_state)?;
    let payload: ClaimPayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let pool = scoped_key(&request, &payload.pool)?;
    let remaining = mgr.claim(&pool, payload.n, payload.capacity).await?;
    let body = match remaining {
        Some(r) => serde_json::json!({ "claimed": true, "remaining": r }),
        None => serde_json::json!({ "claimed": false }),
    };
    Ok(Some(ResponseEnvelope::success(request.request_id, body)))
}

/// Handle `inventory.release`
pub async fn handle_inventory_release<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    require_owner(connection_state)?;
    let payload: ReleaseClaimPayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let pool = scoped_key(&request, &payload.pool)?;
    let remaining = mgr.release_claim(&pool, payload.n).await?;
    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "remaining": remaining }),
    )))
}
