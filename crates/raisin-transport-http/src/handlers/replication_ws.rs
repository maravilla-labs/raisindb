//! WebSocket-based real-time replication
//!
//! This module provides push-based real-time operation synchronization using WebSockets.
//! It complements the existing HTTP-based periodic sync with instant notifications.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::state::AppState;

#[cfg(feature = "storage-rocksdb")]
use raisin_replication::{Operation, VectorClock};

/// WebSocket message types for replication
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsMessage {
    /// Push a new operation to the peer
    PushOperation {
        operation: Operation,
    },
    /// Batch of operations
    PushOperationsBatch {
        operations: Vec<Operation>,
    },
    /// Acknowledge received operations
    Ack {
        operation_ids: Vec<String>,
    },
    /// Request vector clock
    GetVectorClock,
    /// Vector clock response
    VectorClock {
        vector_clock: VectorClock,
    },
    /// Heartbeat/ping
    Ping,
    /// Heartbeat response
    Pong,
    /// Error message
    Error {
        message: String,
    },
}

/// WebSocket upgrade handler for real-time replication
///
/// GET /api/replication/:tenant/:repo/ws
///
/// Establishes a bidirectional WebSocket connection for real-time operation push.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path((tenant_id, repo_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        "WebSocket replication connection requested"
    );

    ws.on_upgrade(move |socket| handle_socket(socket, tenant_id, repo_id, state))
}

/// Handle an individual WebSocket connection
async fn handle_socket(socket: WebSocket, tenant_id: String, repo_id: String, state: AppState) {
    info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        "WebSocket replication connection established"
    );

    let (mut sender, mut receiver) = socket.split();

    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::state::get_rocksdb_from_state;
        use raisin_rocksdb::OpLogRepository;

        let db = match get_rocksdb_from_state(&state) {
            Ok(db) => db,
            Err(e) => {
                error!(error = %e, "Failed to get RocksDB instance");
                let _ = sender
                    .send(Message::Close(None))
                    .await;
                return;
            }
        };

        let oplog_repo = OpLogRepository::new(db);

        // Main message loop
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<WsMessage>(&text) {
                        Ok(ws_msg) => {
                            if let Err(e) = handle_ws_message(
                                &mut sender,
                                ws_msg,
                                &tenant_id,
                                &repo_id,
                                &oplog_repo,
                            )
                            .await
                            {
                                error!(error = %e, "Error handling WebSocket message");

                                let error_msg = WsMessage::Error {
                                    message: e.to_string(),
                                };

                                if let Ok(json) = serde_json::to_string(&error_msg) {
                                    let _ = sender.send(Message::Text(json)).await;
                                }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to parse WebSocket message");
                        }
                    }
                }
                Ok(Message::Binary(_)) => {
                    warn!("Received unexpected binary message");
                }
                Ok(Message::Ping(data)) => {
                    let _ = sender.send(Message::Pong(data)).await;
                }
                Ok(Message::Pong(_)) => {
                    // Pong received
                }
                Ok(Message::Close(_)) => {
                    info!("WebSocket connection closed by peer");
                    break;
                }
                Err(e) => {
                    error!(error = %e, "WebSocket error");
                    break;
                }
            }
        }
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        let _ = sender;
        let _ = receiver;
        error!("WebSocket replication requires storage-rocksdb feature");
    }

    info!(
        tenant_id = %tenant_id,
        repo_id = %repo_id,
        "WebSocket replication connection closed"
    );
}

#[cfg(feature = "storage-rocksdb")]
async fn handle_ws_message(
    sender: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: WsMessage,
    tenant_id: &str,
    repo_id: &str,
    oplog_repo: &raisin_rocksdb::OpLogRepository,
) -> Result<()> {
    use raisin_replication::ReplayEngine;

    match msg {
        WsMessage::PushOperation { operation } => {
            debug!(
                op_id = %operation.op_id,
                op_seq = operation.op_seq,
                "Received operation via WebSocket"
            );

            // Apply operation using CRDT replay engine
            let mut replay_engine = ReplayEngine::new();
            let result = replay_engine.replay(vec![operation.clone()]);

            if !result.applied.is_empty() {
                // Store applied operations
                oplog_repo.put_operations_batch(&result.applied)?;

                // Send acknowledgment
                let ack_msg = WsMessage::Ack {
                    operation_ids: vec![operation.op_id.to_string()],
                };
                let json = serde_json::to_string(&ack_msg)?;
                sender.send(Message::Text(json)).await
                    .map_err(|e| raisin_error::Error::Backend(format!("WebSocket send error: {}", e)))?;
            }

            Ok(())
        }
        WsMessage::PushOperationsBatch { operations } => {
            debug!(count = operations.len(), "Received operation batch via WebSocket");

            // Apply operations using CRDT replay engine
            let mut replay_engine = ReplayEngine::new();
            let result = replay_engine.replay(operations.clone());

            if !result.applied.is_empty() {
                // Store applied operations
                oplog_repo.put_operations_batch(&result.applied)?;

                // Send acknowledgment
                let ack_msg = WsMessage::Ack {
                    operation_ids: result
                        .applied
                        .iter()
                        .map(|op| op.op_id.to_string())
                        .collect(),
                };
                let json = serde_json::to_string(&ack_msg)?;
                sender.send(Message::Text(json)).await
                    .map_err(|e| raisin_error::Error::Backend(format!("WebSocket send error: {}", e)))?;
            }

            Ok(())
        }
        WsMessage::GetVectorClock => {
            // Get current vector clock
            let vc = oplog_repo.get_vector_clock_snapshot(tenant_id, repo_id)?;

            let response = WsMessage::VectorClock {
                vector_clock: vc,
            };
            let json = serde_json::to_string(&response)?;
            sender.send(Message::Text(json)).await
                .map_err(|e| raisin_error::Error::Backend(format!("WebSocket send error: {}", e)))?;

            Ok(())
        }
        WsMessage::Ping => {
            let response = WsMessage::Pong;
            let json = serde_json::to_string(&response)?;
            sender.send(Message::Text(json)).await
                .map_err(|e| raisin_error::Error::Backend(format!("WebSocket send error: {}", e)))?;
            Ok(())
        }
        WsMessage::Ack { operation_ids } => {
            debug!(count = operation_ids.len(), "Received acknowledgment");
            // Operation was successfully received by peer
            // Could update metrics or peer watermarks here
            Ok(())
        }
        WsMessage::VectorClock { vector_clock } => {
            debug!(vc = ?vector_clock, "Received vector clock");
            // Could store or compare vector clocks
            Ok(())
        }
        WsMessage::Pong => {
            // Heartbeat response received
            Ok(())
        }
        WsMessage::Error { message } => {
            warn!(error = %message, "Received error from peer");
            Ok(())
        }
    }
}
