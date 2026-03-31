// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dependency checker trait and related types.

use crate::Platform;
use std::path::PathBuf;

/// Status of a dependency check.
#[derive(Debug, Clone, PartialEq)]
pub enum DependencyStatus {
    /// Dependency is installed and ready to use.
    Installed {
        /// Version string (e.g., "5.3.0")
        version: String,
        /// Path to the executable
        path: PathBuf,
    },
    /// Dependency is not installed.
    NotInstalled,
    /// Dependency is installed but version doesn't meet requirements.
    WrongVersion {
        /// Found version
        found: String,
        /// Required version
        required: String,
    },
    /// Error occurred while checking.
    Error(String),
}

impl DependencyStatus {
    /// Check if the dependency is installed (any version).
    pub fn is_installed(&self) -> bool {
        matches!(self, DependencyStatus::Installed { .. })
    }

    /// Get the version if installed.
    pub fn version(&self) -> Option<&str> {
        match self {
            DependencyStatus::Installed { version, .. } => Some(version),
            _ => None,
        }
    }

    /// Get the path if installed.
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            DependencyStatus::Installed { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// Installation instructions for a specific platform.
#[derive(Debug, Clone)]
pub struct InstallInstructions {
    /// Target platform.
    pub platform: Platform,
    /// Package manager being used (e.g., "brew", "apt", "winget").
    pub package_manager: Option<String>,
    /// Full install command to run.
    pub command: String,
    /// Whether the command requires sudo/admin privileges.
    pub needs_sudo: bool,
    /// Post-installation instructions (e.g., "Restart terminal").
    pub post_install: Option<String>,
    /// URL for manual download/installation.
    pub manual_url: Option<String>,
    /// Features/capabilities this dependency provides.
    pub provides: Vec<String>,
}

impl Default for InstallInstructions {
    fn default() -> Self {
        Self {
            platform: Platform::Unknown,
            package_manager: None,
            command: String::new(),
            needs_sudo: false,
            post_install: None,
            manual_url: None,
            provides: Vec::new(),
        }
    }
}

/// Trait for checking external dependencies.
///
/// Implement this trait for each external tool that RaisinDB may depend on.
pub trait DependencyChecker: Send + Sync {
    /// Unique identifier for this dependency (e.g., "tesseract").
    fn name(&self) -> &str;

    /// Human-readable display name (e.g., "Tesseract OCR").
    fn display_name(&self) -> &str;

    /// Short description of what this dependency is used for.
    fn description(&self) -> &str;

    /// Check if the dependency is installed and get its status.
    fn check(&self) -> DependencyStatus;

    /// Get installation instructions for the current platform.
    fn install_instructions(&self) -> InstallInstructions;

    /// Whether this dependency is required for basic functionality.
    ///
    /// Required dependencies will block server startup until addressed.
    /// Optional dependencies will just disable certain features.
    fn is_required(&self) -> bool {
        false
    }

    /// List of features that will be disabled if this dependency is missing.
    fn features_affected(&self) -> Vec<&str> {
        vec![]
    }
}

/// Result of checking a dependency.
#[derive(Debug)]
pub struct DependencyResult {
    /// Name of the dependency.
    pub name: String,
    /// Display name.
    pub display_name: String,
    /// Current status.
    pub status: DependencyStatus,
    /// Whether it's required.
    pub is_required: bool,
    /// Features affected if missing.
    pub features_affected: Vec<String>,
}

impl DependencyResult {
    /// Create a new result from a checker.
    pub fn from_checker<C: DependencyChecker + ?Sized>(checker: &C) -> Self {
        Self {
            name: checker.name().to_string(),
            display_name: checker.display_name().to_string(),
            status: checker.check(),
            is_required: checker.is_required(),
            features_affected: checker
                .features_affected()
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_status_is_installed() {
        let installed = DependencyStatus::Installed {
            version: "1.0.0".to_string(),
            path: PathBuf::from("/usr/bin/test"),
        };
        assert!(installed.is_installed());

        let not_installed = DependencyStatus::NotInstalled;
        assert!(!not_installed.is_installed());
    }

    #[test]
    fn test_dependency_status_version() {
        let installed = DependencyStatus::Installed {
            version: "5.3.0".to_string(),
            path: PathBuf::from("/usr/bin/tesseract"),
        };
        assert_eq!(installed.version(), Some("5.3.0"));

        let not_installed = DependencyStatus::NotInstalled;
        assert_eq!(not_installed.version(), None);
    }
}
