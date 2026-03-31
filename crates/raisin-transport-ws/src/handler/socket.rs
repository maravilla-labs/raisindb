// SPDX-License-Identifier: BSL-1.1

//! WebSocket connection lifecycle management.
//!
//! Handles the connection lifecycle including authentication, message routing,
//! and cleanup on disconnect.

use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use futures::{stream::StreamExt, SinkExt};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::{
    connection::ConnectionState,
    protocol::{EventMessage, RequestEnvelope, ResponseEnvelope},
};
use raisin_models::auth::AuthContext;

use super::auth_token::authenticate_with_token;
use super::request::process_request;
use super::state::WsState;

/// Handle a single WebSocket connection
pub(super) async fn handle_socket<S, B>(
    socket: WebSocket,
    state: Arc<WsState<S, B>>,
    initial_token: Option<String>,
    tenant_id: String,
    repository: Option<String>,
) where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Try to authenticate from initial token
    let connection_state = if let Some(token) = initial_token {
        match authenticate_with_token(&state, &token).await {
            Ok(conn_state) => {
                info!(
                    connection_id = %conn_state.connection_id,
                    tenant_id = %conn_state.tenant_id,
                    "WebSocket connection authenticated via header"
                );
                conn_state
            }
            Err(e) => {
                warn!("Authentication failed: {}", e);
                let _ = ws_sender.send(Message::Close(None)).await;
                return;
            }
        }
    } else {
        // Check if anonymous access is enabled for this tenant/repo
        let repo_id = repository.clone().unwrap_or_else(|| "default".to_string());
        let anonymous_enabled = state.is_anonymous_enabled(&tenant_id, &repo_id).await;

        if anonymous_enabled {
            create_anonymous_connection(&state, &tenant_id, repository.clone()).await
        } else {
            // SECURITY: Anonymous access disabled - set deny-all context
            let mut conn = ConnectionState::new(
                tenant_id.clone(),
                repository.clone(),
                state.config.max_concurrent_ops,
                state.config.initial_credits,
            );
            conn.set_auth_context(AuthContext::deny_all());
            debug!(
                connection_id = %conn.connection_id,
                tenant_id = %tenant_id,
                "WebSocket connection created with deny-all context, awaiting authentication"
            );
            conn
        }
    };

    // Create channels for sending responses and events
    let (response_tx, mut response_rx) = mpsc::unbounded_channel::<ResponseEnvelope>();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<EventMessage>();

    connection_state.set_response_channel(response_tx);
    connection_state.set_event_channel(event_tx);

    let connection_state = Arc::new(parking_lot::RwLock::new(connection_state));

    // Register the connection in the global registry for event forwarding
    state
        .connection_registry
        .register(Arc::clone(&connection_state));

    // Send initial "connected" message with anonymous token if available
    send_connected_message(&connection_state, &mut ws_sender).await;

    // Spawn task to send responses back to client
    let ws_sender_handle = spawn_sender_task(ws_sender, &mut response_rx, &mut event_rx);

    // Process incoming messages
    process_incoming_messages(&mut ws_receiver, &state, &connection_state).await;

    // Cleanup
    let connection_id = {
        let conn = connection_state.read();
        let id = conn.connection_id.clone();
        info!(connection_id = %id, "WebSocket connection closed");
        conn.cleanup();
        id
    };

    // Unregister the connection from the global registry
    state.connection_registry.unregister(&connection_id);

    // Wait for sender task to finish
    ws_sender_handle.abort();
}

/// Create an anonymous connection state with resolved permissions.
async fn create_anonymous_connection<S, B>(
    state: &WsState<S, B>,
    tenant_id: &str,
    repository: Option<String>,
) -> ConnectionState
where
    S: raisin_storage::Storage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let repo_id = repository.clone().unwrap_or_else(|| "default".to_string());
    let permission_service = raisin_core::PermissionService::new(state.storage.clone());
    let resolved_permissions = permission_service
        .resolve_anonymous_user(tenant_id, &repo_id, "main")
        .await
        .unwrap_or_else(|e| {
            warn!(
                error = %e,
                "Failed to resolve anonymous user permissions, using empty permissions"
            );
            None
        })
        .unwrap_or_else(|| {
            warn!("Physical anonymous user not found, using empty permissions");
            raisin_models::permissions::ResolvedPermissions::anonymous(vec![])
        });

    let mut conn = ConnectionState::new(
        tenant_id.to_string(),
        repository.clone(),
        state.config.max_concurrent_ops,
        state.config.initial_credits,
    );

    // Use the user_id from resolved permissions (should be the anonymous user's node ID)
    let user_id = resolved_permissions.user_id.clone();
    conn.set_user_id(user_id.clone());

    // Use AuthContext::for_user for the physical anonymous user (not anonymous())
    conn.set_auth_context(AuthContext::for_user(&user_id).with_permissions(resolved_permissions));

    // Generate anonymous JWT token for HTTP API calls
    let anonymous_token = state
        .auth_service
        .generate_token_pair(user_id.clone(), tenant_id.to_string(), repository.clone())
        .ok();

    if let Some(ref token_pair) = anonymous_token {
        conn.set_anonymous_token(Some(token_pair.access_token.clone()));
    }

    info!(
        connection_id = %conn.connection_id,
        tenant_id = %tenant_id,
        repository = ?repository,
        user_id = %user_id,
        has_token = anonymous_token.is_some(),
        "WebSocket connection auto-authenticated as physical anonymous user with resolved permissions"
    );
    conn
}

