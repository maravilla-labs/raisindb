//! TCP server for accepting incoming replication peer connections
//!
//! This module implements a TCP server that listens for incoming connections from
//! other cluster nodes and handles replication protocol messages.

mod handshake;
mod io;
mod message_handler;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, warn};

use crate::config::ClusterConfig;
use crate::coordinator::{CoordinatorError, ReplicationCoordinator};
use crate::tcp_protocol::ReplicationMessage;
use crate::{CheckpointProvider, OperationLogStorage};

/// TCP server for accepting incoming peer connections
pub struct ReplicationServer {
    /// Replication coordinator for triggering sync on incoming connections
    coordinator: Arc<ReplicationCoordinator>,

    /// Cluster configuration
    cluster_config: ClusterConfig,

    /// Storage backend for serving operations
    pub(super) storage: Arc<dyn OperationLogStorage>,

    /// This node's cluster ID
    pub(super) cluster_node_id: String,

    /// Optional checkpoint provider for serving RocksDB snapshots
    pub(super) checkpoint_provider: Option<Arc<dyn CheckpointProvider>>,
}

impl ReplicationServer {
    /// Create a new replication server
    pub fn new(
        coordinator: Arc<ReplicationCoordinator>,
        cluster_config: ClusterConfig,
        storage: Arc<dyn OperationLogStorage>,
    ) -> Self {
        let cluster_node_id = cluster_config.node_id.clone();

        Self {
            coordinator,
            cluster_config,
            storage,
            cluster_node_id,
            checkpoint_provider: None,
        }
    }

    /// Builder method to add a checkpoint provider
    ///
    /// This allows the server to handle RequestCheckpoint messages for cluster catch-up.
    pub fn with_checkpoint_provider(
        mut self,
        checkpoint_provider: Arc<dyn CheckpointProvider>,
    ) -> Self {
        self.checkpoint_provider = Some(checkpoint_provider);
        self
    }

    /// Start the TCP server and listen for incoming connections
    pub async fn start(self: Arc<Self>) -> Result<(), CoordinatorError> {
        let bind_addr = format!(
            "{}:{}",
            self.cluster_config.bind_address, self.cluster_config.replication_port
        );

        info!(
            bind_addr = %bind_addr,
            node_id = %self.cluster_node_id,
            "Starting replication TCP server"
        );

        let listener = TcpListener::bind(&bind_addr).await.map_err(|e| {
            CoordinatorError::Network(format!("Failed to bind to {}: {}", bind_addr, e))
        })?;

        info!(bind_addr = %bind_addr, "Replication server listening");

        // Accept connections loop
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    debug!(peer_addr = %peer_addr, "Accepted incoming connection");

                    let server = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = server.handle_connection(stream).await {
                            warn!(peer_addr = %peer_addr, error = %e, "Connection handler failed");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Failed to accept connection");
                }
            }
        }
    }

    /// Handle an incoming connection from a peer
    async fn handle_connection(&self, mut stream: TcpStream) -> Result<(), CoordinatorError> {
        let peer_addr = stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        // Perform handshake
        let peer_id = self.perform_handshake(&mut stream).await?;

        info!(peer_id = %peer_id, peer_addr = %peer_addr, "Peer connected and authenticated");

        // Note: We don't trigger sync on incoming connections because:
        // - The peer initiating the connection (outgoing) already triggers sync
        // - Triggering sync from both sides creates race conditions and redundant syncs
        // - Sync-on-connect only needs to happen from the outgoing connection side

        // Handle messages in a loop
        loop {
            match self.receive_message(&mut stream).await {
                Ok(message) => {
                    debug!(peer_id = %peer_id, "Received message: {:?}", message);

                    if let Err(e) = self.handle_message(&mut stream, &peer_id, message).await {
                        error!(peer_id = %peer_id, error = %e, "Failed to handle message");
                        let error_msg = ReplicationMessage::error(
                            crate::tcp_protocol::ErrorCode::InternalError,
                            e.to_string(),
                        );
                        let _ = self.send_message(&mut stream, &error_msg).await;
                        break;
                    }
                }
                Err(e) => {
                    warn!(peer_id = %peer_id, error = %e, "Connection closed or error reading message");
                    break;
                }
            }
        }

        info!(peer_id = %peer_id, "Connection closed");
        Ok(())
    }
}
