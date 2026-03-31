// SPDX-License-Identifier: BSL-1.1

//! Transaction operation handlers

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope, TransactionBeginPayload},
};

/// Handle transaction begin operation
pub async fn handle_transaction_begin<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TransactionBeginPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    // Begin transaction context
    let ctx = state.storage.begin_context().await?;

    // Set tenant and repository
    ctx.set_tenant_repo(tenant_id, repo)?;

    // Set branch
    ctx.set_branch(branch)?;

    // Set actor (user ID from connection state or system)
    {
        let conn = connection_state.read();
        let actor = conn.user_id.as_deref().unwrap_or("system");
        ctx.set_actor(actor)?;
    }

    // SECURITY: Set auth context for RLS enforcement
    // This ensures all operations within the transaction are subject to
    // the same permission checks as non-transactional operations
    {
        let conn = connection_state.read();
        if let Some(auth) = conn.auth_context() {
            ctx.set_auth_context(auth.clone())?;
        }
    }

    // Set commit message if provided
    if let Some(message) = &payload.message {
        ctx.set_message(message)?;
    }

    // Store transaction context in connection state
    {
        let mut conn = connection_state.write();
        conn.set_transaction_context(ctx)?;
    }

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "status": "transaction_started",
        }),
    )))
}

/// Handle transaction commit operation
pub async fn handle_transaction_commit<S, B>(
    _state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    // Get and remove transaction context from connection state
    let ctx = {
        let mut conn = connection_state.write();
        conn.take_transaction_context()
            .ok_or_else(|| WsError::InvalidRequest("No active transaction".to_string()))?
    };

    // Commit the transaction
    ctx.commit().await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "status": "transaction_committed",
        }),
    )))
}

/// Handle transaction rollback operation
pub async fn handle_transaction_rollback<S, B>(
    _state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    // Get and remove transaction context from connection state
    let ctx = {
        let mut conn = connection_state.write();
        conn.take_transaction_context()
            .ok_or_else(|| WsError::InvalidRequest("No active transaction".to_string()))?
    };

    // Rollback the transaction
    ctx.rollback().await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "status": "transaction_rolled_back",
        }),
    )))
}
