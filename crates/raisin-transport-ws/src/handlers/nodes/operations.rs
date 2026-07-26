// SPDX-License-Identifier: BSL-1.1

//! Node manipulation handlers: move, rename, copy, copy_tree, reorder,
//! move_child_before, move_child_after.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        NodeApplyChildOrderPayload, NodeCopyPayload, NodeCopyTreePayload,
        NodeMoveChildAfterPayload, NodeMoveChildBeforePayload, NodeMovePayload, NodeRenamePayload,
        NodeReorderPayload, RequestEnvelope, ResponseEnvelope,
    },
};

use super::helpers::{build_node_service, connection_actor, extract_context};

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
    let actor = connection_actor(connection_state);

    // `NodeService::reorder_child` owns the position→fractional-label conversion
    // (read the siblings, find the neighbours at `position`, mint a label between
    // them). This handler used to be a stub that looked the node up and returned
    // it unchanged, so `node_reorder` reported success while changing nothing.
    node_service
        .reorder_child(
            &payload.parent_path,
            &payload.child_name,
            payload.position as usize,
            Some("Reorder child"),
            actor.as_deref(),
        )
        .await?;

    // Return the node as it now stands, so the caller sees the assigned
    // `order_key` rather than having to re-read.
    let child_path = format!(
        "{}/{}",
        payload.parent_path.trim_end_matches('/'),
        payload.child_name
    );
    let reordered = node_service
        .get_by_path(&child_path)
        .await?
        .ok_or_else(|| WsError::InvalidRequest(format!("Node not found: {}", child_path)))?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(reordered)?,
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
    let actor = connection_actor(connection_state);

    node_service
        .move_child_before(
            &payload.parent_path,
            &payload.child_name,
            &payload.before_child_name,
            Some("Move child before sibling"),
            actor.as_deref(),
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
    let actor = connection_actor(connection_state);

    node_service
        .move_child_after(
            &payload.parent_path,
            &payload.child_name,
            &payload.after_child_name,
            Some("Move child after sibling"),
            actor.as_deref(),
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(())?,
    )))
}

/// Handle "apply child order from another branch" — replays `source_branch`'s
/// sibling order for a parent onto the request's (target) branch. The primitive
/// a selective publish uses to carry node order across branches.
pub async fn handle_node_apply_child_order<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeApplyChildOrderPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;

    raisin_storage::apply_child_order_from_branch(
        state.storage.nodes(),
        ctx.tenant_id,
        ctx.repo,
        ctx.branch, // target = the request's branch
        &payload.source_branch,
        ctx.workspace,
        &payload.parent_path,
        None,
    )
    .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(())?,
    )))
}
