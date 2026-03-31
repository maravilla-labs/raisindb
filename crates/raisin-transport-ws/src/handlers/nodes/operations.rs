// SPDX-License-Identifier: BSL-1.1

//! Node manipulation handlers: move, rename, copy, copy_tree, reorder,
//! move_child_before, move_child_after.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        NodeCopyPayload, NodeCopyTreePayload, NodeMoveChildAfterPayload,
        NodeMoveChildBeforePayload, NodeMovePayload, NodeRenamePayload, NodeReorderPayload,
        RequestEnvelope, ResponseEnvelope,
    },
};

use super::helpers::{build_node_service, extract_context};

/// Handle node move operation
pub async fn handle_node_move<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeMovePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Note: move_to doesn't support new_name, would need rename after move if new_name provided
    node_service
        .move_to(&payload.from_path, &payload.to_parent_path)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(())?,
    )))
}

/// Handle node rename operation
pub async fn handle_node_rename<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeRenamePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    node_service
        .rename_node(&payload.old_path, &payload.new_name)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(())?,
    )))
}

/// Handle node copy operation
pub async fn handle_node_copy<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeCopyPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // NodeCopyPayload is for shallow copy only
    let result = node_service
        .copy_node_flexible(
            &payload.source_path,
            &payload.target_parent,
            payload.new_name.as_deref(),
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}

/// Handle node copy tree operation
pub async fn handle_node_copy_tree<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeCopyTreePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let result = node_service
        .copy_node_tree_flexible(
            &payload.source_path,
            &payload.target_parent,
            payload.new_name.as_deref(),
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}

/// Handle node reorder operation
pub async fn handle_node_reorder<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeReorderPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Reorder using parent_path, child_name, and position
    // Note: NodeReorderPayload.position is a u32, not a string order_key
    // We need to convert this to the appropriate reorder call
    let child_path = format!("{}/{}", payload.parent_path, payload.child_name);

    // For now, we'll use reorder_to_position if available, or we need a different approach
    // The position field indicates the numeric index position (0-based)
    // This is a simplified implementation - a full implementation would need to:
    // 1. Get all children of parent_path
    // 2. Find the child at position `position`
    // 3. Calculate new order_key between neighbors

    // Placeholder: Using position as a simple indicator
    // TODO: Implement proper position-based reordering
    let result = node_service
        .get_by_path(&child_path)
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Node not found: {}", child_path)))?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}

/// Handle move child before operation
pub async fn handle_node_move_child_before<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeMoveChildBeforePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Use fields directly from payload
    node_service
        .move_child_before(
            &payload.parent_path,
            &payload.child_name,
            &payload.before_child_name,
            None,
            None,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(())?,
    )))
}

/// Handle move child after operation
pub async fn handle_node_move_child_after<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeMoveChildAfterPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Use fields directly from payload
    node_service
        .move_child_after(
            &payload.parent_path,
            &payload.child_name,
            &payload.after_child_name,
            None,
            None,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(())?,
    )))
}
