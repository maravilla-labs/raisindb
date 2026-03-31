// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! API response types for dependency status.
//!
//! These types are used by the admin console API to display
//! dependency information and installation instructions.

use crate::checker::{DependencyChecker, DependencyStatus};

/// Format for admin console API response.
#[derive(Debug, serde::Serialize)]
pub struct DependencyApiStatus {
    /// Dependency identifier
    pub name: String,
    /// Human-readable name
    pub display_name: String,
    /// Short description
    pub description: String,
    /// Current status
    pub status: ApiStatus,
    /// Version if installed
    pub version: Option<String>,
    /// Features that are affected if missing
    pub required_for: Vec<String>,
    /// Installation instructions for current platform
    pub install_instructions: Option<ApiInstallInstructions>,
}

/// Status for API response.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiStatus {
    Installed,
    Skipped,
    NotAvailable,
}

/// Installation instructions for API response.
#[derive(Debug, serde::Serialize)]
pub struct ApiInstallInstructions {
    /// Target platform name
    pub platform: String,
    /// Package manager name if applicable
    pub package_manager: Option<String>,
    /// Full install command
    pub command: String,
    /// Whether sudo is required
    pub needs_sudo: bool,
    /// Post-install instructions
    pub post_install: Option<String>,
    /// Manual installation URL
    pub manual_url: Option<String>,
}

impl DependencyApiStatus {
    /// Create API status from a dependency checker.
    pub fn from_checker(checker: &dyn DependencyChecker, skipped: bool) -> Self {
        let status = checker.check();
        let instructions = checker.install_instructions();

        let (api_status, version) = match &status {
            DependencyStatus::Installed { version, .. } => {
                (ApiStatus::Installed, Some(version.clone()))
            }
            _ if skipped => (ApiStatus::Skipped, None),
            _ => (ApiStatus::NotAvailable, None),
        };

        Self {
            name: checker.name().to_string(),
            display_name: checker.display_name().to_string(),
            description: checker.description().to_string(),
            status: api_status,
            version,
            required_for: checker
                .features_affected()
                .iter()
                .map(|s| s.to_string())
                .collect(),
            install_instructions: Some(ApiInstallInstructions {
                platform: instructions.platform.display_name().to_string(),
                package_manager: instructions.package_manager.clone(),
                command: instructions.command.clone(),
                needs_sudo: instructions.needs_sudo,
                post_install: instructions.post_install.clone(),
                manual_url: instructions.manual_url.clone(),
            }),
        }
    }
}
