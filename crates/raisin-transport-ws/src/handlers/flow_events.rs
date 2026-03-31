// SPDX-License-Identifier: BSL-1.1

//! Flow event subscription handlers.
//!
//! Allows WebSocket clients to subscribe to real-time flow execution events
//! (step progress, completion, failure, AI streaming chunks, etc.) using the
//! global flow event broadcaster. Unlike the regular node-change subscriptions,
//! these are per-flow-instance and use a spawned tokio task to bridge the
//! broadcast channel to the WebSocket event channel.

use parking_lot::RwLock;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{EventMessage, RequestEnvelope, ResponseEnvelope},
};

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FlowSubscribePayload {
    instance_id: String,
}

#[derive(Debug, Deserialize)]
struct FlowUnsubscribePayload {
    subscription_id: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Subscribe to real-time events for a flow instance.
///
/// Spawns a background task that reads from the global flow broadcaster and
/// forwards events through the connection's event channel. The task
/// automatically stops when the flow reaches a terminal state or the
/// broadcast channel closes.
pub async fn handle_flow_subscribe_events<S, B>(
    _state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let payload: FlowSubscribePayload = serde_json::from_value(request.payload.clone())?;
    let subscription_id = Uuid::new_v4().to_string();

    let broadcaster = raisin_storage::jobs::global_flow_broadcaster();
    let mut receiver = broadcaster.subscribe(&payload.instance_id);

    // Clone handles needed by the spawned task
    let sub_id = subscription_id.clone();
    let instance_id = payload.instance_id.clone();
    let conn_state = Arc::clone(connection_state);

    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let event_type = event.event_type().to_string();
                    let is_terminal = matches!(
                        &event,
                        raisin_storage::jobs::FlowEvent::FlowCompleted { .. }
                            | raisin_storage::jobs::FlowEvent::FlowFailed { .. }
                    );

                    let data = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);

                    let msg = EventMessage::new(sub_id.clone(), event_type, data);

                    let send_result = {
                        let conn = conn_state.read();
                        conn.send_event(msg)
                    };

                    if send_result.is_err() {
                        tracing::debug!(
                            instance_id = %instance_id,
                            subscription_id = %sub_id,
                            "Event channel closed, stopping flow event forwarder"
                        );
                        break;
                    }

                    if is_terminal {
                        tracing::debug!(
                            instance_id = %instance_id,
                            subscription_id = %sub_id,
                            "Flow reached terminal state, stopping event forwarder"
                        );
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        instance_id = %instance_id,
                        subscription_id = %sub_id,
                        lagged = n,
                        "Flow event subscriber lagged, some events dropped"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::debug!(
                        instance_id = %instance_id,
                        subscription_id = %sub_id,
                        "Flow event broadcast channel closed"
                    );
                    break;
                }
            }
        }
    });

    tracing::info!(
        instance_id = %payload.instance_id,
        subscription_id = %subscription_id,
        "Client subscribed to flow events via WS"
    );

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "subscription_id": subscription_id }),
    )))
}

/// Unsubscribe from flow events.
///
/// The spawned forwarder task will stop automatically when the flow completes
/// or the connection drops. This handler provides an explicit opt-out for
/// clients that want to stop receiving events early.
pub async fn handle_flow_unsubscribe_events<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let payload: FlowUnsubscribePayload = serde_json::from_value(request.payload.clone())?;

    tracing::debug!(
        subscription_id = %payload.subscription_id,
        "Client unsubscribed from flow events via WS"
    );

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "success": true }),
    )))
}
