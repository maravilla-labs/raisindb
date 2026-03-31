// SPDX-License-Identifier: BSL-1.1

//! Archetype management operation handlers

use parking_lot::RwLock;
use raisin_models::nodes::types::Archetype;
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{ArchetypeRepository, CommitMetadata, Storage};
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        ArchetypeCreatePayload, ArchetypeDeletePayload, ArchetypeGetPayload, ArchetypeListPayload,
        ArchetypePublishPayload, ArchetypeUnpublishPayload, ArchetypeUpdatePayload,
        RequestEnvelope, ResponseEnvelope,
    },
};

/// Handle archetype creation
pub async fn handle_archetype_create<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ArchetypeCreatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    // Parse the Archetype from the JSON payload
    let archetype: Archetype = serde_json::from_value(payload.archetype)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid archetype definition: {}", e)))?;

    let commit = CommitMetadata {
        message: format!("Create archetype {}", archetype.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .archetypes()
        .create(
            BranchScope::new(tenant_id, repo, branch),
            archetype.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage
        .archetypes()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &archetype.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Failed to create archetype: {}", archetype.name))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(stored)?,
    )))
}

/// Handle archetype get
pub async fn handle_archetype_get<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ArchetypeGetPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let archetype = state
        .storage
        .archetypes()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            None,
        )
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Archetype not found: {}", payload.name)))?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(archetype)?,
    )))
}

/// Handle archetype list
pub async fn handle_archetype_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ArchetypeListPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let archetypes = if payload.published_only.unwrap_or(false) {
        state
            .storage
            .archetypes()
            .list_published(BranchScope::new(tenant_id, repo, branch), None)
            .await?
    } else {
        state
            .storage
            .archetypes()
            .list(BranchScope::new(tenant_id, repo, branch), None)
            .await?
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(archetypes)?,
    )))
}

/// Handle archetype update
pub async fn handle_archetype_update<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ArchetypeUpdatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    // Parse the Archetype from the JSON payload
    let mut archetype: Archetype = serde_json::from_value(payload.archetype)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid archetype definition: {}", e)))?;

    // Ensure target exists before updating
    let existing = state
        .storage
        .archetypes()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            None,
        )
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Archetype not found: {}", payload.name)))?;

    // Preserve identifiers and creation metadata
    archetype.id = existing.id;
    archetype.name = payload.name.clone();
    archetype.created_at = existing.created_at;

    let commit = CommitMetadata {
        message: format!("Update archetype {}", archetype.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .archetypes()
        .update(
            BranchScope::new(tenant_id, repo, branch),
            archetype.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage
        .archetypes()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &archetype.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Failed to update archetype: {}", archetype.name))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(stored)?,
    )))
}

/// Handle archetype deletion
pub async fn handle_archetype_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ArchetypeDeletePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Delete archetype {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    let deleted = state
        .storage
        .archetypes()
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
            "Archetype not found: {}",
            payload.name
        )))
    }
}

/// Handle archetype publish
pub async fn handle_archetype_publish<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ArchetypePublishPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Publish archetype {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .archetypes()
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

/// Handle archetype unpublish
pub async fn handle_archetype_unpublish<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: ArchetypeUnpublishPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Unpublish archetype {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .archetypes()
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
