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

//! Token cleanup job helpers.
//!
//! This module provides helpers for creating token cleanup jobs
//! that remove expired one-time tokens (magic links, API keys, invite tokens, etc.).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Token types that can be cleaned up
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupTokenType {
    /// Magic link tokens
    MagicLink,
    /// API access tokens
    ApiKey,
    /// Invitation tokens
    Invite,
    /// Password reset tokens
    PasswordReset,
    /// Email verification tokens
    EmailVerification,
    /// All token types
    All,
}

impl CleanupTokenType {
    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MagicLink => "magic_link",
            Self::ApiKey => "api_key",
            Self::Invite => "invite",
            Self::PasswordReset => "password_reset",
            Self::EmailVerification => "email_verification",
            Self::All => "all",
        }
    }

    /// Parse from string representation
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "magic_link" => Some(Self::MagicLink),
            "api_key" => Some(Self::ApiKey),
            "invite" => Some(Self::Invite),
            "password_reset" => Some(Self::PasswordReset),
            "email_verification" => Some(Self::EmailVerification),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

impl std::fmt::Display for CleanupTokenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Token cleanup job configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCleanupConfig {
    /// Maximum number of tokens to process per batch
    pub batch_size: usize,
    /// Types of tokens to clean up
    pub token_types: Vec<CleanupTokenType>,
    /// Whether to log each deleted token
    pub verbose_logging: bool,
    /// Grace period in seconds after expiration before deletion
    /// (allows for clock skew and retry scenarios)
    pub grace_period_seconds: u64,
}

impl Default for TokenCleanupConfig {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            token_types: vec![CleanupTokenType::All],
            verbose_logging: false,
            grace_period_seconds: 300, // 5 minutes grace period
        }
    }
}

impl TokenCleanupConfig {
    /// Create a new token cleanup config with custom batch size
    pub fn with_batch_size(batch_size: usize) -> Self {
        Self {
            batch_size,
            ..Default::default()
        }
    }

    /// Set specific token types to clean up
    pub fn with_token_types(mut self, types: Vec<CleanupTokenType>) -> Self {
        self.token_types = types;
        self
    }

    /// Enable verbose logging
    pub fn with_verbose_logging(mut self) -> Self {
        self.verbose_logging = true;
        self
    }

    /// Set grace period
    pub fn with_grace_period(mut self, seconds: u64) -> Self {
        self.grace_period_seconds = seconds;
        self
    }

    /// Convert to metadata HashMap for JobContext
    pub fn to_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert("batch_size".to_string(), serde_json::json!(self.batch_size));
        map.insert(
            "token_types".to_string(),
            serde_json::json!(self
                .token_types
                .iter()
                .map(|t| t.as_str())
                .collect::<Vec<_>>()),
        );
        map.insert(
            "verbose_logging".to_string(),
            serde_json::json!(self.verbose_logging),
        );
        map.insert(
            "grace_period_seconds".to_string(),
            serde_json::json!(self.grace_period_seconds),
        );
        map
    }

    /// Parse from metadata HashMap
    pub fn from_metadata(metadata: &HashMap<String, serde_json::Value>) -> Self {
        let token_types = metadata
            .get("token_types")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(CleanupTokenType::parse)
                    .collect()
            })
            .unwrap_or_else(|| vec![CleanupTokenType::All]);

        Self {
            batch_size: metadata
                .get("batch_size")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(1000),
            token_types,
            verbose_logging: metadata
                .get("verbose_logging")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            grace_period_seconds: metadata
                .get("grace_period_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(300),
        }
    }
}

/// Result of a token cleanup job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCleanupResult {
    /// Number of tokens scanned
    pub tokens_scanned: usize,
    /// Number of expired tokens deleted
    pub tokens_deleted: usize,
    /// Breakdown of deleted tokens by type
    pub deleted_by_type: HashMap<String, usize>,
    /// Whether there are more tokens to process
    pub has_more: bool,
    /// Any errors encountered (non-fatal)
    pub errors: Vec<String>,
}

impl TokenCleanupResult {
    /// Create a new empty result
    pub fn new() -> Self {
        Self {
            tokens_scanned: 0,
            tokens_deleted: 0,
            deleted_by_type: HashMap::new(),
            has_more: false,
            errors: Vec::new(),
        }
    }

    /// Add a scan count
    pub fn scanned(&mut self, count: usize) {
        self.tokens_scanned += count;
    }

    /// Record a deleted token
    pub fn deleted(&mut self, token_type: &str) {
        self.tokens_deleted += 1;
        *self
            .deleted_by_type
            .entry(token_type.to_string())
            .or_insert(0) += 1;
    }

    /// Add multiple deleted tokens
    pub fn deleted_batch(&mut self, token_type: &str, count: usize) {
        self.tokens_deleted += count;
        *self
            .deleted_by_type
            .entry(token_type.to_string())
            .or_insert(0) += count;
    }

    /// Set has_more flag
    pub fn set_has_more(&mut self, has_more: bool) {
        self.has_more = has_more;
    }

    /// Add an error
    pub fn add_error(&mut self, error: impl Into<String>) {
        self.errors.push(error.into());
    }
}

impl Default for TokenCleanupResult {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cleanup_token_type_roundtrip() {
        let types = vec![
            CleanupTokenType::MagicLink,
            CleanupTokenType::ApiKey,
            CleanupTokenType::Invite,
            CleanupTokenType::PasswordReset,
            CleanupTokenType::EmailVerification,
            CleanupTokenType::All,
        ];

        for t in types {
            let s = t.as_str();
            let restored = CleanupTokenType::parse(s).unwrap();
            assert_eq!(restored, t);
        }
    }

    #[test]
    fn test_default_config() {
        let config = TokenCleanupConfig::default();
        assert_eq!(config.batch_size, 1000);
        assert_eq!(config.token_types, vec![CleanupTokenType::All]);
        assert!(!config.verbose_logging);
        assert_eq!(config.grace_period_seconds, 300);
    }

    #[test]
    fn test_config_builder() {
        let config = TokenCleanupConfig::with_batch_size(500)
            .with_token_types(vec![CleanupTokenType::MagicLink, CleanupTokenType::Invite])
            .with_grace_period(600)
            .with_verbose_logging();

        assert_eq!(config.batch_size, 500);
        assert_eq!(
            config.token_types,
            vec![CleanupTokenType::MagicLink, CleanupTokenType::Invite]
        );
        assert!(config.verbose_logging);
        assert_eq!(config.grace_period_seconds, 600);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let original = TokenCleanupConfig::with_batch_size(500)
            .with_token_types(vec![CleanupTokenType::MagicLink, CleanupTokenType::ApiKey])
            .with_grace_period(900);

        let metadata = original.to_metadata();
        let restored = TokenCleanupConfig::from_metadata(&metadata);

        assert_eq!(restored.batch_size, original.batch_size);
        assert_eq!(restored.token_types, original.token_types);
        assert_eq!(restored.grace_period_seconds, original.grace_period_seconds);
    }

    #[test]
    fn test_result_tracking() {
        let mut result = TokenCleanupResult::new();
        result.scanned(100);
        result.deleted("magic_link");
        result.deleted("magic_link");
        result.deleted_batch("invite", 5);
        result.set_has_more(true);
        result.add_error("Test error");

        assert_eq!(result.tokens_scanned, 100);
        assert_eq!(result.tokens_deleted, 7);
        assert_eq!(result.deleted_by_type.get("magic_link"), Some(&2));
        assert_eq!(result.deleted_by_type.get("invite"), Some(&5));
        assert!(result.has_more);
        assert_eq!(result.errors.len(), 1);
    }
}
