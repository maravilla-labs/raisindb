// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Error types for PostgreSQL wire protocol transport layer
//!
//! This module defines error types specific to the pgwire transport implementation,
//! including protocol errors, authentication failures, and type conversion issues.

use pgwire::error::{ErrorInfo, PgWireError};
use thiserror::Error;

/// Result type alias for pgwire transport operations
pub type Result<T> = std::result::Result<T, PgWireTransportError>;

/// Error type for pgwire transport layer operations
///
/// This error type covers all error conditions that can occur in the PostgreSQL
/// wire protocol transport layer, from low-level IO errors to high-level protocol
/// and query execution errors.
#[derive(Debug, Error)]
pub enum PgWireTransportError {
    /// IO errors during network communication
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Protocol-level errors in pgwire message handling
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Authentication and authorization errors
    #[error("Authentication error: {0}")]
    Auth(String),

    /// Query execution errors
    #[error("Query error: {0}")]
    Query(String),

    /// Type conversion and mapping errors
    #[error("Type conversion error: {0}")]
    TypeConversion(String),

    /// Internal server errors
    #[error("Internal error: {0}")]
    Internal(String),

    /// Storage layer errors from RaisinDB
    #[error("Storage error: {0}")]
    Storage(#[from] raisin_error::Error),
}

impl PgWireTransportError {
    /// Create an authentication error
    ///
    /// # Arguments
    /// * `msg` - Error message describing the authentication failure
    ///
    /// # Examples
    /// ```
    /// use raisin_transport_pgwire::error::PgWireTransportError;
    ///
    /// let err = PgWireTransportError::auth("Invalid credentials");
    /// ```
    pub fn auth(msg: impl Into<String>) -> Self {
        Self::Auth(msg.into())
    }

    /// Create a query execution error
    ///
    /// # Arguments
    /// * `msg` - Error message describing the query failure
    ///
    /// # Examples
    /// ```
    /// use raisin_transport_pgwire::error::PgWireTransportError;
    ///
    /// let err = PgWireTransportError::query("Invalid SQL syntax");
    /// ```
    pub fn query(msg: impl Into<String>) -> Self {
        Self::Query(msg.into())
    }

    /// Create a protocol error
    ///
    /// # Arguments
    /// * `msg` - Error message describing the protocol violation
    ///
    /// # Examples
    /// ```
    /// use raisin_transport_pgwire::error::PgWireTransportError;
    ///
    /// let err = PgWireTransportError::protocol("Invalid message format");
    /// ```
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::Protocol(msg.into())
    }

    /// Create a type conversion error
    ///
    /// # Arguments
    /// * `msg` - Error message describing the conversion failure
    ///
    /// # Examples
    /// ```
    /// use raisin_transport_pgwire::error::PgWireTransportError;
    ///
    /// let err = PgWireTransportError::type_conversion("Cannot convert JSON to INTEGER");
    /// ```
    pub fn type_conversion(msg: impl Into<String>) -> Self {
        Self::TypeConversion(msg.into())
    }

    /// Create an internal error
    ///
    /// # Arguments
    /// * `msg` - Error message describing the internal failure
    ///
    /// # Examples
    /// ```
    /// use raisin_transport_pgwire::error::PgWireTransportError;
    ///
    /// let err = PgWireTransportError::internal("Unexpected state");
    /// ```
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}

/// Convert PgWireTransportError to pgwire's PgWireError
///
/// This implementation allows seamless integration with the pgwire library
/// by converting our custom error types to the appropriate pgwire error variants.
impl From<PgWireTransportError> for PgWireError {
    fn from(err: PgWireTransportError) -> Self {
        match err {
            // IO errors map directly to pgwire's IoError variant
            PgWireTransportError::Io(io_err) => PgWireError::IoError(io_err),

            // All other errors are converted to UserError with appropriate severity and code
            PgWireTransportError::Protocol(msg) => {
                let info = ErrorInfo::new(
                    "ERROR".to_string(),
                    "08P01".to_string(), // protocol_violation
                    msg,
                );
                PgWireError::UserError(Box::new(info))
            }

            PgWireTransportError::Auth(msg) => {
                let info = ErrorInfo::new(
                    "ERROR".to_string(),
                    "28000".to_string(), // invalid_authorization_specification
                    msg,
                );
                PgWireError::UserError(Box::new(info))
            }

            PgWireTransportError::Query(msg) => {
                let info = ErrorInfo::new(
                    "ERROR".to_string(),
                    "42601".to_string(), // syntax_error
                    msg,
                );
                PgWireError::UserError(Box::new(info))
            }

            PgWireTransportError::TypeConversion(msg) => {
                let info = ErrorInfo::new(
                    "ERROR".to_string(),
                    "42804".to_string(), // datatype_mismatch
                    msg,
                );
                PgWireError::UserError(Box::new(info))
            }

            PgWireTransportError::Internal(msg) => {
                let info = ErrorInfo::new(
                    "ERROR".to_string(),
                    "XX000".to_string(), // internal_error
                    msg,
                );
                PgWireError::UserError(Box::new(info))
            }

            PgWireTransportError::Storage(err) => {
                let info = ErrorInfo::new(
                    "ERROR".to_string(),
                    "58000".to_string(), // system_error
                    err.to_string(),
                );
                PgWireError::UserError(Box::new(info))
            }
        }
    }
}

// Ensure error type is Send + Sync for use in async contexts
static_assertions::assert_impl_all!(PgWireTransportError: Send, Sync);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_helper() {
        let err = PgWireTransportError::auth("test auth error");
        assert!(matches!(err, PgWireTransportError::Auth(_)));
        assert_eq!(err.to_string(), "Authentication error: test auth error");
    }

    #[test]
    fn test_query_helper() {
        let err = PgWireTransportError::query("test query error");
        assert!(matches!(err, PgWireTransportError::Query(_)));
        assert_eq!(err.to_string(), "Query error: test query error");
    }

    #[test]
    fn test_protocol_helper() {
        let err = PgWireTransportError::protocol("test protocol error");
        assert!(matches!(err, PgWireTransportError::Protocol(_)));
        assert_eq!(err.to_string(), "Protocol error: test protocol error");
    }

    #[test]
    fn test_type_conversion_helper() {
        let err = PgWireTransportError::type_conversion("test conversion error");
        assert!(matches!(err, PgWireTransportError::TypeConversion(_)));
        assert_eq!(
            err.to_string(),
            "Type conversion error: test conversion error"
        );
    }

    #[test]
    fn test_internal_helper() {
        let err = PgWireTransportError::internal("test internal error");
        assert!(matches!(err, PgWireTransportError::Internal(_)));
        assert_eq!(err.to_string(), "Internal error: test internal error");
    }

    #[test]
    fn test_io_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection lost");
        let err = PgWireTransportError::from(io_err);
        assert!(matches!(err, PgWireTransportError::Io(_)));
    }

    #[test]
    fn test_conversion_to_pgwire_error() {
        // Test auth error conversion
        let auth_err = PgWireTransportError::auth("invalid password");
        let pg_err: PgWireError = auth_err.into();
        assert!(matches!(pg_err, PgWireError::UserError(_)));

        // Test query error conversion
        let query_err = PgWireTransportError::query("syntax error");
        let pg_err: PgWireError = query_err.into();
        assert!(matches!(pg_err, PgWireError::UserError(_)));

        // Test IO error conversion
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broken");
        let transport_err = PgWireTransportError::from(io_err);
        let pg_err: PgWireError = transport_err.into();
        assert!(matches!(pg_err, PgWireError::IoError(_)));
    }
}
