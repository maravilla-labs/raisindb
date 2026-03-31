// SPDX-License-Identifier: BSL-1.1

//! Property path handlers: get and update properties by dot-path.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{PropertyGetPayload, PropertyUpdatePayload, RequestEnvelope, ResponseEnvelope},
};

use super::helpers::{build_node_service, extract_context, json_to_property_value};

/// Handle get property by path operation
pub async fn handle_property_get<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: PropertyGetPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let result = node_service
        .get_property_by_path(&payload.node_path, &payload.property_path)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(result)?,
    )))
}

/// Handle update property by path operation
pub async fn handle_property_update<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: PropertyUpdatePayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    let property_value = json_to_property_value(&payload.value);

    node_service
        .update_property_by_path(&payload.node_path, &payload.property_path, property_value)
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(())?,
    )))
}
