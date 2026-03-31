//! Peer connection management: connect, send, receive, disconnect

use std::sync::Arc;
use std::time::Instant;

use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, info};

use crate::config::PeerConfig;
use crate::tcp_protocol::ReplicationMessage;

use super::types::{ConnectionPool, ConnectionState, PeerConnection, PeerManagerError, PeerStatus};
use super::PeerManager;

impl PeerManager {
    /// Add a peer to manage
    pub async fn add_peer(&self, peer_config: PeerConfig) {
        if !peer_config.enabled {
            info!(peer_id = %peer_config.node_id, "Peer is disabled, skipping");
            return;
        }

        let peer_id = peer_config.node_id.clone();
        let max_connections = self.config.max_connections_per_peer;

        let conn = Arc::new(Mutex::new(PeerConnection {
            peer_config,
            pool: ConnectionPool::new(max_connections),
            last_heartbeat: Instant::now(),
            state: ConnectionState::Disconnected,
            failed_attempts: 0,
            last_error: None,
            max_connections,
        }));

        let mut peers = self.peers.write().await;
        peers.insert(peer_id.clone(), conn);

        info!(peer_id = %peer_id, max_connections = %max_connections, "Peer added to manager with connection pool");
    }

    /// Connect to a specific peer
    pub async fn connect_to_peer(&self, peer_id: &str) -> Result<(), PeerManagerError> {
        let peers = self.peers.read().await;
        let conn = peers
            .get(peer_id)
            .ok_or_else(|| PeerManagerError::PeerNotFound(peer_id.to_string()))?
            .clone();
        drop(peers);

        let mut conn_guard = conn.lock().await;

        // Don't reconnect if already connected
        if conn_guard.state == ConnectionState::Connected {
            return Ok(());
        }

        conn_guard.state = ConnectionState::Connecting;
        let peer_config = conn_guard.peer_config.clone();
        let address = peer_config.address();
        drop(conn_guard);

        info!(peer_id = %peer_id, address = %address, "Connecting to peer");

        match self.open_stream(&peer_config, peer_id).await {
            Ok(stream) => {
                let mut conn_guard = conn.lock().await;

                conn_guard.pool.add_connection(stream);
                conn_guard.state = ConnectionState::Connected;
                conn_guard.last_heartbeat = Instant::now();
                conn_guard.failed_attempts = 0;
                conn_guard.last_error = None;

                info!(peer_id = %peer_id, "Successfully connected to peer and added to pool");

                // Drop lock before invoking callback
                drop(conn_guard);

                // Always trigger sync on connect to ensure fresh instances get synchronized
                // This is critical for cluster consistency - even if we were previously connected,
                // we may have missed operations while disconnected
                if let Some(ref callback) = *self.on_connected.lock().await {
                    info!(peer_id = %peer_id, "Triggering sync-on-connect callback");
                    callback(peer_id.to_string());
                }

                Ok(())
            }
            Err(e) => {
                let mut conn_guard = conn.lock().await;
                conn_guard.state = ConnectionState::Failed;
                conn_guard.failed_attempts += 1;
                conn_guard.last_error = Some(e.to_string());
                info!(peer_id = %peer_id, error = %e, "Failed to connect, will retry");
                Err(e)
            }
        }
    }

    /// Send a message to a peer
    pub async fn send_message(
        &self,
        peer_id: &str,
        message: &ReplicationMessage,
    ) -> Result<(), PeerManagerError> {
        let (conn, mut stream) = self.acquire_stream(peer_id).await?;
        let result = self.send_message_internal(&mut stream, message).await;
        let success = result.is_ok();
        self.release_stream(peer_id, conn, stream, success).await;
        result
    }

    /// Send a request to a peer and wait for a response on the same connection
    pub async fn send_request(
        &self,
        peer_id: &str,
        message: &ReplicationMessage,
    ) -> Result<ReplicationMessage, PeerManagerError> {
        let (conn, mut stream) = self.acquire_stream(peer_id).await?;

        if let Err(e) = self.send_message_internal(&mut stream, message).await {
            self.release_stream(peer_id, conn, stream, false).await;
            return Err(e);
        }

        let response = self.receive_message_internal(&mut stream).await;
        let success = response.is_ok();
        self.release_stream(peer_id, conn, stream, success).await;
        response
    }

    /// Disconnect from a peer
    pub async fn disconnect_peer(&self, peer_id: &str) -> Result<(), PeerManagerError> {
        let peers = self.peers.write().await;
        if let Some(conn) = peers.get(peer_id) {
            let mut conn_guard = conn.lock().await;
            conn_guard.pool.clear();
            conn_guard.state = ConnectionState::Disconnected;
            info!(peer_id = %peer_id, "Disconnected from peer and cleared connection pool");
            Ok(())
        } else {
            Err(PeerManagerError::PeerNotFound(peer_id.to_string()))
        }
    }

    /// Get peer status
    pub async fn get_peer_status(&self, peer_id: &str) -> Option<PeerStatus> {
        let peers = self.peers.read().await;
        if let Some(conn) = peers.get(peer_id) {
            let conn_guard = conn.lock().await;
            Some(PeerStatus {
                peer_id: peer_id.to_string(),
                state: conn_guard.state,
                last_heartbeat: conn_guard.last_heartbeat,
                failed_attempts: conn_guard.failed_attempts,
                last_error: conn_guard.last_error.clone(),
            })
        } else {
            None
        }
    }

    /// Get status for all peers
    pub async fn get_all_peer_status(&self) -> Vec<PeerStatus> {
        let peers = self.peers.read().await;
        let mut statuses = Vec::new();

        for (peer_id, conn) in peers.iter() {
            let conn_guard = conn.lock().await;
            statuses.push(PeerStatus {
                peer_id: peer_id.clone(),
                state: conn_guard.state,
                last_heartbeat: conn_guard.last_heartbeat,
                failed_attempts: conn_guard.failed_attempts,
                last_error: conn_guard.last_error.clone(),
            });
        }

        statuses
    }
}
