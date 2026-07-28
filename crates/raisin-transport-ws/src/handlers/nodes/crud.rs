// SPDX-License-Identifier: BSL-1.1

//! CRUD handlers for node operations: create, update, delete, get.

use chrono::Utc;
use parking_lot::RwLock;
use raisin_core::sanitize_name;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::TransactionalStorage;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        AuditQueryPayload, NodeCreateDeepPayload, NodeCreatePayload, NodeDeletePayload,
        NodeGetPayload, NodeHistoryPayload, NodeUpdatePayload, RequestEnvelope, ResponseEnvelope,
    },
};

use super::helpers::{build_node_service, extract_context, json_to_property_value, RequestContext};

/// Handle node creation
pub async fn handle_node_create<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeCreatePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let (parent_path, node) = build_create_node(
        &ctx,
        payload.node_type,
        &payload.path,
        payload.properties,
        payload.content,
    )?;

    // Check if there's an active transaction in the connection state
    // If so, use the transaction context instead of direct NodeService
    let tx_arc_opt = {
        let connection_guard = connection_state.read();
        connection_guard.get_transaction_context()
    }; // Guard is dropped here, before any await

    let created_node = if let Some(tx_ctx) = tx_arc_opt {
        // Use transaction context to create the node
        // Use add_node for create operations (optimized, skips existence checks)
        tx_ctx.add_node(ctx.workspace, &node).await?;
        node
    } else {
        // No active transaction, use regular NodeService with add_node
        // This will validate the NodeType, parent, properties, and publish events centrally
        node_service.add_node(&parent_path, node).await?
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(created_node)?,
    )))
}

/// Build a `Node` from create-style payload fields, returning the resolved parent path
/// and the fully-populated node (path/parent/timestamps/content set). Shared by the
/// `create`, `create_deep` and `upsert_deep` handlers.
fn build_create_node(
    ctx: &RequestContext<'_>,
    node_type: String,
    path: &str,
    properties: HashMap<String, serde_json::Value>,
    content: Option<serde_json::Value>,
) -> Result<(String, Node), WsError> {
    // Extract parent path from the full path
    // For example: "/posts/my-post" -> parent_path="/posts", name="my-post"
    //              "/my-post" -> parent_path="/", name="my-post"
    let (parent_path, node_name) = if let Some(idx) = path.rfind('/') {
        let parent = &path[..idx];
        let name = &path[idx + 1..];
        (if parent.is_empty() { "/" } else { parent }, name)
    } else {
        ("/", path)
    };

    // Sanitize the node name to keep consistency with non-transactional creates
    let clean_name = sanitize_name(node_name).map_err(|err| {
        WsError::InvalidRequest(format!("Invalid node name '{}': {}", node_name, err))
    })?;

    // Convert properties from HashMap<String, serde_json::Value> to HashMap<String, PropertyValue>
    let properties: HashMap<String, PropertyValue> = properties
        .into_iter()
        .map(|(k, v)| (k, json_to_property_value(&v)))
        .collect();

    let mut node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        name: clean_name.clone(),
        path: String::new(),
        node_type,
        archetype: None,
        properties,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: None,
        version: 1,
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: None,
        translations: None,
        tenant_id: Some(ctx.tenant_id.to_string()),
        workspace: Some(ctx.workspace.to_string()),
        owner_id: None,
        relations: vec![],
    };

    // Set full path (used directly for transactional context; overwritten for non-transactional)
    let full_path = if parent_path == "/" || parent_path.is_empty() {
        format!("/{}", clean_name)
    } else {
        format!("{}/{}", parent_path, clean_name)
    };
    node.path = full_path.clone();
    node.parent = Node::extract_parent_name_from_path(&full_path);

    // Ensure timestamps exist so transactional adds persist metadata
    let now = Utc::now();
    node.created_at = Some(now);
    node.updated_at = Some(now);

    // If content is provided, add it to properties
    if let Some(content) = content {
        node.properties
            .insert("content".to_string(), json_to_property_value(&content));
    }

    Ok((parent_path.to_string(), node))
}

