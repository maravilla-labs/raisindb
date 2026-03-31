// SPDX-License-Identifier: BSL-1.1

//! Event subscription handlers

use parking_lot::RwLock;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        RequestEnvelope, ResponseEnvelope, SubscribePayload, SubscriptionResponse,
        UnsubscribePayload,
    },
};

/// Handle event subscription
///
/// Supports deduplication: if a subscription with identical filters already exists,
/// returns the existing subscription_id instead of creating a new one.
pub async fn handle_subscribe<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: SubscribePayload = serde_json::from_value(request.payload.clone())?;

    // Generate candidate subscription ID
    let candidate_id = Uuid::new_v4().to_string();

    // Extract workspace filter for indexing (before moving payload)
    let workspace_filter = payload.filters.workspace.clone();

    // Add subscription to connection state (returns existing ID if duplicate)
    let (connection_id, actual_id, is_new) = {
        let conn = connection_state.write();
        let conn_id = conn.connection_id.clone();
        let actual_id = conn.add_subscription(candidate_id.clone(), payload.filters);
        let is_new = actual_id == candidate_id;
        (conn_id, actual_id, is_new)
    };

    // Only update workspace subscription index for NEW subscriptions
    // Duplicates already have their workspace indexed
    if is_new {
        state
            .connection_registry
            .add_workspace_subscription(&connection_id, workspace_filter.as_deref());
    }

    let response = SubscriptionResponse {
        subscription_id: actual_id,
    };

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::to_value(response)?,
    )))
}

/// Handle event unsubscription
pub async fn handle_unsubscribe<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage,
    B: raisin_binary::BinaryStorage,
{
    let payload: UnsubscribePayload = serde_json::from_value(request.payload.clone())?;

    // Get subscription workspace and remove it from connection state
    let (removed, connection_id, workspace) = {
        let conn = connection_state.write();
        let conn_id = conn.connection_id.clone();
        // Get the subscription's workspace filter before removing
        let workspace = conn
            .get_subscriptions()
            .iter()
            .find(|(id, _)| id == &payload.subscription_id)
            .and_then(|(_, filters)| filters.workspace.clone());
        let removed = conn.remove_subscription(&payload.subscription_id);
        (removed, conn_id, workspace)
    };

    if removed {
        // Update workspace subscription index
        state
            .connection_registry
            .remove_workspace_subscription(&connection_id, workspace.as_deref());

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({ "unsubscribed": true }),
        )))
    } else {
        Err(WsError::InvalidRequest(format!(
            "Subscription {} not found",
            payload.subscription_id
        )))
    }
}
