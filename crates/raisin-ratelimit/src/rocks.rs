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

//! RocksDB-backed rate limiter using sliding window algorithm

use raisin_context::{RateLimitInfo, RateLimiter};
use rocksdb::{Options, DB};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Rate limit bucket stored in RocksDB
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RateBucket {
    /// Timestamps of requests in the current window
    timestamps: Vec<u64>,
    /// Last cleanup time
    last_cleanup: u64,
}

impl RateBucket {
    fn new() -> Self {
        Self {
            timestamps: Vec::new(),
            last_cleanup: current_timestamp(),
        }
    }

    /// Remove timestamps outside the window
    fn cleanup(&mut self, window_secs: u64) {
        let now = current_timestamp();
        let cutoff = now.saturating_sub(window_secs);
        self.timestamps.retain(|&ts| ts >= cutoff);
        self.last_cleanup = now;
    }

    /// Add a new request timestamp
    fn record(&mut self) {
        self.timestamps.push(current_timestamp());
    }

    /// Count requests in the current window
    fn count_in_window(&self, window_secs: u64) -> usize {
        let now = current_timestamp();
        let cutoff = now.saturating_sub(window_secs);
        self.timestamps.iter().filter(|&&ts| ts >= cutoff).count()
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time must be after UNIX_EPOCH")
        .as_secs()
}

/// RocksDB-backed rate limiter
///
/// Uses a sliding window algorithm to track request counts.
/// Periodically cleans up old timestamps to prevent unbounded growth.
pub struct RocksRateLimiter {
    db: Arc<DB>,
}

impl RocksRateLimiter {
    /// Open or create a RocksDB database for rate limiting
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rocksdb::Error> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let db = DB::open(&opts, path)?;
        Ok(Self { db: Arc::new(db) })
    }

    /// Get or create a bucket for a key
    fn get_bucket(&self, key: &str) -> RateBucket {
        match self.db.get(key.as_bytes()) {
            Ok(Some(bytes)) => rmp_serde::from_slice(&bytes).unwrap_or_else(|_| RateBucket::new()),
            _ => RateBucket::new(),
        }
    }

    /// Store a bucket
    fn put_bucket(&self, key: &str, bucket: &RateBucket) -> Result<(), rocksdb::Error> {
        let bytes = rmp_serde::to_vec(bucket).expect("RateBucket serialization should never fail");
        self.db.put(key.as_bytes(), bytes)?;
        Ok(())
    }

    /// Internal rate limit check and record
    async fn check_and_record_internal(
        &self,
        key: &str,
        limit: usize,
        window: Duration,
    ) -> RateLimitInfo {
        let window_secs = window.as_secs();
        let mut bucket = self.get_bucket(key);

        // Cleanup old timestamps periodically
        if current_timestamp() - bucket.last_cleanup > window_secs / 2 {
            bucket.cleanup(window_secs);
        }

        let current = bucket.count_in_window(window_secs);
        let allowed = current < limit;

        if allowed {
            bucket.record();
            if let Err(e) = self.put_bucket(key, &bucket) {
                eprintln!("Failed to update rate limit for key '{}': {}", key, e);
            }
        }

        // Calculate time until reset
        let oldest_in_window = bucket
            .timestamps
            .iter()
            .filter(|&&ts| ts >= current_timestamp().saturating_sub(window_secs))
            .min()
            .copied();

        let reset_after = if let Some(oldest) = oldest_in_window {
            let elapsed_since_oldest = current_timestamp().saturating_sub(oldest);
            let remaining = window_secs.saturating_sub(elapsed_since_oldest);
            Duration::from_secs(remaining)
        } else {
            window
        };

        RateLimitInfo {
            allowed,
            current,
            limit,
            reset_after,
        }
    }
}

impl RateLimiter for RocksRateLimiter {
    async fn check_rate(&self, key: &str, limit: usize, window: Duration) -> RateLimitInfo {
        self.check_and_record_internal(key, limit, window).await
    }

    async fn record(&self, key: &str, window: Duration) {
        let window_secs = window.as_secs();
        let mut bucket = self.get_bucket(key);
        bucket.record();
        bucket.cleanup(window_secs);
        if let Err(e) = self.put_bucket(key, &bucket) {
            eprintln!("Failed to record rate limit for key '{}': {}", key, e);
        }
    }

    async fn reset(&self, key: &str) {
        if let Err(e) = self.db.delete(key.as_bytes()) {
            eprintln!("Failed to reset rate limit for key '{}': {}", key, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let dir = tempdir().unwrap();
        let limiter = RocksRateLimiter::open(dir.path()).unwrap();

        // First request should be allowed
        let info = limiter
            .check_rate("test-key", 5, Duration::from_secs(60))
            .await;
        assert!(info.allowed);
        assert_eq!(info.current, 0); // Count before recording

        // Record a few more
        for _ in 0..4 {
            limiter.record("test-key", Duration::from_secs(60)).await;
        }

        // 6th request should be denied (already have 5)
        let info = limiter
            .check_rate("test-key", 5, Duration::from_secs(60))
            .await;
        assert!(!info.allowed);
        assert_eq!(info.current, 5);
    }

    #[tokio::test]
    async fn test_rate_limiter_reset() {
        let dir = tempdir().unwrap();
        let limiter = RocksRateLimiter::open(dir.path()).unwrap();

        // Fill the bucket
        for _ in 0..5 {
            limiter
                .check_rate("test-key", 5, Duration::from_secs(60))
                .await;
        }

        // Should be at limit
        let info = limiter
            .check_rate("test-key", 5, Duration::from_secs(60))
            .await;
        assert!(!info.allowed);

        // Reset
        limiter.reset("test-key").await;

        // Should be allowed again
        let info = limiter
            .check_rate("test-key", 5, Duration::from_secs(60))
            .await;
        assert!(info.allowed);
    }

    #[tokio::test]
    async fn test_rate_limiter_different_keys() {
        let dir = tempdir().unwrap();
        let limiter = RocksRateLimiter::open(dir.path()).unwrap();

        // Fill bucket for key1
        for _ in 0..5 {
            limiter.check_rate("key1", 5, Duration::from_secs(60)).await;
        }

        // key1 should be limited
        let info = limiter.check_rate("key1", 5, Duration::from_secs(60)).await;
        assert!(!info.allowed);

        // key2 should still be allowed
        let info = limiter.check_rate("key2", 5, Duration::from_secs(60)).await;
        assert!(info.allowed);
    }

    #[test]
    fn test_bucket_cleanup() {
        let mut bucket = RateBucket::new();
        let now = current_timestamp();

        // Add some old timestamps
        bucket.timestamps.push(now - 120);
        bucket.timestamps.push(now - 90);
        bucket.timestamps.push(now - 30);
        bucket.timestamps.push(now - 10);

        // Cleanup with 60 second window
        bucket.cleanup(60);

        // Should only keep timestamps from last 60 seconds
        assert_eq!(bucket.timestamps.len(), 2);
        assert!(bucket.timestamps.iter().all(|&ts| ts >= now - 60));
    }
}
