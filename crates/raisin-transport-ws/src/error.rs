// SPDX-License-Identifier: BSL-1.1

//! Error types for WebSocket transport

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WsError {
    #[error("Authentication error: {0}")]
    AuthError(#[from] crate::auth::AuthError),

    #[error("Connection error: {0}")]
    ConnectionError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Operation error: {0}")]
    OperationError(String),

    #[error("Not authenticated")]
    NotAuthenticated,

    #[error("Permission denied")]
    PermissionDenied,

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Timeout")]
    Timeout,

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Internal error: {0}")]
    InternalError(String),
}

impl WsError {
    pub fn error_code(&self) -> &'static str {
        match self {
            WsError::AuthError(_) => "AUTH_ERROR",
            WsError::ConnectionError(_) => "CONNECTION_ERROR",
            WsError::ProtocolError(_) => "PROTOCOL_ERROR",
            WsError::SerializationError(_) => "SERIALIZATION_ERROR",
            WsError::OperationError(_) => "OPERATION_ERROR",
            WsError::NotAuthenticated => "NOT_AUTHENTICATED",
            WsError::PermissionDenied => "PERMISSION_DENIED",
            WsError::RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
            WsError::ConnectionClosed => "CONNECTION_CLOSED",
            WsError::Timeout => "TIMEOUT",
            WsError::InvalidRequest(_) => "INVALID_REQUEST",
            WsError::StorageError(_) => "STORAGE_ERROR",
            WsError::InternalError(_) => "INTERNAL_ERROR",
        }
    }
}

impl From<raisin_error::Error> for WsError {
    fn from(err: raisin_error::Error) -> Self {
        WsError::StorageError(err.to_string())
    }
}

impl From<serde_json::Error> for WsError {
    fn from(err: serde_json::Error) -> Self {
        WsError::SerializationError(err.to_string())
    }
}

impl From<rmp_serde::encode::Error> for WsError {
    fn from(err: rmp_serde::encode::Error) -> Self {
        WsError::SerializationError(err.to_string())
    }
}

impl From<rmp_serde::decode::Error> for WsError {
    fn from(err: rmp_serde::decode::Error) -> Self {
        WsError::SerializationError(err.to_string())
    }
}

impl From<crate::connection::TransactionError> for WsError {
    fn from(err: crate::connection::TransactionError) -> Self {
        WsError::InvalidRequest(err.to_string())
    }
}

impl From<raisin_flow_runtime::types::FlowError> for WsError {
    fn from(err: raisin_flow_runtime::types::FlowError) -> Self {
        use raisin_flow_runtime::types::FlowError;
        match err {
            FlowError::NodeNotFound(msg) => WsError::InvalidRequest(msg),
            FlowError::InvalidDefinition(msg) => WsError::InvalidRequest(msg),
            FlowError::InvalidStateTransition { from, to } => {
                WsError::InvalidRequest(format!("Invalid state transition from {} to {}", from, to))
            }
            FlowError::AlreadyTerminated { status } => {
                WsError::InvalidRequest(format!("Flow instance is already {}", status))
            }
            FlowError::NotSupported(msg) => WsError::InternalError(msg),
            FlowError::Serialization(msg) => WsError::InternalError(msg),
            other => WsError::InternalError(other.to_string()),
        }
    }
}
