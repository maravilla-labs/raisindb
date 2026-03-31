// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Types for the dependency manager.

/// Result of the setup process.
#[derive(Debug, Default)]
pub struct SetupResult {
    /// Dependencies that are installed and available
    pub installed: Vec<String>,
    /// Dependencies that were skipped by user
    pub skipped: Vec<String>,
    /// Dependencies that are unavailable (required but missing)
    pub unavailable: Vec<String>,
}

impl SetupResult {
    /// Check if all dependencies are available.
    pub fn all_available(&self) -> bool {
        self.unavailable.is_empty()
    }
}

/// Result of trying to enable a dependency.
#[derive(Debug)]
pub enum EnableResult {
    /// Dependency is now enabled
    Enabled {
        /// Version that was detected
        version: String,
    },
    /// Dependency is still not installed
    NotInstalled {
        /// Installation instructions
        instructions: crate::InstallInstructions,
    },
}

/// Errors that can occur during setup.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// User chose to exit setup
    #[error("User exited setup")]
    UserExit,

    /// Unknown dependency name
    #[error("Unknown dependency: {0}")]
    UnknownDependency(String),

    /// IO error during setup
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