/// Handle deep node creation: create the node, auto-creating any missing ancestor folders.
///
/// Unlike `handle_node_create`, this always routes through `NodeService::add_deep_node_typed`
/// (deep ancestor creation isn't supported inside an active WS transaction).
pub async fn handle_node_create_deep<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeCreateDeepPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let folder_type = payload
        .parent_node_type
        .unwrap_or_else(|| "raisin:Folder".to_string());

    let (parent_path, node) = build_create_node(
        &ctx,
        payload.node_type,
        &payload.path,
        payload.properties,
        payload.content,
    )?;

    let created_node = node_service
        .add_deep_node_typed(&parent_path, node, &folder_type)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(created_node)?,
    )))
}

/// Handle deep node upsert: create-or-update by path, auto-creating missing ancestor folders.
pub async fn handle_node_upsert_deep<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeCreateDeepPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let folder_type = payload
        .parent_node_type
        .unwrap_or_else(|| "raisin:Folder".to_string());

    let (parent_path, node) = build_create_node(
        &ctx,
        payload.node_type,
        &payload.path,
        payload.properties,
        payload.content,
    )?;

    let upserted_node = node_service
        .upsert_deep_node(&parent_path, node, &folder_type)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(upserted_node)?,
    )))
}

/// Handle node update
pub async fn handle_node_update<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeUpdatePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Get the existing node
    let mut node = node_service
        .get(&payload.node_id)
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Node not found: {}", payload.node_id)))?;

    // Update properties - convert from JSON to PropertyValue
    for (key, value) in payload.properties {
        node.properties.insert(key, json_to_property_value(&value));
    }

    // Update content if provided
    if let Some(content) = payload.content {
        node.properties
            .insert("content".to_string(), json_to_property_value(&content));
    }

    // Update timestamp
    node.updated_at = Some(chrono::Utc::now());

    // Save the updated node
    node_service.update_node(node.clone()).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(node)?,
    )))
}

/// Handle node deletion
pub async fn handle_node_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeDeletePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Delete the node
    let deleted = node_service.delete(&payload.node_id).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "deleted": deleted }),
    )))
}

/// Handle node get
pub async fn handle_node_get<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeGetPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Get the node
    let node = node_service.get(&payload.node_id).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(node)?,
    )))
}

/// Handle node revision history (git-style "file history")
pub async fn handle_node_history<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeHistoryPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let entries = if let Some(node_id) = payload.node_id.as_deref() {
        node_service.history(node_id, payload.limit).await?
    } else if let Some(path) = payload.path.as_deref() {
        node_service.history_by_path(path, payload.limit).await?
    } else {
        return Err(WsError::InvalidRequest(
            "node_history requires either 'node_id' or 'path'".to_string(),
        ));
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(entries)?,
    )))
}

/// Handle audit-log query: return a node's audit-log entries.
///
/// Audit logs are only produced for NodeTypes marked `auditable` (gated in
/// NodeService). Identify the node by `node_id` or `path`.
pub async fn handle_audit_query<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    use raisin_audit::AuditRepository;

    let payload: AuditQueryPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Resolve the target node through the (RLS-aware) NodeService for BOTH the
    // id and path forms. This authorizes the read: if the caller can't read the
    // node (deleted or RLS-denied), `get`/`get_by_path` returns None and we
    // return empty rather than leaking another node's audit log (IDOR guard).
    let resolved = match (payload.node_id.as_deref(), payload.path.as_deref()) {
        (Some(node_id), _) => node_service.get(node_id).await?,
        (None, Some(path)) => node_service.get_by_path(path).await?,
        (None, None) => {
            return Err(WsError::InvalidRequest(
                "audit_query requires either 'node_id' or 'path'".to_string(),
            ));
        }
    };

    let Some(node) = resolved else {
        return Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!([]),
        )));
    };

    let scope = raisin_audit::AuditScope::new(&ctx.tenant_id, &ctx.repo, &ctx.branch, &ctx.workspace);
    let logs = state
        .audit
        .get_logs_scoped(scope, &node.id, payload.limit.map(|l| l as usize))
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(logs)?,
    )))
}
