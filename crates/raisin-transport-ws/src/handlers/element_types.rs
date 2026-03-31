// SPDX-License-Identifier: BSL-1.1

//! ElementType management operation handlers

use parking_lot::RwLock;
use raisin_models::nodes::element::element_type::ElementType;
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{CommitMetadata, ElementTypeRepository, Storage};
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        ElementTypeCreatePayload, ElementTypeDeletePayload, ElementTypeGetPayload,
        ElementTypeListPayload, ElementTypePublishPayload, ElementTypeUnpublishPayload,
        ElementTypeUpdatePayload, RequestEnvelope, ResponseEnvelope,
    },
};

/// Handle element type creation
pub async fn handle_element_type_create<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ElementTypeCreatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    // Parse the ElementType from the JSON payload
    let element_type: ElementType = serde_json::from_value(payload.element_type)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid element type definition: {}", e)))?;

    let commit = CommitMetadata {
        message: format!("Create element type {}", element_type.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .element_types()
        .create(
            BranchScope::new(tenant_id, repo, branch),
            element_type.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage
        .element_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &element_type.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!(
                "Failed to create element type: {}",
                element_type.name
            ))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(stored)?,
    )))
}

/// Handle element type get
pub async fn handle_element_type_get<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ElementTypeGetPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let element_type = state
        .storage
        .element_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Element type not found: {}", payload.name))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(element_type)?,
    )))
}

/// Handle element type list
pub async fn handle_element_type_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ElementTypeListPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let element_types = if payload.published_only.unwrap_or(false) {
        state
            .storage
            .element_types()
            .list_published(BranchScope::new(tenant_id, repo, branch), None)
            .await?
    } else {
        state
            .storage
            .element_types()
            .list(BranchScope::new(tenant_id, repo, branch), None)
            .await?
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(element_types)?,
    )))
}

/// Handle element type update
pub async fn handle_element_type_update<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ElementTypeUpdatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    // Parse the ElementType from the JSON payload
    let mut element_type: ElementType = serde_json::from_value(payload.element_type)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid element type definition: {}", e)))?;

    // Ensure target exists before updating
    let existing = state
        .storage
        .element_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Element type not found: {}", payload.name))
        })?;

    // Preserve identifiers and creation metadata
    element_type.id = existing.id;
    element_type.name = payload.name.clone();
    element_type.created_at = existing.created_at;

    let commit = CommitMetadata {
        message: format!("Update element type {}", element_type.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .element_types()
        .update(
            BranchScope::new(tenant_id, repo, branch),
            element_type.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage
        .element_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &element_type.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!(
                "Failed to update element type: {}",
                element_type.name
            ))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(stored)?,
    )))
}

/// Handle element type deletion
pub async fn handle_element_type_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ElementTypeDeletePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Delete element type {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    let deleted = state
        .storage
        .element_types()
        .delete(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            commit,
        )
        .await?;

    if deleted.is_some() {
        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({"success": true}),
        )))
    } else {
        Err(WsError::InvalidRequest(format!(
            "Element type not found: {}",
            payload.name
        )))
    }
}

/// Handle element type publish
pub async fn handle_element_type_publish<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ElementTypePublishPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Publish element type {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .element_types()
        .publish(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            commit,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({"success": true}),
    )))
}

/// Handle element type unpublish
pub async fn handle_element_type_unpublish<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ElementTypeUnpublishPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Unpublish element type {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .element_types()
        .unpublish(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            commit,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({"success": true}),
    )))
}
