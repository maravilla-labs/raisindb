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
    /// A spatial index scan hit its per-cell entry budget, so it cannot answer
    /// from the index without answering SHORT.
    ///
    /// Typed rather than a `Backend(String)` because it is not a failure but a
    /// **re-plan signal**: the SQL executor recognises it and degrades to the
    /// spatial fallback (a correct-but-slow row scan) instead of failing the
    /// query. A string match on the message would have made that routing depend
    /// on wording nobody would think to preserve.
    #[error(
        "Spatial index budget exceeded: cell '{cell}' for property '{property}' in workspace \
         '{workspace}' holds more than {limit} entries; refusing to answer from a partial scan"
    )]
    SpatialBudgetExceeded {
        workspace: String,
        property: String,
        cell: String,
        limit: usize,
    },
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

    /// Whether this is the spatial per-cell budget signal.
    ///
    /// The query planner/executor uses it to re-plan onto the spatial fallback.
    /// Every other caller should treat it as an ordinary error.
    pub fn is_spatial_budget_exceeded(&self) -> bool {
        matches!(self, Self::SpatialBudgetExceeded { .. })
    }
}
