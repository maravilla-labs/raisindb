// SPDX-License-Identifier: BSL-1.1

//! Tree traversal handlers: list_children, get_tree (nested), get_tree_flat.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        NodeGetTreeFlatPayload, NodeGetTreePayload, NodeListChildrenPayload, RequestEnvelope,
        ResponseEnvelope,
    },
};

use super::helpers::{build_node_service, extract_context};

/// Handle list children operation
pub async fn handle_node_list_children<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeListChildrenPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Parse cursor if provided (it's JSON-serialized PageCursor)
    let cursor: Option<raisin_models::tree::PageCursor> = payload
        .cursor
        .as_ref()
        .and_then(|c| serde_json::from_str(c).ok());

    let limit = payload.limit.unwrap_or(50) as usize;

    let result = node_service
        .list_children_page(&payload.parent_path, cursor.as_ref(), limit)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "nodes": result.items,
            "next_cursor": result.next_cursor,
        }),
    )))
}

/// Handle get tree operation (nested children)
pub async fn handle_node_get_tree<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeGetTreePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let result = node_service
        .deep_children_nested(&payload.parent_path, payload.max_depth.unwrap_or(u32::MAX))
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}

/// Handle get tree flat operation
pub async fn handle_node_get_tree_flat<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeGetTreeFlatPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let result = node_service
        .deep_children_flat(&payload.parent_path, payload.max_depth.unwrap_or(u32::MAX))
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}
