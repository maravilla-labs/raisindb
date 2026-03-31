// SPDX-License-Identifier: BSL-1.1

//! Query handlers for node operations: query, query_by_path,
//! and query_by_property.

use parking_lot::RwLock;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{NodeQueryPayload, RequestEnvelope, ResponseEnvelope},
};

use super::helpers::{build_node_service, extract_context};

/// Handle node query
#[allow(deprecated)] // TODO(v0.2): Replace list_all() with new query API when available
pub async fn handle_node_query<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeQueryPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // The query is expected to have a "type" field for querying by node type
    // or other query parameters
    let nodes = if let Some(node_type) = payload.query.get("type").and_then(|v| v.as_str()) {
        node_service.list_by_type(node_type).await?
    } else if let Some(parent) = payload.query.get("parent").and_then(|v| v.as_str()) {
        node_service.list_by_parent(parent).await?
    } else {
        // Default to listing all nodes
        node_service.list_all().await?
    };

    // Apply pagination if requested
    let start = payload.offset.unwrap_or(0) as usize;
    let limit = payload.limit.map(|l| l as usize);

    let paginated: Vec<_> = if let Some(limit) = limit {
        nodes.into_iter().skip(start).take(limit).collect()
    } else {
        nodes.into_iter().skip(start).collect()
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(paginated)?,
    )))
}

/// Handle node query by path
pub async fn handle_node_query_by_path<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    // Parse the query payload - expecting a path field
    let query: serde_json::Value = serde_json::from_value(request.payload.clone())?;
    let path = query
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| WsError::InvalidRequest("Path required in query".to_string()))?;

    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // Get node by path
    let node = node_service.get_by_path(path).await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(node)?,
    )))
}

/// Handle node query by property
#[allow(deprecated)] // TODO(v0.2): Replace list_all() with indexed property queries
pub async fn handle_node_query_by_property<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: NodeQueryPayload = serde_json::from_value(request.payload.clone())?;
    let ctx = extract_context(&request)?;
    let node_service = build_node_service(state, connection_state, &ctx);

    // For property queries, we'll need to list all nodes and filter
    // This is not optimal but works for now - a proper implementation
    // would use indexed property queries
    let all_nodes = node_service.list_all().await?;

    // Filter by property key-value pairs in the query
    let filtered: Vec<_> = all_nodes
        .into_iter()
        .filter(|node| {
            // Check if all query properties match
            payload
                .query
                .as_object()
                .map(|obj| {
                    obj.iter().all(|(key, value)| {
                        node.properties
                            .get(key)
                            .map(|prop| {
                                // Simple comparison - convert to JSON for comparison
                                serde_json::to_value(prop).ok() == Some(value.clone())
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
        .collect();

    // Apply pagination if requested
    let start = payload.offset.unwrap_or(0) as usize;
    let limit = payload.limit.map(|l| l as usize);

    let paginated: Vec<_> = if let Some(limit) = limit {
        filtered.into_iter().skip(start).take(limit).collect()
    } else {
        filtered.into_iter().skip(start).collect()
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(paginated)?,
    )))
}
