// SPDX-License-Identifier: BSL-1.1

//! NodeType management operation handlers

use parking_lot::RwLock;
use raisin_core::{NodeTypeResolver, NodeValidator};
use raisin_models::nodes::types::NodeType;
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{CommitMetadata, NodeTypeRepository, Storage};
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        NodeTypeCreatePayload, NodeTypeDeletePayload, NodeTypeGetPayload,
        NodeTypeGetResolvedPayload, NodeTypeListPayload, NodeTypePublishPayload,
        NodeTypeUnpublishPayload, NodeTypeUpdatePayload, NodeTypeValidatePayload, RequestEnvelope,
        ResponseEnvelope,
    },
};

/// Handle node type creation
pub async fn handle_node_type_create<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeCreatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    // Parse the NodeType from the JSON payload
    let node_type: NodeType = serde_json::from_value(payload.node_type)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid node type definition: {}", e)))?;

    // Validate initial_structure if present
    if node_type.initial_structure.is_some() {
        let storage = state.storage.clone();
        let tenant_id_owned = tenant_id.to_string();
        let repo_owned = repo.to_string();
        let branch_owned = branch.to_string();

        if node_type
            .validate_full(|name| {
                let storage = storage.clone();
                let tenant_id = tenant_id_owned.clone();
                let repo = repo_owned.clone();
                let branch = branch_owned.clone();

                async move {
                    storage
                        .node_types()
                        .get(BranchScope::new(&tenant_id, &repo, &branch), &name, None)
                        .await
                        .map(|opt| opt.is_some())
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .is_err()
        {
            return Err(WsError::InvalidRequest(
                "Invalid initial structure".to_string(),
            ));
        }
    }

    let commit = CommitMetadata {
        message: format!("Create node type {}", node_type.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .node_types()
        .create(
            BranchScope::new(tenant_id, repo, branch),
            node_type.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage
        .node_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &node_type.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Failed to create node type: {}", node_type.name))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(stored)?,
    )))
}

/// Handle node type get
pub async fn handle_node_type_get<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeGetPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let node_type = state
        .storage
        .node_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            None,
        )
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Node type not found: {}", payload.name)))?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(node_type)?,
    )))
}

/// Handle node type list
pub async fn handle_node_type_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeListPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let node_types = if payload.published_only.unwrap_or(false) {
        state
            .storage
            .node_types()
            .list_published(BranchScope::new(tenant_id, repo, branch), None)
            .await?
    } else {
        state
            .storage
            .node_types()
            .list(BranchScope::new(tenant_id, repo, branch), None)
            .await?
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(node_types)?,
    )))
}

/// Handle node type update
pub async fn handle_node_type_update<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeUpdatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    // Parse the NodeType from the JSON payload
    let mut node_type: NodeType = serde_json::from_value(payload.node_type)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid node type definition: {}", e)))?;

    // Ensure target exists before updating
    let existing = state
        .storage
        .node_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &payload.name,
            None,
        )
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Node type not found: {}", payload.name)))?;

    // Preserve identifiers and creation metadata
    node_type.id = existing.id;
    node_type.name = payload.name.clone();
    node_type.created_at = existing.created_at;

    // Validate initial_structure if present
    if node_type.initial_structure.is_some() {
        let storage = state.storage.clone();
        let tenant_id_owned = tenant_id.to_string();
        let repo_owned = repo.to_string();
        let branch_owned = branch.to_string();

        node_type
            .validate_full(|name: String| {
                let storage = storage.clone();
                let tenant_id = tenant_id_owned.clone();
                let repo = repo_owned.clone();
                let branch = branch_owned.clone();

                async move {
                    storage
                        .node_types()
                        .get(BranchScope::new(&tenant_id, &repo, &branch), &name, None)
                        .await
                        .map(|opt| opt.is_some())
                        .map_err(|e| e.to_string())
                }
            })
            .await
            .map_err(|_| WsError::InvalidRequest("Invalid initial structure".to_string()))?;
    }

    let commit = CommitMetadata {
        message: format!("Update node type {}", node_type.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .node_types()
        .update(
            BranchScope::new(tenant_id, repo, branch),
            node_type.clone(),
            commit,
        )
        .await?;

    let stored = state
        .storage
        .node_types()
        .get(
            BranchScope::new(tenant_id, repo, branch),
            &node_type.name,
            None,
        )
        .await?
        .ok_or_else(|| {
            WsError::InvalidRequest(format!("Failed to update node type: {}", node_type.name))
        })?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(stored)?,
    )))
}

/// Handle node type deletion
pub async fn handle_node_type_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeDeletePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Delete node type {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    let deleted = state
        .storage
        .node_types()
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
            "Node type not found: {}",
            payload.name
        )))
    }
}

/// Handle node type publish
pub async fn handle_node_type_publish<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypePublishPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Publish node type {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .node_types()
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

/// Handle node type unpublish
pub async fn handle_node_type_unpublish<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeUnpublishPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let commit = CommitMetadata {
        message: format!("Unpublish node type {}", payload.name),
        actor: "system".to_string(), // TODO: Get actor from connection state
        is_system: true,
    };

    state
        .storage
        .node_types()
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

/// Handle node validation against its node type
pub async fn handle_node_type_validate<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeValidatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    let workspace = request
        .context
        .workspace
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Workspace required".to_string()))?;

    // Parse the node from the JSON payload
    let node: raisin_models::nodes::Node = serde_json::from_value(payload.node)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid node: {}", e)))?;

    let validator = NodeValidator::new(
        state.storage.clone(),
        tenant_id.to_string(),
        repo.to_string(),
        branch.to_string(),
    );

    match validator.validate_node(workspace, &node).await {
        Ok(()) => Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "valid": true,
                "errors": []
            }),
        ))),
        Err(e) => Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "valid": false,
                "errors": [e.to_string()]
            }),
        ))),
    }
}

/// Handle getting resolved node type with full inheritance applied
pub async fn handle_node_type_get_resolved<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeTypeGetResolvedPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");

    let resolver = NodeTypeResolver::new(
        state.storage.clone(),
        tenant_id.to_string(),
        repo.to_string(),
        branch.to_string(),
    );

    let resolved = if let Some(workspace) = request.context.workspace.as_deref() {
        resolver
            .resolve_for_workspace(workspace, &payload.name)
            .await?
    } else {
        resolver.resolve(&payload.name).await?
    };

    // Return as JSON with extra metadata
    let response = serde_json::json!({
        "node_type": resolved.node_type,
        "resolved_properties": resolved.resolved_properties,
        "resolved_allowed_children": resolved.resolved_allowed_children,
        "inheritance_chain": resolved.inheritance_chain,
    });

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        response,
    )))
}
