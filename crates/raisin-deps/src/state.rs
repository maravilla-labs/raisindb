// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Persistence types for dependency setup state.
//!
//! This module provides types for persisting dependency setup decisions
//! similar to how database migrations are tracked. Once a user has made
//! a decision about a dependency (installed or skipped), that decision
//! is persisted and they won't be prompted again on restart.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// State of all dependency setup decisions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DependencySetupState {
    /// Individual dependency records
    pub dependencies: HashMap<String, DependencyRecord>,
    /// When the state was last updated
    pub updated_at: Option<DateTime<Utc>>,
}

impl DependencySetupState {
    /// Create a new empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a dependency has been set up (either installed or skipped).
    pub fn is_setup_completed(&self, name: &str) -> bool {
        self.dependencies
            .get(name)
            .is_some_and(|r| r.status != SetupStatus::NotSetup)
    }

    /// Check if a dependency is marked as installed.
    pub fn is_installed(&self, name: &str) -> bool {
        self.dependencies
            .get(name)
            .is_some_and(|r| r.status == SetupStatus::Installed)
    }

    /// Check if a dependency was skipped.
    pub fn is_skipped(&self, name: &str) -> bool {
        self.dependencies
            .get(name)
            .is_some_and(|r| r.status == SetupStatus::Skipped)
    }

    /// Mark a dependency as installed.
    pub fn mark_installed(&mut self, name: &str, version: Option<String>) {
        let now = Utc::now();
        self.dependencies.insert(
            name.to_string(),
            DependencyRecord {
                name: name.to_string(),
                status: SetupStatus::Installed,
                checked_at: now,
                version,
            },
        );
        self.updated_at = Some(now);
    }

    /// Mark a dependency as skipped.
    pub fn mark_skipped(&mut self, name: &str) {
        let now = Utc::now();
        self.dependencies.insert(
            name.to_string(),
            DependencyRecord {
                name: name.to_string(),
                status: SetupStatus::Skipped,
                checked_at: now,
                version: None,
            },
        );
        self.updated_at = Some(now);
    }

    /// Reset a dependency to not setup (will prompt again on next startup).
    pub fn reset(&mut self, name: &str) {
        self.dependencies.remove(name);
        self.updated_at = Some(Utc::now());
    }

    /// Get the record for a specific dependency.
    pub fn get(&self, name: &str) -> Option<&DependencyRecord> {
        self.dependencies.get(name)
    }

    /// Get all dependency records.
    pub fn all(&self) -> impl Iterator<Item = &DependencyRecord> {
        self.dependencies.values()
    }
}

/// Record for a single dependency's setup status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyRecord {
    /// Dependency identifier (e.g., "tesseract")
    pub name: String,
    /// Current setup status
    pub status: SetupStatus,
    /// When this status was last checked/set
    pub checked_at: DateTime<Utc>,
    /// Version if installed
    pub version: Option<String>,
}

/// Setup status for a dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SetupStatus {
    /// User confirmed installation is complete
    Installed,
    /// User explicitly chose to skip installation
    Skipped,
    /// Not yet prompted/setup
    #[default]
    NotSetup,
}

impl SetupStatus {
    /// Check if setup is complete (either installed or skipped).
    pub fn is_complete(&self) -> bool {
        matches!(self, SetupStatus::Installed | SetupStatus::Skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_setup_state() {
        let mut state = DependencySetupState::new();

        // Initially not set up
        assert!(!state.is_setup_completed("tesseract"));
        assert!(!state.is_installed("tesseract"));
        assert!(!state.is_skipped("tesseract"));

        // Mark as installed
        state.mark_installed("tesseract", Some("5.3.0".to_string()));
        assert!(state.is_setup_completed("tesseract"));
        assert!(state.is_installed("tesseract"));
        assert!(!state.is_skipped("tesseract"));

        let record = state.get("tesseract").unwrap();
        assert_eq!(record.version.as_deref(), Some("5.3.0"));

        // Mark another as skipped
        state.mark_skipped("ffmpeg");
        assert!(state.is_setup_completed("ffmpeg"));
        assert!(!state.is_installed("ffmpeg"));
        assert!(state.is_skipped("ffmpeg"));
    }

    #[test]
    fn test_setup_status_is_complete() {
        assert!(SetupStatus::Installed.is_complete());
        assert!(SetupStatus::Skipped.is_complete());
        assert!(!SetupStatus::NotSetup.is_complete());
    }
}
