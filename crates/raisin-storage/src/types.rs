// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Common types used throughout the storage layer.
//!
//! This module contains shared data structures used by multiple traits and modules.

/// Metadata describing a commit applied to the repository.
#[derive(Debug, Clone)]
pub struct CommitMetadata {
    /// Commit message describing the change
    pub message: String,
    /// Actor performing the change (user ID, system identifier, etc.)
    pub actor: String,
    /// Whether this change was performed by the system (vs a human actor)
    pub is_system: bool,
}

impl CommitMetadata {
    /// Convenience constructor for user initiated commits
    pub fn new(message: impl Into<String>, actor: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            actor: actor.into(),
            is_system: false,
        }
    }

    /// Convenience constructor for system initiated commits
    pub fn system(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            actor: "system".to_string(),
            is_system: true,
        }
    }
}
