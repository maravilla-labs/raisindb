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

//! Repository-level authentication configuration.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_roles() -> Vec<String> {
    vec!["viewer".to_string()]
}

/// Repository-level authentication configuration.
///
/// Controls authentication behavior for a specific repository,
/// including whether registration is allowed and default roles for new users.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoAuthConfig {
    /// Repository ID
    pub repo_id: String,

    /// Whether user registration is allowed for this repository
    #[serde(default = "default_true")]
    pub allow_registration: bool,

    /// Default roles to assign to new users (e.g., ["viewer"])
    /// If empty, falls back to ["viewer"]
    #[serde(default = "default_roles")]
    pub default_roles: Vec<String>,

    /// Whether to require email verification before granting access
    #[serde(default)]
    pub require_email_verification: bool,

    /// Whether to auto-approve access (true) or require admin approval (false)
    #[serde(default = "default_true")]
    pub auto_approve_access: bool,

    /// Custom welcome message for registration page
    #[serde(skip_serializing_if = "Option::is_none")]
    pub welcome_message: Option<String>,

    /// Whether anonymous (unauthenticated) access is enabled for this repository.
    /// - Some(true): Explicitly enable anonymous access
    /// - Some(false): Explicitly disable anonymous access
    /// - None: Inherit from tenant-level setting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anonymous_enabled: Option<bool>,

    /// CORS allowed origins for authentication endpoints.
    /// These origins are allowed to make cross-origin requests to /auth/{repo}/* endpoints.
    /// Example: ["http://localhost:5173", "https://app.example.com"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cors_allowed_origins: Vec<String>,
}

impl Default for RepoAuthConfig {
    fn default() -> Self {
        Self {
            repo_id: String::new(),
            allow_registration: true,
            default_roles: default_roles(),
            require_email_verification: false,
            auto_approve_access: true,
            welcome_message: None,
            anonymous_enabled: None,
            cors_allowed_origins: Vec::new(),
        }
    }
}

impl RepoAuthConfig {
    /// Create a new config for a repository with default settings
    pub fn new(repo_id: String) -> Self {
        Self {
            repo_id,
            ..Default::default()
        }
    }

    /// Get the effective default roles (never empty)
    pub fn effective_default_roles(&self) -> Vec<String> {
        if self.default_roles.is_empty() {
            default_roles()
        } else {
            self.default_roles.clone()
        }
    }

    /// Check if anonymous access is enabled for this repository.
    ///
    /// Returns the repository-level setting if explicitly configured,
    /// otherwise falls back to the tenant-level default.
    pub fn is_anonymous_enabled(&self, tenant_default: bool) -> bool {
        self.anonymous_enabled.unwrap_or(tenant_default)
    }
}
