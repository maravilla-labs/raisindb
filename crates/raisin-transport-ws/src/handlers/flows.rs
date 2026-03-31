// SPDX-License-Identifier: BSL-1.1

//! Flow execution handlers: run, resume, get status, and cancel.
//!
//! These handlers are thin WebSocket adapters that delegate to the shared
//! [`raisin_flow_runtime::service`] module for transport-agnostic logic.

use parking_lot::RwLock;
use serde::Deserialize;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope},
};

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FlowRunPayload {
    flow_path: String,
    #[serde(default)]
    input: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct FlowResumePayload {
    instance_id: String,
    #[serde(default)]
    resume_data: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct FlowInstanceIdPayload {
    instance_id: String,
}

// ---------------------------------------------------------------------------
// RocksDB-backed implementations
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
mod inner {
    use super::*;
    use raisin_flow_runtime::service;
    use raisin_rocksdb::get_flow_job_scheduler;
    use raisin_storage::Storage;

    /// Extract the actor identity and home path from the connection state.
    fn extract_actor(connection_state: &Arc<RwLock<ConnectionState>>) -> (String, Option<String>) {
        let conn = connection_state.read();
        let auth = conn.auth_context().cloned();
        drop(conn);
        let actor = auth
            .as_ref()
            .and_then(|a| a.user_id.clone())
            .unwrap_or_else(|| "ws_api".to_string());
        let actor_home = auth.as_ref().and_then(|a| a.home.clone());
        (actor, actor_home)
    }

    /// Require `context.repository` from the request.
    fn require_repo(request: &RequestEnvelope) -> Result<String, WsError> {
        request
            .context
            .repository
            .clone()
            .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))
    }

    pub async fn handle_flow_run<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: FlowRunPayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let (actor, actor_home) = extract_actor(connection_state);
        let scheduler = get_flow_job_scheduler(&state.rocksdb_storage)?;

        let result = service::run_flow(
            state.storage.as_ref(),
            scheduler,
            &repo,
            &payload.flow_path,
            payload.input,
            actor,
            actor_home,
        )
        .await?;

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "instance_id": result.instance_id,
                "job_id": result.job_id,
                "status": "queued",
            }),
        )))
    }

    pub async fn handle_flow_resume<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: FlowResumePayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let _actor = extract_actor(connection_state);
        let scheduler = get_flow_job_scheduler(&state.rocksdb_storage)?;

        let result = service::resume_flow(
            state.storage.as_ref(),
            scheduler,
            &repo,
            &payload.instance_id,
            payload.resume_data,
        )
        .await?;

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "instance_id": result.instance_id,
                "job_id": result.job_id,
                "status": "resumed",
            }),
        )))
    }

    pub async fn handle_flow_get_instance_status<S, B>(
        state: &Arc<WsState<S, B>>,
        _connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: FlowInstanceIdPayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;

        let status =
            service::get_instance_status(state.storage.as_ref(), &repo, &payload.instance_id)
                .await?;

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::to_value(&status).unwrap_or_default(),
        )))
    }

    pub async fn handle_flow_cancel<S, B>(
        state: &Arc<WsState<S, B>>,
        _connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: FlowInstanceIdPayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let scheduler = get_flow_job_scheduler(&state.rocksdb_storage)?;

        service::cancel_instance(
            state.storage.as_ref(),
            scheduler,
            &repo,
            &payload.instance_id,
        )
        .await?;

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "instance_id": payload.instance_id,
                "status": "cancelled",
            }),
        )))
    }
}

// ---------------------------------------------------------------------------
// Feature-gated re-exports / fallback stubs
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
pub use inner::{
    handle_flow_cancel, handle_flow_get_instance_status, handle_flow_resume, handle_flow_run,
};

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_flow_run<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Flow execution requires RocksDB backend".to_string(),
    )))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_flow_resume<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Flow resume requires RocksDB backend".to_string(),
    )))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_flow_get_instance_status<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Flow instance status requires RocksDB backend".to_string(),
    )))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_flow_cancel<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Flow cancel requires RocksDB backend".to_string(),
    )))
}
