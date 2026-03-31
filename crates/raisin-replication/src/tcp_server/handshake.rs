//! Handshake protocol for incoming replication peer connections

use tokio::net::TcpStream;

use crate::coordinator::CoordinatorError;
use crate::tcp_protocol::ReplicationMessage;

use super::ReplicationServer;

impl ReplicationServer {
    /// Perform handshake with incoming peer
    pub(super) async fn perform_handshake(
        &self,
        stream: &mut TcpStream,
    ) -> Result<String, CoordinatorError> {
        // Wait for Hello message
        let message = self.receive_message(stream).await?;

        match message {
            ReplicationMessage::Hello {
                cluster_node_id,
                protocol_version,
                ..
            } => {
                // Validate protocol version
                if protocol_version != crate::tcp_protocol::PROTOCOL_VERSION {
                    let error_msg = ReplicationMessage::error(
                        crate::tcp_protocol::ErrorCode::ProtocolVersionMismatch,
                        format!(
                            "Protocol version mismatch: expected {}, got {}",
                            crate::tcp_protocol::PROTOCOL_VERSION,
                            protocol_version
                        ),
                    );
                    self.send_message(stream, &error_msg).await?;
                    return Err(CoordinatorError::Protocol(format!(
                        "Protocol version mismatch: expected {}, got {}",
                        crate::tcp_protocol::PROTOCOL_VERSION,
                        protocol_version
                    )));
                }

                // Send HelloAck
                let hello_ack = ReplicationMessage::HelloAck {
                    cluster_node_id: self.cluster_node_id.clone(),
                    protocol_version: crate::tcp_protocol::PROTOCOL_VERSION,
                    metadata: None,
                };
                self.send_message(stream, &hello_ack).await?;

                Ok(cluster_node_id)
            }
            _ => {
                let error_msg = ReplicationMessage::error(
                    crate::tcp_protocol::ErrorCode::ProtocolError,
                    "Expected Hello message".to_string(),
                );
                self.send_message(stream, &error_msg).await?;
                Err(CoordinatorError::Protocol(
                    "Expected Hello message".to_string(),
                ))
            }
        }
    }
}
