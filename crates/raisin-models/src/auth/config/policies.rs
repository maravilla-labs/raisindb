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

//! Session settings, password policy, and rate limiting configuration.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// Session settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSettings {
    /// Access token duration in seconds
    #[serde(default = "default_access_token_duration")]
    pub access_token_duration_seconds: u64,

    /// Refresh token duration in seconds
    #[serde(default = "default_refresh_token_duration")]
    pub refresh_token_duration_seconds: u64,

    /// Maximum concurrent sessions per user
    #[serde(default = "default_max_sessions")]
    pub max_sessions_per_user: u32,

    /// Sudo mode threshold in seconds (require re-auth after this time)
    #[serde(default = "default_sudo_threshold")]
    pub sudo_threshold_seconds: u64,

    /// Whether to rotate refresh tokens on each use
    #[serde(default = "default_true")]
    pub rotate_refresh_tokens: bool,

    /// Whether to revoke all tokens in family on reuse detection
    #[serde(default = "default_true")]
    pub revoke_on_reuse_detection: bool,
}

fn default_access_token_duration() -> u64 {
    3600 // 1 hour
}

fn default_refresh_token_duration() -> u64 {
    30 * 24 * 3600 // 30 days
}

fn default_max_sessions() -> u32 {
    10
}

fn default_sudo_threshold() -> u64 {
    300 // 5 minutes
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            access_token_duration_seconds: default_access_token_duration(),
            refresh_token_duration_seconds: default_refresh_token_duration(),
            max_sessions_per_user: default_max_sessions(),
            sudo_threshold_seconds: default_sudo_threshold(),
            rotate_refresh_tokens: true,
            revoke_on_reuse_detection: true,
        }
    }
}

/// Password policy for local authentication
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PasswordPolicy {
    /// Minimum password length
    #[serde(default = "default_min_length")]
    pub min_length: u32,

    /// Maximum password length
    #[serde(default = "default_max_length")]
    pub max_length: u32,

    /// Require at least one uppercase letter
    #[serde(default = "default_true")]
    pub require_uppercase: bool,

    /// Require at least one lowercase letter
    #[serde(default = "default_true")]
    pub require_lowercase: bool,

    /// Require at least one digit
    #[serde(default = "default_true")]
    pub require_digit: bool,

    /// Require at least one special character
    #[serde(default)]
    pub require_special: bool,

    /// Number of previous passwords to check against
    #[serde(default)]
    pub password_history_count: u32,

    /// Password expiry in days (0 = never)
    #[serde(default)]
    pub expiry_days: u32,
}

fn default_min_length() -> u32 {
    12
}

fn default_max_length() -> u32 {
    128
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            min_length: default_min_length(),
            max_length: default_max_length(),
            require_uppercase: true,
            require_lowercase: true,
            require_digit: true,
            require_special: false,
            password_history_count: 0,
            expiry_days: 0,
        }
    }
}

impl PasswordPolicy {
    /// Validate a password against this policy
    pub fn validate(&self, password: &str) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if password.len() < self.min_length as usize {
            errors.push(format!(
                "Password must be at least {} characters",
                self.min_length
            ));
        }

        if password.len() > self.max_length as usize {
            errors.push(format!(
                "Password must be at most {} characters",
                self.max_length
            ));
        }

        if self.require_uppercase && !password.chars().any(|c| c.is_uppercase()) {
            errors.push("Password must contain at least one uppercase letter".to_string());
        }

        if self.require_lowercase && !password.chars().any(|c| c.is_lowercase()) {
            errors.push("Password must contain at least one lowercase letter".to_string());
        }

        if self.require_digit && !password.chars().any(|c| c.is_ascii_digit()) {
            errors.push("Password must contain at least one digit".to_string());
        }

        if self.require_special
            && !password
                .chars()
                .any(|c| !c.is_alphanumeric() && !c.is_whitespace())
        {
            errors.push("Password must contain at least one special character".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Rate limiting settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimitSettings {
    /// Maximum login attempts per minute per IP
    #[serde(default = "default_max_attempts")]
    pub max_attempts_per_minute: u32,

    /// Account lockout duration in minutes after too many failures
    #[serde(default = "default_lockout_duration")]
    pub lockout_duration_minutes: u32,

    /// Number of failed attempts before lockout
    #[serde(default = "default_lockout_threshold")]
    pub lockout_threshold: u32,

    /// Maximum magic link requests per hour per email
    #[serde(default = "default_magic_link_rate")]
    pub magic_link_per_hour: u32,
}

fn default_max_attempts() -> u32 {
    10
}

fn default_lockout_duration() -> u32 {
    15
}

fn default_lockout_threshold() -> u32 {
    5
}

fn default_magic_link_rate() -> u32 {
    5
}

impl Default for RateLimitSettings {
    fn default() -> Self {
        Self {
            max_attempts_per_minute: default_max_attempts(),
            lockout_duration_minutes: default_lockout_duration(),
            lockout_threshold: default_lockout_threshold(),
            magic_link_per_hour: default_magic_link_rate(),
        }
    }
}
