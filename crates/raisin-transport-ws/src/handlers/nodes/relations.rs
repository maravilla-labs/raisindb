// SPDX-License-Identifier: BSL-1.1

//! Relation handlers: add, remove, and get relationships between nodes.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        RelationAddPayload, RelationRemovePayload, RelationsGetPayload, RequestEnvelope,
        ResponseEnvelope,
    },
};

use super::helpers::{build_node_service, extract_context};

/// Handle add relation operation
pub async fn handle_relation_add<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RelationAddPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    node_service
        .add_relation(
            &payload.source_path,
            &payload.target_workspace,
            &payload.target_path,
            payload.weight,
            payload
                .relation_type
                .clone()
                .or_else(|| Some("related".to_string())),
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "success": true }),
    )))
}

/// Handle remove relation operation
pub async fn handle_relation_remove<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RelationRemovePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    node_service
        .remove_relation(
            &payload.source_path,
            &payload.target_workspace,
            &payload.target_path,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "success": true }),
    )))
}

/// Handle get relationships operation
pub async fn handle_relations_get<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: RelationsGetPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // get_node_relationships only takes node_path
    let result = node_service
        .get_node_relationships(&payload.node_path)
        .await?;

    // Note: The protocol doesn't support filtering by direction or relation_type in RelationsGetPayload
    // If filtering is needed, it should be added to the payload definition

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}
