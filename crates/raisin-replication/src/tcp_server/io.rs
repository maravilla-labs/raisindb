//! TCP message I/O helpers for reading and writing length-prefixed messages

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::coordinator::CoordinatorError;
use crate::tcp_protocol::ReplicationMessage;

use super::ReplicationServer;

impl ReplicationServer {
    /// Send a message to the peer
    pub(super) async fn send_message(
        &self,
        stream: &mut TcpStream,
        message: &ReplicationMessage,
    ) -> Result<(), CoordinatorError> {
        let encoded = message
            .encode_with_length()
            .map_err(|e| CoordinatorError::Protocol(e.to_string()))?;

        stream
            .write_all(&encoded)
            .await
            .map_err(|e| CoordinatorError::Network(format!("Failed to send message: {}", e)))?;

        Ok(())
    }

    /// Receive a message from the peer
    pub(super) async fn receive_message(
        &self,
        stream: &mut TcpStream,
    ) -> Result<ReplicationMessage, CoordinatorError> {
        // Read 4-byte length prefix
        let mut len_bytes = [0u8; 4];
        stream.read_exact(&mut len_bytes).await.map_err(|e| {
            CoordinatorError::Network(format!("Failed to read length prefix: {}", e))
        })?;

        let len = u32::from_be_bytes(len_bytes) as usize;

        // Validate message size
        if len > crate::tcp_protocol::MAX_MESSAGE_SIZE {
            return Err(CoordinatorError::Protocol(format!(
                "Message too large: {} bytes (max: {})",
                len,
                crate::tcp_protocol::MAX_MESSAGE_SIZE
            )));
        }

        // Read message body
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).await.map_err(|e| {
            CoordinatorError::Network(format!("Failed to read message body: {}", e))
        })?;

        ReplicationMessage::from_bytes(&body).map_err(|e| {
            // Debug: Print first 50 bytes on error
            tracing::error!(
                "Failed to deserialize message. Error: {}. First 50 bytes: {:?}",
                e,
                &body[..body.len().min(50)]
            );
            CoordinatorError::Protocol(e.to_string())
        })
    }
}
