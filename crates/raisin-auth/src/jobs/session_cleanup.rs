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

//! Session cleanup job helpers.
//!
//! This module provides helpers for creating session cleanup jobs
//! that remove expired sessions from the session store.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Session cleanup job configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCleanupConfig {
    /// Maximum number of sessions to process per batch
    pub batch_size: usize,
    /// Whether to invalidate related cache entries
    pub invalidate_cache: bool,
    /// Maximum age in seconds for idle sessions to be considered expired
    /// (overrides per-session expiration if set)
    pub max_idle_seconds: Option<u64>,
    /// Whether to log each deleted session
    pub verbose_logging: bool,
}

impl Default for SessionCleanupConfig {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            invalidate_cache: true,
            max_idle_seconds: None,
            verbose_logging: false,
        }
    }
}

impl SessionCleanupConfig {
    /// Create a new session cleanup config with custom batch size
    pub fn with_batch_size(batch_size: usize) -> Self {
        Self {
            batch_size,
            ..Default::default()
        }
    }

    /// Set cache invalidation behavior
    pub fn with_cache_invalidation(mut self, invalidate: bool) -> Self {
        self.invalidate_cache = invalidate;
        self
    }

    /// Set max idle time
    pub fn with_max_idle(mut self, seconds: u64) -> Self {
        self.max_idle_seconds = Some(seconds);
        self
    }

    /// Enable verbose logging
    pub fn with_verbose_logging(mut self) -> Self {
        self.verbose_logging = true;
        self
    }

    /// Convert to metadata HashMap for JobContext
    pub fn to_metadata(&self) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert("batch_size".to_string(), serde_json::json!(self.batch_size));
        map.insert(
            "invalidate_cache".to_string(),
            serde_json::json!(self.invalidate_cache),
        );
        if let Some(max_idle) = self.max_idle_seconds {
            map.insert("max_idle_seconds".to_string(), serde_json::json!(max_idle));
        }
        map.insert(
            "verbose_logging".to_string(),
            serde_json::json!(self.verbose_logging),
        );
        map
    }

    /// Parse from metadata HashMap
    pub fn from_metadata(metadata: &HashMap<String, serde_json::Value>) -> Self {
        Self {
            batch_size: metadata
                .get("batch_size")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(1000),
            invalidate_cache: metadata
                .get("invalidate_cache")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            max_idle_seconds: metadata.get("max_idle_seconds").and_then(|v| v.as_u64()),
            verbose_logging: metadata
                .get("verbose_logging")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }
    }
}

/// Result of a session cleanup job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCleanupResult {
    /// Number of sessions scanned
    pub sessions_scanned: usize,
    /// Number of expired sessions deleted
    pub sessions_deleted: usize,
    /// Number of cache entries invalidated
    pub cache_entries_invalidated: usize,
    /// Whether there are more sessions to process
    pub has_more: bool,
    /// Any errors encountered (non-fatal)
    pub errors: Vec<String>,
}

impl SessionCleanupResult {
    /// Create a new empty result
    pub fn new() -> Self {
        Self {
            sessions_scanned: 0,
            sessions_deleted: 0,
            cache_entries_invalidated: 0,
            has_more: false,
            errors: Vec::new(),
        }
    }

    /// Add a scan count
    pub fn scanned(&mut self, count: usize) {
        self.sessions_scanned += count;
    }

    /// Add a delete count
    pub fn deleted(&mut self, count: usize) {
        self.sessions_deleted += count;
    }

    /// Add cache invalidation count
    pub fn invalidated(&mut self, count: usize) {
        self.cache_entries_invalidated += count;
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

impl Default for SessionCleanupResult {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SessionCleanupConfig::default();
        assert_eq!(config.batch_size, 1000);
        assert!(config.invalidate_cache);
        assert!(config.max_idle_seconds.is_none());
        assert!(!config.verbose_logging);
    }

    #[test]
    fn test_config_builder() {
        let config = SessionCleanupConfig::with_batch_size(500)
            .with_cache_invalidation(false)
            .with_max_idle(3600)
            .with_verbose_logging();

        assert_eq!(config.batch_size, 500);
        assert!(!config.invalidate_cache);
        assert_eq!(config.max_idle_seconds, Some(3600));
        assert!(config.verbose_logging);
    }

    #[test]
    fn test_metadata_roundtrip() {
        let original = SessionCleanupConfig::with_batch_size(500).with_max_idle(7200);

        let metadata = original.to_metadata();
        let restored = SessionCleanupConfig::from_metadata(&metadata);

        assert_eq!(restored.batch_size, original.batch_size);
        assert_eq!(restored.max_idle_seconds, original.max_idle_seconds);
    }

    #[test]
    fn test_result_tracking() {
        let mut result = SessionCleanupResult::new();
        result.scanned(100);
        result.deleted(25);
        result.invalidated(25);
        result.set_has_more(true);
        result.add_error("Test error");

        assert_eq!(result.sessions_scanned, 100);
        assert_eq!(result.sessions_deleted, 25);
        assert_eq!(result.cache_entries_invalidated, 25);
        assert!(result.has_more);
        assert_eq!(result.errors.len(), 1);
    }
}