/// Send the initial "connected" message to the client.
async fn send_connected_message(
    connection_state: &Arc<parking_lot::RwLock<ConnectionState>>,
    ws_sender: &mut futures::stream::SplitSink<WebSocket, Message>,
) {
    let (connection_id, anonymous_token, user_id) = {
        let conn = connection_state.read();
        (
            conn.connection_id.clone(),
            conn.anonymous_token().cloned(),
            conn.user_id.clone(),
        )
    };

    let connected_message = serde_json::json!({
        "type": "connected",
        "connection_id": connection_id,
        "anonymous": anonymous_token.is_some(),
        "token": anonymous_token,
        "user_id": user_id,
    });

    if let Ok(data) = rmp_serde::encode::to_vec_named(&connected_message) {
        if let Err(e) = ws_sender.send(Message::Binary(Bytes::from(data))).await {
            error!("Failed to send connected message: {}", e);
        } else {
            debug!(
                connection_id = %connection_id,
                has_token = anonymous_token.is_some(),
                "Sent connected message to client"
            );
        }
    }
}

/// Spawn the task that sends responses and events back to the client.
fn spawn_sender_task(
    mut ws_sender: futures::stream::SplitSink<WebSocket, Message>,
    response_rx: &mut mpsc::UnboundedReceiver<ResponseEnvelope>,
    event_rx: &mut mpsc::UnboundedReceiver<EventMessage>,
) -> tokio::task::JoinHandle<()> {
    // We need to take ownership of the receivers
    let mut response_rx_owned = std::mem::replace(response_rx, mpsc::unbounded_channel().1);
    let mut event_rx_owned = std::mem::replace(event_rx, mpsc::unbounded_channel().1);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(response) = response_rx_owned.recv() => {
                    tracing::info!("Received response from channel - request_id: {}, status: {:?}", response.request_id, response.status);

                    match rmp_serde::encode::to_vec_named(&response) {
                        Ok(data) => {
                            tracing::info!("Serialized response to MessagePack - size: {} bytes, request_id: {}", data.len(), response.request_id);
                            match ws_sender.send(Message::Binary(Bytes::from(data))).await {
                                Ok(_) => {
                                    tracing::info!("Sent response over WebSocket - request_id: {}", response.request_id);
                                }
                                Err(e) => {
                                    error!("Failed to send response over WebSocket: {}", e);
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to serialize response: {}", e);
                        }
                    }
                }
                Some(event) = event_rx_owned.recv() => {
                    match rmp_serde::encode::to_vec_named(&event) {
                        Ok(data) => {
                            if let Err(e) = ws_sender.send(Message::Binary(Bytes::from(data))).await {
                                error!("Failed to send event: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Failed to serialize event: {}", e);
                        }
                    }
                }
                else => break,
            }
        }

        let _ = ws_sender.send(Message::Close(None)).await;
    })
}

/// Process incoming WebSocket messages until the connection closes.
async fn process_incoming_messages<S, B>(
    ws_receiver: &mut futures::stream::SplitStream<WebSocket>,
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<parking_lot::RwLock<ConnectionState>>,
) where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    while let Some(msg) = ws_receiver.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                tracing::info!("Received binary message - size: {} bytes", data.len());

                if data.is_empty() {
                    tracing::debug!("Received empty binary message (heartbeat ping)");
                    continue;
                }

                tracing::info!(
                    "Attempting to deserialize MessagePack request - size: {} bytes",
                    data.len()
                );

                match rmp_serde::from_slice::<RequestEnvelope>(&data) {
                    Ok(request) => {
                        tracing::info!(
                            "Successfully deserialized request - ID: {}, type: {:?}",
                            request.request_id,
                            request.request_type
                        );

                        let state = Arc::clone(state);
                        let connection_state = Arc::clone(connection_state);

                        let has_transaction = {
                            let conn = connection_state.read();
                            conn.has_active_transaction()
                        };

                        if has_transaction {
                            tracing::info!(
                                "Processing transactional request inline - ID: {}",
                                request.request_id
                            );
                            process_request(state, connection_state, request).await;
                        } else {
                            tracing::info!(
                                "Spawning async task to process request - ID: {}",
                                request.request_id
                            );
                            tokio::spawn(async move {
                                tracing::info!(
                                    "Processing request in async task - ID: {}",
                                    request.request_id
                                );
                                process_request(state, connection_state, request).await;
                                tracing::info!("Finished processing request in async task");
                            });
                        }
                    }
                    Err(e) => {
                        error!("Failed to deserialize request: {}", e);
                    }
                }
            }
            Ok(Message::Text(text)) => {
                warn!("Received unexpected text message: {}", text);
            }
            Ok(Message::Close(_)) => {
                info!("Client closed connection");
                break;
            }
            Ok(Message::Ping(_)) => {
                debug!("Received ping");
            }
            Ok(Message::Pong(_)) => {
                debug!("Received pong");
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
        }
    }
}
