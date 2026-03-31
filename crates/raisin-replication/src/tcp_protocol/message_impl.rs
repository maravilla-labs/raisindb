//! Serialization, deserialization, and constructor methods for [`ReplicationMessage`].
//!
//! See also: [`message`](super::message) for the enum definition and variant documentation.

use uuid::Uuid;

use super::constants::{MAX_MESSAGE_SIZE, PROTOCOL_VERSION};
use super::error::{ErrorCode, ProtocolError};
use super::message::ReplicationMessage;

impl ReplicationMessage {
    /// Serialize message to MessagePack binary format
    ///
    /// # Errors
    ///
    /// Returns error if serialization fails or message is too large.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProtocolError> {
        // Log UpdateNodeType operations before serialization
        if let ReplicationMessage::PushOperations { operations }
        | ReplicationMessage::OperationBatch { operations, .. }
        | ReplicationMessage::LogTailResponse { operations, .. } = self
        {
            for op in operations {
                if matches!(op.op_type, crate::OpType::UpdateNodeType { .. }) {
                    tracing::debug!(
                        op_id = %op.op_id,
                        tenant_id = %op.tenant_id,
                        repo_id = %op.repo_id,
                        op_seq = op.op_seq,
                        "Serializing UpdateNodeType operation for network transfer"
                    );
                }
            }
        }

        let bytes = rmp_serde::to_vec_named(self).map_err(|e| {
            // Log detailed error for operations containing NodeTypes
            if let ReplicationMessage::PushOperations { operations }
            | ReplicationMessage::OperationBatch { operations, .. }
            | ReplicationMessage::LogTailResponse { operations, .. } = self
            {
                for op in operations {
                    if matches!(op.op_type, crate::OpType::UpdateNodeType { .. }) {
                        tracing::error!(
                            op_id = %op.op_id,
                            tenant_id = %op.tenant_id,
                            repo_id = %op.repo_id,
                            op_seq = op.op_seq,
                            "Failed to serialize UpdateNodeType operation for network: {}",
                            e
                        );
                    }
                }
            }
            ProtocolError::Serialization(e.to_string())
        })?;

        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::MessageTooLarge {
                size: bytes.len(),
                max: MAX_MESSAGE_SIZE,
            });
        }

        // Log successful serialization size for UpdateNodeType operations
        if let ReplicationMessage::PushOperations { operations }
        | ReplicationMessage::OperationBatch { operations, .. }
        | ReplicationMessage::LogTailResponse { operations, .. } = self
        {
            let nodetype_count = operations
                .iter()
                .filter(|op| matches!(op.op_type, crate::OpType::UpdateNodeType { .. }))
                .count();
            if nodetype_count > 0 {
                tracing::debug!(
                    nodetype_count = nodetype_count,
                    total_ops = operations.len(),
                    serialized_size = bytes.len(),
                    "Serialized message containing UpdateNodeType operations"
                );
            }
        }

        Ok(bytes)
    }

    /// Deserialize message from MessagePack binary format
    ///
    /// # Errors
    ///
    /// Returns error if deserialization fails.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::MessageTooLarge {
                size: bytes.len(),
                max: MAX_MESSAGE_SIZE,
            });
        }

        tracing::trace!(
            bytes_len = bytes.len(),
            "Deserializing ReplicationMessage from network"
        );

        let msg = rmp_serde::from_slice(bytes).map_err(|e| {
            tracing::error!(
                bytes_len = bytes.len(),
                error = %e,
                "Failed to deserialize ReplicationMessage from network"
            );
            ProtocolError::Deserialization(e.to_string())
        })?;

        // Log UpdateNodeType operations after deserialization
        if let ReplicationMessage::PushOperations { operations }
        | ReplicationMessage::OperationBatch { operations, .. }
        | ReplicationMessage::LogTailResponse { operations, .. } = &msg
        {
            for op in operations {
                if matches!(op.op_type, crate::OpType::UpdateNodeType { .. }) {
                    tracing::debug!(
                        op_id = %op.op_id,
                        tenant_id = %op.tenant_id,
                        repo_id = %op.repo_id,
                        op_seq = op.op_seq,
                        "Deserialized UpdateNodeType operation from network"
                    );
                }
            }

            let nodetype_count = operations
                .iter()
                .filter(|op| matches!(op.op_type, crate::OpType::UpdateNodeType { .. }))
                .count();
            if nodetype_count > 0 {
                tracing::debug!(
                    nodetype_count = nodetype_count,
                    total_ops = operations.len(),
                    "Deserialized message containing UpdateNodeType operations"
                );
            }
        }

        Ok(msg)
    }

    /// Encode message with length prefix for wire transmission
    ///
    /// Format: [4-byte length (big-endian u32)][message bytes]
    pub fn encode_with_length(&self) -> Result<Vec<u8>, ProtocolError> {
        let msg_bytes = self.to_bytes()?;
        let len = msg_bytes.len() as u32;

        let mut encoded = Vec::with_capacity(4 + msg_bytes.len());
        encoded.extend_from_slice(&len.to_be_bytes());
        encoded.extend_from_slice(&msg_bytes);

        Ok(encoded)
    }

    /// Create a Hello message
    pub fn hello(cluster_node_id: String) -> Self {
        Self::Hello {
            cluster_node_id,
            protocol_version: PROTOCOL_VERSION,
            metadata: None,
        }
    }

    /// Create a HelloAck message
    pub fn hello_ack(cluster_node_id: String) -> Self {
        Self::HelloAck {
            cluster_node_id,
            protocol_version: PROTOCOL_VERSION,
            metadata: None,
        }
    }

    /// Create a Ping message with current timestamp
    pub fn ping() -> Self {
        Self::Ping {
            timestamp_ms: current_timestamp_ms(),
        }
    }

    /// Create a Pong message echoing the timestamp
    pub fn pong(timestamp_ms: u64) -> Self {
        Self::Pong { timestamp_ms }
    }

    /// Create an Error message
    pub fn error(code: ErrorCode, message: String) -> Self {
        Self::Error {
            code,
            message,
            details: None,
        }
    }

    /// Create an Ack message for successfully applied operations
    pub fn ack(op_ids: Vec<Uuid>) -> Self {
        Self::Ack { op_ids }
    }
}

/// Get current timestamp in milliseconds since Unix epoch
fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(std::time::Duration::from_secs(0))
        .as_millis() as u64
}
