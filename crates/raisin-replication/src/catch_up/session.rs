//! Catch-up session initiation (Phase 4).
//!
//! Establishes a catch-up session with the selected source peer
//! via the replication protocol handshake.

use super::types::{CatchUpSession, PeerStatus};
use super::CatchUpCoordinator;
use crate::{ReplicationMessage, VectorClock};
use raisin_error::{Error, Result};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::info;

impl CatchUpCoordinator {
    /// Phase 4: Initiate catch-up session with source peer
    pub(super) async fn initiate_catch_up(
        &self,
        source_peer: &PeerStatus,
    ) -> Result<CatchUpSession> {
        info!(
            peer_id = %source_peer.node_id,
            "Phase 4: Initiating catch-up session"
        );

        // Connect to source peer
        let mut stream = timeout(
            self.network_timeout,
            TcpStream::connect(&source_peer.address),
        )
        .await
        .map_err(|_| Error::Backend("Connection timeout".to_string()))?
        .map_err(|e| Error::Backend(format!("Failed to connect to source: {}", e)))?;

        // Send Hello handshake first (required by ReplicationServer)
        let hello = ReplicationMessage::Hello {
            cluster_node_id: self.local_node_id.clone(),
            protocol_version: crate::tcp_protocol::PROTOCOL_VERSION,
            metadata: None,
        };
        Self::send_message(&mut stream, &hello).await?;

        // Wait for HelloAck
        let hello_response = Self::receive_message(&mut stream).await?;
        match hello_response {
            ReplicationMessage::HelloAck { .. } => {
                // Handshake successful, continue
            }
            _ => {
                return Err(Error::Backend(
                    "Expected HelloAck response to Hello".to_string(),
                ));
            }
        }

        // Send InitiateCatchUp request
        let request = ReplicationMessage::InitiateCatchUp {
            requesting_node: self.local_node_id.clone(),
            local_vector_clock: VectorClock::new(), // Fresh node has empty clock
        };

        Self::send_message(&mut stream, &request).await?;

        // Receive CatchUpAck
        let response = Self::receive_message(&mut stream).await?;

        match response {
            ReplicationMessage::CatchUpAck {
                source_node,
                snapshot_id,
                snapshot_vector_clock,
                estimated_transfer_size_bytes: _,
            } => {
                // Store connection for later use
                let mut connections = self.peer_connections.write().await;
                connections.insert(source_peer.node_id.clone(), stream);

                Ok(CatchUpSession {
                    session_id: snapshot_id,
                    source_node_id: source_node,
                    snapshot_vector_clock,
                })
            }
            _ => Err(Error::Backend(
                "Unexpected response to InitiateCatchUp".to_string(),
            )),
        }
    }
}
