//! Internal I/O helpers for stream acquisition, release, and message framing

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::time;
use tracing::{debug, error, info};

use crate::config::PeerConfig;
use crate::tcp_protocol::ReplicationMessage;

use super::types::{ConnectionState, PeerConnection, PeerManagerError};
use super::PeerManager;

impl PeerManager {
    pub(super) async fn acquire_stream(
        &self,
        peer_id: &str,
    ) -> Result<(Arc<Mutex<PeerConnection>>, TcpStream), PeerManagerError> {
        let conn = {
            let peers = self.peers.read().await;
            peers
                .get(peer_id)
                .ok_or_else(|| PeerManagerError::PeerNotFound(peer_id.to_string()))?
                .clone()
        };

        let wait_limit = Duration::from_secs(self.config.connect_timeout_seconds.max(1));
        let start = Instant::now();

        loop {
            let mut conn_guard = conn.lock().await;

            match conn_guard.state {
                ConnectionState::Connected => {
                    // proceed below
                }
                ConnectionState::Connecting => {
                    drop(conn_guard);
                    time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
                ConnectionState::Disconnected | ConnectionState::Failed => {
                    drop(conn_guard);
                    self.connect_to_peer(peer_id).await?;
                    continue;
                }
                ConnectionState::Disabled => {
                    return Err(PeerManagerError::NotConnected(peer_id.to_string()));
                }
            }

            if let Some(stream) = conn_guard.pool.try_acquire() {
                debug!(peer_id = %peer_id, "Acquired existing connection from pool");
                return Ok((conn.clone(), stream));
            }

            if conn_guard.pool.can_create_more() {
                conn_guard.pool.increment_count();
                let peer_config = conn_guard.peer_config.clone();
                drop(conn_guard);

                match self.open_stream(&peer_config, peer_id).await {
                    Ok(stream) => {
                        debug!(peer_id = %peer_id, "Created new connection for pool");
                        return Ok((conn.clone(), stream));
                    }
                    Err(e) => {
                        let mut conn_guard = conn.lock().await;
                        let had_other_connections = conn_guard.pool.current_count > 1;
                        conn_guard.pool.drop_connection();
                        conn_guard.last_error = Some(e.to_string());
                        drop(conn_guard);

                        if !had_other_connections || start.elapsed() >= wait_limit {
                            return Err(e);
                        }

                        time::sleep(Duration::from_millis(25)).await;
                        continue;
                    }
                }
            } else {
                drop(conn_guard);

                if start.elapsed() >= wait_limit {
                    return Err(PeerManagerError::Connection(
                        "No available connections".to_string(),
                    ));
                }

                time::sleep(Duration::from_millis(25)).await;
            }
        }
    }

    pub(super) async fn release_stream(
        &self,
        peer_id: &str,
        conn: Arc<Mutex<PeerConnection>>,
        stream: TcpStream,
        success: bool,
    ) {
        let mut conn_guard = conn.lock().await;
        if success {
            conn_guard.pool.release(stream);
        } else {
            debug!(peer_id = %peer_id, "Dropping failed connection from pool");
            conn_guard.pool.drop_connection();
            drop(stream);
        }
    }

    pub(super) async fn open_stream(
        &self,
        peer_config: &PeerConfig,
        peer_id: &str,
    ) -> Result<TcpStream, PeerManagerError> {
        let address = peer_config.address();

        let connect_result = time::timeout(
            Duration::from_secs(self.config.connect_timeout_seconds),
            TcpStream::connect(&address),
        )
        .await;

        match connect_result {
            Ok(Ok(mut stream)) => {
                let hello = ReplicationMessage::hello(self.cluster_node_id.clone());
                self.send_message_internal(&mut stream, &hello).await?;

                match self.receive_message_internal(&mut stream).await {
                    Ok(ReplicationMessage::HelloAck { .. }) => Ok(stream),
                    Ok(msg) => {
                        error!(peer_id = %peer_id, "Expected HelloAck, got {:?}", msg);
                        Err(PeerManagerError::Handshake(format!(
                            "Unexpected message: {:?}",
                            msg
                        )))
                    }
                    Err(e) => {
                        error!(peer_id = %peer_id, error = %e, "Failed to receive HelloAck");
                        Err(PeerManagerError::Handshake(e.to_string()))
                    }
                }
            }
            Ok(Err(e)) => {
                debug!(peer_id = %peer_id, error = %e, "Connection attempt failed");
                Err(PeerManagerError::Connection(e.to_string()))
            }
            Err(_) => {
                error!(peer_id = %peer_id, "Connection timeout");
                Err(PeerManagerError::Timeout)
            }
        }
    }

    pub(super) async fn send_message_internal(
        &self,
        stream: &mut TcpStream,
        message: &ReplicationMessage,
    ) -> Result<(), PeerManagerError> {
        let encoded = message
            .encode_with_length()
            .map_err(|e| PeerManagerError::Protocol(e.to_string()))?;

        time::timeout(
            Duration::from_secs(self.config.write_timeout_seconds),
            stream.write_all(&encoded),
        )
        .await
        .map_err(|_| PeerManagerError::Timeout)?
        .map_err(PeerManagerError::Io)?;

        Ok(())
    }

    pub(super) async fn receive_message_internal(
        &self,
        stream: &mut TcpStream,
    ) -> Result<ReplicationMessage, PeerManagerError> {
        // Read 4-byte length prefix
        let mut len_bytes = [0u8; 4];
        time::timeout(
            Duration::from_secs(self.config.read_timeout_seconds),
            stream.read_exact(&mut len_bytes),
        )
        .await
        .map_err(|_| PeerManagerError::Timeout)?
        .map_err(PeerManagerError::Io)?;

        let len = u32::from_be_bytes(len_bytes) as usize;

        // Read message body
        let mut body = vec![0u8; len];
        time::timeout(
            Duration::from_secs(self.config.read_timeout_seconds),
            stream.read_exact(&mut body),
        )
        .await
        .map_err(|_| PeerManagerError::Timeout)?
        .map_err(PeerManagerError::Io)?;

        ReplicationMessage::from_bytes(&body).map_err(|e| PeerManagerError::Protocol(e.to_string()))
    }
}
