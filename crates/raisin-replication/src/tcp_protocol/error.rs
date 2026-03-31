//! Error types for TCP replication protocol

use serde::{Deserialize, Serialize};

/// Error codes for replication protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum ErrorCode {
    /// Protocol version mismatch
    ProtocolVersionMismatch = 1000,

    /// Authentication failed
    AuthenticationFailed = 1001,

    /// Protocol error (generic)
    ProtocolError = 1002,

    /// Requested repository not found
    RepositoryNotFound = 1003,

    /// Operation validation failed
    InvalidOperation = 1004,

    /// Internal server error
    InternalError = 1005,

    /// Message too large
    MessageTooLarge = 1006,

    /// Rate limit exceeded
    RateLimitExceeded = 1007,
}

/// Protocol errors
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Message too large: {size} bytes (max: {max} bytes)")]
    MessageTooLarge { size: usize, max: usize },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: u8, actual: u8 },

    #[error("Invalid message type")]
    InvalidMessageType,

    #[error("Connection closed")]
    ConnectionClosed,
}
