// SPDX-License-Identifier: BSL-1.1

//! Secret store handlers for the WebSocket node API.
//!
//! The same contract as the HTTP surface (`raisin-transport-http`'s
//! `handlers/secrets.rs`): admin-gated, and **no request returns a plaintext
//! value**. `secret_get` deliberately means *get metadata*; there is no
//! `secret_read`. Server-side use resolves a `secret://` reference through
//! `SecretStore::resolve`, which is not reachable from any client-facing path.
//!
//! Scope is `{tenant, repo, branch}` taken from the request context, never from
//! the payload — the same rule the lock handlers follow for their key scoping.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
#[cfg(feature = "storage-rocksdb")]
use serde::Deserialize;
use std::sync::Arc;

#[cfg(feature = "storage-rocksdb")]
use raisin_rocksdb::secret_store::{SecretOwner, SecretScope, SecretStore};

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope},
};

#[cfg(feature = "storage-rocksdb")]
#[derive(Deserialize)]
struct SecretNamePayload {
    name: String,
}

#[cfg(feature = "storage-rocksdb")]
#[derive(Deserialize)]
struct SecretValuePayload {
    name: String,
    /// The plaintext to seal. Sent, never returned.
    value: String,
}

/// Admin gate. Managing credentials is an operator action, so an ordinary
/// authenticated principal is refused just as it is over HTTP.
fn require_admin(connection_state: &Arc<RwLock<ConnectionState>>) -> Result<String, WsError> {
    let conn = connection_state.read();
    match conn.auth_context() {
        Some(ctx)
            if ctx.is_system
                || ctx
                    .roles
                    .iter()
                    .any(|r| r == "system_admin" || r == "admin")
                || ctx.permissions().is_some_and(|p| p.is_system_admin) =>
        {
            Ok(ctx.actor_id())
        }
        Some(_) => Err(WsError::PermissionDenied),
        None => Err(WsError::NotAuthenticated),
    }
}

#[cfg(feature = "storage-rocksdb")]
fn scope(request: &RequestEnvelope) -> Result<SecretScope, WsError> {
    let repo = request
        .context
        .repository
        .as_deref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    Ok(SecretScope::new(
        request.context.tenant_id.clone(),
        repo,
        branch,
    ))
}

#[cfg(feature = "storage-rocksdb")]
fn store<S, B>(state: &Arc<WsState<S, B>>) -> Result<Arc<SecretStore>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let rocksdb = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| WsError::InternalError("RocksDB storage not available".to_string()))?;
    rocksdb
        .secret_store()
        .map_err(|e| WsError::InternalError(e.to_string()))
}

/// The store's errors reach the client with the `Gone` / `Pending` split
/// intact — a caller that retries on one and gives up on the other is correct
/// only if the two stay distinguishable.
#[cfg(feature = "storage-rocksdb")]
fn ws_err(e: raisin_rocksdb::secret_store::SecretError) -> WsError {
    WsError::OperationError(e.to_string())
}

/// Handle `secret_put` — create, or append a version. Returns name + version.
pub async fn handle_secret_put<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let actor = require_admin(connection_state)?;
    #[cfg(feature = "storage-rocksdb")]
    {
        let payload: SecretValuePayload = serde_json::from_value(request.payload.clone())?;
        let scope = scope(&request)?;
        let (name, version) = store(state)?
            .put(
                &scope,
                &payload.name,
                payload.value.as_bytes(),
                &SecretOwner::actor(actor),
            )
            .map_err(ws_err)?;
        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "name": name,
                "version": version,
                "reference": raisin_models::secret_ref::format_secret_ref(&name, None),
            }),
        )))
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = (state, actor, request);
        Err(unsupported())
    }
}

/// Handle `secret_rotate` — a new version stamped `rotated_at`. An append, so a
/// pinned `secret://name@N` keeps resolving.
pub async fn handle_secret_rotate<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let actor = require_admin(connection_state)?;
    #[cfg(feature = "storage-rocksdb")]
    {
        let payload: SecretValuePayload = serde_json::from_value(request.payload.clone())?;
        let scope = scope(&request)?;
        let (name, version) = store(state)?
            .rotate(
                &scope,
                &payload.name,
                payload.value.as_bytes(),
                &SecretOwner::actor(actor),
            )
            .map_err(ws_err)?;
        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "name": name,
                "version": version,
                "rotated": true,
                "reference": raisin_models::secret_ref::format_secret_ref(&name, None),
            }),
        )))
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = (state, actor, request);
        Err(unsupported())
    }
}

/// Handle `secret_list` — metadata for the newest version of every secret in
/// the branch. `SecretMetadata` has no field that can hold a value.
pub async fn handle_secret_list<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    require_admin(connection_state)?;
    #[cfg(feature = "storage-rocksdb")]
    {
        let scope = scope(&request)?;
        let secrets = store(state)?.list(&scope).map_err(ws_err)?;
        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({ "secrets": secrets }),
        )))
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = (state, request);
        Err(unsupported())
    }
}

/// Handle `secret_get` — metadata for every version of one secret.
///
/// **Metadata only.** The name is `secret_get` to match the CRUD vocabulary of
/// the other request types; there is no request that returns the value.
pub async fn handle_secret_get<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    require_admin(connection_state)?;
    #[cfg(feature = "storage-rocksdb")]
    {
        let payload: SecretNamePayload = serde_json::from_value(request.payload.clone())?;
        let scope = scope(&request)?;
        let versions = store(state)?
            .list_versions(&scope, &payload.name)
            .map_err(ws_err)?;
        if versions.is_empty() {
            return Err(WsError::OperationError(format!(
                "No secret named '{}'",
                payload.name
            )));
        }
        // Newest first, so entry 0 IS the current state of the name. Its fields
        // are flattened to the top level so this body has the SAME shape the
        // HTTP surface returns: a `SecretMetadata` with `versions` alongside.
        // Two surfaces describing one secret differently is how a client ends
        // up with two code paths for one concept.
        let mut body = serde_json::to_value(&versions[0])
            .map_err(|e| WsError::InternalError(e.to_string()))?;
        let map = body
            .as_object_mut()
            .expect("SecretMetadata serializes to a JSON object");
        map.insert(
            "versions".to_string(),
            serde_json::to_value(&versions).map_err(|e| WsError::InternalError(e.to_string()))?,
        );
        Ok(Some(ResponseEnvelope::success(request.request_id, body)))
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = (state, request);
        Err(unsupported())
    }
}

/// Handle `secret_delete` — append a tombstone. Prior versions stay readable
/// through a pinned reference, so older node revisions still resolve.
pub async fn handle_secret_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let actor = require_admin(connection_state)?;
    #[cfg(feature = "storage-rocksdb")]
    {
        let payload: SecretNamePayload = serde_json::from_value(request.payload.clone())?;
        let scope = scope(&request)?;
        let version = store(state)?
            .delete(&scope, &payload.name, &SecretOwner::actor(actor))
            .map_err(ws_err)?;
        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "name": payload.name,
                "version": version,
                "deleted": true,
            }),
        )))
    }
    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = (state, actor, request);
        Err(unsupported())
    }
}

#[cfg(not(feature = "storage-rocksdb"))]
fn unsupported() -> WsError {
    WsError::InvalidRequest("The secret store requires the RocksDB storage backend".to_string())
}
