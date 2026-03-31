//! Heartbeat monitoring and automatic reconnection for replication peers

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::time;
use tracing::{debug, info, warn};

use crate::tcp_protocol::ReplicationMessage;

use super::types::{ConnectionState, PeerManagerError};
use super::PeerManager;

impl PeerManager {
    /// Start heartbeat monitoring for all peers
    ///
    /// This also handles reconnection of failed peers
    pub async fn start_heartbeat_monitor(self: Arc<Self>) {
        let interval = Duration::from_secs(self.config.heartbeat_interval_seconds);
        let mut ticker = time::interval(interval);

        loop {
            ticker.tick().await;

            let peer_ids: Vec<String> = {
                let peers = self.peers.read().await;
                peers.keys().cloned().collect()
            };

            for peer_id in peer_ids {
                let manager = self.clone();
                let peer_id_clone = peer_id.clone();

                tokio::spawn(async move {
                    // Check peer state first
                    let state = {
                        let peers = manager.peers.read().await;
                        if let Some(conn) = peers.get(&peer_id_clone) {
                            let conn_guard = conn.lock().await;
                            conn_guard.state
                        } else {
                            return;
                        }
                    };

                    match state {
                        ConnectionState::Connected => {
                            // Send heartbeat to connected peers
                            if let Err(e) = manager.send_heartbeat(&peer_id_clone).await {
                                warn!(peer_id = %peer_id_clone, error = %e, "Heartbeat failed");
                            }
                        }
                        ConnectionState::Failed | ConnectionState::Disconnected => {
                            // Retry connection for failed/disconnected peers
                            info!(peer_id = %peer_id_clone, "Retrying connection to failed peer");
                            if let Err(e) = manager.connect_to_peer(&peer_id_clone).await {
                                debug!(peer_id = %peer_id_clone, error = %e, "Reconnection attempt failed");
                            } else {
                                info!(peer_id = %peer_id_clone, "Successfully reconnected to peer");
                            }
                        }
                        ConnectionState::Connecting => {
                            // Already connecting, skip
                            debug!(peer_id = %peer_id_clone, "Peer connection in progress");
                        }
                        ConnectionState::Disabled => {
                            // Peer is disabled, skip entirely
                            debug!(peer_id = %peer_id_clone, "Peer is disabled");
                        }
                    }
                });
            }
        }
    }

    /// Send heartbeat to a peer
    pub(super) async fn send_heartbeat(&self, peer_id: &str) -> Result<(), PeerManagerError> {
        let ping = ReplicationMessage::ping();

        match time::timeout(Duration::from_secs(5), self.send_request(peer_id, &ping)).await {
            Ok(Ok(ReplicationMessage::Pong { .. })) => {
                // Update last_heartbeat
                let peers = self.peers.read().await;
                if let Some(conn) = peers.get(peer_id) {
                    let mut conn_guard = conn.lock().await;
                    conn_guard.last_heartbeat = Instant::now();
                }
                debug!(peer_id = %peer_id, "Heartbeat successful");
                Ok(())
            }
            Ok(Ok(msg)) => {
                warn!(peer_id = %peer_id, "Expected Pong, got {:?}", msg);
                Err(PeerManagerError::Protocol(format!(
                    "Unexpected message: {:?}",
                    msg
                )))
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PeerManagerError::Timeout),
        }
    }
}
