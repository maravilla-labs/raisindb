// SPDX-License-Identifier: BSL-1.1

//! Tag management operation handlers

use parking_lot::RwLock;
use raisin_hlc::HLC;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{Storage, TagRepository};
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        RequestEnvelope, ResponseEnvelope, TagCreatePayload, TagDeletePayload, TagGetPayload,
        TagListPayload,
    },
};

/// Handle tag creation
pub async fn handle_tag_create<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TagCreatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Parse revision from HLC string format
    let revision: HLC = payload
        .revision
        .parse()
        .map_err(|e| WsError::InvalidRequest(format!("Invalid revision: {}", e)))?;

    let tag = state
        .storage
        .tags()
        .create_tag(
            tenant_id,
            repo,
            &payload.name,
            &revision,
            "system", // TODO: Get actor from connection state
            payload.message,
            false, // protected - TODO: Add to payload if needed
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(tag)?,
    )))
}

/// Handle tag get
pub async fn handle_tag_get<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TagGetPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    let tag = state
        .storage
        .tags()
        .get_tag(tenant_id, repo, &payload.name)
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Tag not found: {}", payload.name)))?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(tag)?,
    )))
}

/// Handle tag list
pub async fn handle_tag_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let _payload: TagListPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    let tags = state.storage.tags().list_tags(tenant_id, repo).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(tags)?,
    )))
}

/// Handle tag deletion
pub async fn handle_tag_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TagDeletePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    let deleted = state
        .storage
        .tags()
        .delete_tag(tenant_id, repo, &payload.name)
        .await?;

    if deleted {
        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({"success": true}),
        )))
    } else {
        Err(WsError::InvalidRequest(format!(
            "Tag not found: {}",
            payload.name
        )))
    }
}
