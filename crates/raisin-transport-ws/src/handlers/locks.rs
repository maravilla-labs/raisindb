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

#[derive(Deserialize)]
struct AcquirePayload {
    key: String,
    ttl_ms: i64,
    #[serde(default)]
    owner: Option<String>,
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
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: AcquirePayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let key = scoped_key(&request, &payload.key)?;
    let owner = payload.owner.unwrap_or_else(|| "anonymous".to_string());
    let ttl = Duration::from_millis(payload.ttl_ms.max(0) as u64);

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
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
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
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RenewPayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let key = scoped_key(&request, &payload.key)?;
    let ttl = Duration::from_millis(payload.ttl_ms.max(0) as u64);
    let renewed = mgr.renew(&key, payload.token, ttl).await?;
    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "renewed": renewed }),
    )))
}

/// Handle `inventory.claim`
pub async fn handle_inventory_claim<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
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
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ReleaseClaimPayload = serde_json::from_value(request.payload.clone())?;
    let mgr = manager(state)?;
    let pool = scoped_key(&request, &payload.pool)?;
    let remaining = mgr.release_claim(&pool, payload.n).await?;
    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "remaining": remaining }),
    )))
}
