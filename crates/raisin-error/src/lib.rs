// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Common error types for RaisinDB

use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Already exists: {0}")]
    AlreadyExists(String),
    #[error("Validation failed: {0}")]
    Validation(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Backend error: {0}")]
    Backend(String),
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Lock error: {0}")]
    Lock(String),
    #[error("Encoding error: {0}")]
    Encoding(String),
    #[error("Invalid state: {0}")]
    InvalidState(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    /// Create a storage backend error
    ///
    /// Helper for storage implementations to create Backend errors
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::Backend(msg.into())
    }

    /// Create a lock error
    ///
    /// Helper for converting mutex/lock errors
    pub fn lock(msg: impl Into<String>) -> Self {
        Self::Lock(msg.into())
    }

    /// Create an encoding error
    ///
    /// Helper for string encoding/decoding errors
    pub fn encoding(msg: impl Into<String>) -> Self {
        Self::Encoding(msg.into())
    }

    /// Create an invalid state error
    ///
    /// Helper for unexpected state conditions
    pub fn invalid_state(msg: impl Into<String>) -> Self {
        Self::InvalidState(msg.into())
    }

    /// Create an internal error
    ///
    /// Helper for internal invariant violations
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::Internal(msg.into())
    }
}
