// SPDX-License-Identifier: BSL-1.1

//! Workspace operation handlers

use parking_lot::RwLock;
use raisin_models::timestamp::StorageTimestamp;
use raisin_models::workspace::Workspace;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        RequestEnvelope, ResponseEnvelope, WorkspaceCreatePayload, WorkspaceDeletePayload,
        WorkspaceGetPayload, WorkspaceUpdatePayload,
    },
};

/// Handle workspace creation
pub async fn handle_workspace_create<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: WorkspaceCreatePayload = serde_json::from_value(request.payload.clone())?;

    // Extract and validate context
    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Create a new workspace object
    let mut workspace = Workspace::new(payload.name);
    workspace.description = payload.description;

    // Save the workspace using WorkspaceService
    state.ws_svc.put(tenant_id, repo, workspace.clone()).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(workspace)?,
    )))
}

/// Handle workspace get
pub async fn handle_workspace_get<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: WorkspaceGetPayload = serde_json::from_value(request.payload.clone())?;

    // Extract and validate context
    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Get the workspace using WorkspaceService
    let workspace = state
        .ws_svc
        .get(tenant_id, repo, &payload.name)
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Workspace not found: {}", payload.name)))?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(workspace)?,
    )))
}

/// Handle workspace list
pub async fn handle_workspace_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    // Extract and validate context
    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // List all workspaces using WorkspaceService
    let mut workspaces = state.ws_svc.list(tenant_id, repo).await?;

    // Sort by name for consistent ordering
    workspaces.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(workspaces)?,
    )))
}

/// Handle workspace deletion
pub async fn handle_workspace_delete<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    // Parse payload for validation even though we don't support deletion
    let _payload: WorkspaceDeletePayload = serde_json::from_value(request.payload.clone())?;

    // Workspace deletion is not currently supported in the storage layer
    // Workspaces are intended to be permanent once created
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_SUPPORTED".to_string(),
        "Workspace deletion is not supported. Workspaces are permanent once created.".to_string(),
    )))
}

/// Handle workspace update
pub async fn handle_workspace_update<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: WorkspaceUpdatePayload = serde_json::from_value(request.payload.clone())?;

    // Extract and validate context
    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

    // Get the existing workspace
    let mut workspace = state
        .ws_svc
        .get(tenant_id, repo, &payload.name)
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Workspace not found: {}", payload.name)))?;

    // Update fields if provided
    if let Some(description) = payload.description {
        workspace.description = Some(description);
    }
    if let Some(allowed_node_types) = payload.allowed_node_types {
        workspace.allowed_node_types = allowed_node_types;
    }
    if let Some(allowed_root_node_types) = payload.allowed_root_node_types {
        workspace.allowed_root_node_types = allowed_root_node_types;
    }

    // Update timestamp
    workspace.updated_at = Some(StorageTimestamp::now());

    // Save the updated workspace
    state.ws_svc.put(tenant_id, repo, workspace.clone()).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(workspace)?,
    )))
}
