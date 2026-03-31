//! Shared TCP protocol helpers
//!
//! This module provides common utilities for sending and receiving messages
//! over TCP sockets using the MessagePack-based replication protocol.

use crate::ReplicationMessage;
use raisin_error::{Error, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Send a protocol message over TCP
///
/// Encodes the message with MessagePack and sends it with a 4-byte length prefix
/// (big-endian u32) for framing.
///
/// # Arguments
/// * `stream` - TCP stream to send message on
/// * `message` - Message to send
///
/// # Errors
/// Returns error if encoding or network I/O fails
pub async fn send_message(stream: &mut TcpStream, message: &ReplicationMessage) -> Result<()> {
    let encoded = message
        .encode_with_length()
        .map_err(|e| Error::Backend(format!("Failed to encode message: {}", e)))?;

    stream
        .write_all(&encoded)
        .await
        .map_err(|e| Error::Backend(format!("Failed to send message: {}", e)))?;

    Ok(())
}

/// Receive a protocol message from TCP
///
/// Reads a 4-byte length prefix (big-endian u32) followed by the MessagePack-encoded
/// message body.
///
/// # Arguments
/// * `stream` - TCP stream to receive message from
///
/// # Returns
/// The deserialized message
///
/// # Errors
/// Returns error if:
/// - Network I/O fails
/// - Message size exceeds MAX_MESSAGE_SIZE
/// - Deserialization fails
pub async fn receive_message(stream: &mut TcpStream) -> Result<ReplicationMessage> {
    // Read 4-byte length prefix
    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .await
        .map_err(|e| Error::Backend(format!("Failed to read length prefix: {}", e)))?;

    let len = u32::from_be_bytes(len_bytes) as usize;

    // Validate message size
    if len > crate::tcp_protocol::MAX_MESSAGE_SIZE {
        return Err(Error::Backend(format!(
            "Message too large: {} bytes (max: {})",
            len,
            crate::tcp_protocol::MAX_MESSAGE_SIZE
        )));
    }

    // Read message body
    let mut body = vec![0u8; len];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|e| Error::Backend(format!("Failed to read message body: {}", e)))?;

    // Deserialize message
    ReplicationMessage::from_bytes(&body)
        .map_err(|e| Error::Backend(format!("Failed to deserialize message: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[tokio::test]
    async fn test_send_receive_hello() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let message = receive_message(&mut stream).await.unwrap();
            message
        });

        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let message = ReplicationMessage::hello("node1".to_string());
            send_message(&mut stream, &message).await.unwrap();
        });

        client.await.unwrap();
        let received = server.await.unwrap();

        match received {
            ReplicationMessage::Hello {
                cluster_node_id, ..
            } => {
                assert_eq!(cluster_node_id, "node1");
            }
            _ => panic!("Expected Hello message"),
        }
    }

    #[tokio::test]
    async fn test_message_too_large() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let result = receive_message(&mut stream).await;
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("Message too large"));
        });

        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            // Send a fake length that's too large
            let fake_len = (crate::tcp_protocol::MAX_MESSAGE_SIZE + 1) as u32;
            stream.write_all(&fake_len.to_be_bytes()).await.unwrap();
        });

        client.await.unwrap();
        server.await.unwrap();
    }
}
