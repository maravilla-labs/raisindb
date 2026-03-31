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

use thiserror::Error;

#[derive(Debug, Error)]
/// Error type for all operations in the raisin-models crate.
///
/// This error type is stable for library users and does not expose dependency error types directly.
pub enum RaisinModelError {
    #[error("Validation error: {0}")]
    Validation(#[from] validator::ValidationErrors),
    #[error("Serialization/deserialization error: {0}")]
    Serde(String),
    #[error("Other error: {0}")]
    Other(String),
    // Optionally, for internal use or debugging, not for stable public API:
    #[doc(hidden)]
    #[error("Internal error: {0}")]
    Internal(Box<dyn std::error::Error + Send + Sync>),
}

impl RaisinModelError {
    /// Create a Serde error from any error implementing std::error::Error.
    pub fn from_serde<E: std::error::Error>(err: E) -> Self {
        RaisinModelError::Serde(err.to_string())
    }
    /// Create an internal error from any error implementing std::error::Error.
    pub fn internal<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        RaisinModelError::Internal(Box::new(err))
    }
}
