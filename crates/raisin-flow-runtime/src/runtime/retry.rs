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

//! Retry logic with exponential backoff
//!
//! Provides configurable retry behavior for flow steps.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Retry configuration for a flow step
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (0 = no retries)
    pub max_retries: u32,
    /// Base delay in milliseconds for exponential backoff
    pub base_delay_ms: u64,
    /// Maximum delay cap in milliseconds
    pub max_delay_ms: u64,
    /// Jitter factor (0.0 to 1.0) to add randomness
    pub jitter_factor: f64,
}

impl RetryConfig {
    /// Create a new retry configuration
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            base_delay_ms: 1000,  // 1 second
            max_delay_ms: 120000, // 2 minutes
            jitter_factor: 0.1,
        }
    }

    /// Calculate delay for a given attempt number (0-indexed)
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        if attempt >= self.max_retries {
            return Duration::ZERO;
        }

        // Exponential backoff: base * 2^attempt
        let delay_ms = self
            .base_delay_ms
            .saturating_mul(2u64.saturating_pow(attempt));
        let capped_delay = delay_ms.min(self.max_delay_ms);

        // Add jitter
        let jitter_range = (capped_delay as f64 * self.jitter_factor) as u64;
        let jitter = if jitter_range > 0 {
            rand::random::<u64>() % jitter_range
        } else {
            0
        };

        Duration::from_millis(capped_delay + jitter)
    }

    /// Check if another retry is allowed
    pub fn should_retry(&self, attempt: u32) -> bool {
        attempt < self.max_retries
    }
}

/// Predefined retry strategies
pub mod strategies {
    use super::RetryConfig;

    /// No retries
    pub fn none() -> RetryConfig {
        RetryConfig::default()
    }

    /// Quick retry for transient failures (3 retries, 1s base)
    pub fn quick() -> RetryConfig {
        RetryConfig {
            max_retries: 3,
            base_delay_ms: 1000,
            max_delay_ms: 10000,
            jitter_factor: 0.1,
        }
    }

    /// Standard retry for most operations (5 retries, 2s base)
    pub fn standard() -> RetryConfig {
        RetryConfig {
            max_retries: 5,
            base_delay_ms: 2000,
            max_delay_ms: 60000,
            jitter_factor: 0.15,
        }
    }

    /// Aggressive retry for critical operations (10 retries, 5s base)
    pub fn aggressive() -> RetryConfig {
        RetryConfig {
            max_retries: 10,
            base_delay_ms: 5000,
            max_delay_ms: 120000,
            jitter_factor: 0.2,
        }
    }

    /// Retry strategy optimized for LLM calls (rate limits, transient errors)
    pub fn llm() -> RetryConfig {
        RetryConfig {
            max_retries: 5,
            base_delay_ms: 10000, // 10 seconds - LLM rate limits need longer waits
            max_delay_ms: 120000,
            jitter_factor: 0.25,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff() {
        let config = RetryConfig {
            max_retries: 5,
            base_delay_ms: 1000,
            max_delay_ms: 60000,
            jitter_factor: 0.0, // No jitter for predictable test
        };

        assert_eq!(config.calculate_delay(0), Duration::from_millis(1000));
        assert_eq!(config.calculate_delay(1), Duration::from_millis(2000));
        assert_eq!(config.calculate_delay(2), Duration::from_millis(4000));
        assert_eq!(config.calculate_delay(3), Duration::from_millis(8000));
    }

    #[test]
    fn test_max_delay_cap() {
        let config = RetryConfig {
            max_retries: 10,
            base_delay_ms: 10000,
            max_delay_ms: 30000,
            jitter_factor: 0.0,
        };

        // Should be capped at max_delay_ms
        assert_eq!(config.calculate_delay(5), Duration::from_millis(30000));
    }

    #[test]
    fn test_should_retry() {
        let config = RetryConfig::new(3);
        assert!(config.should_retry(0));
        assert!(config.should_retry(1));
        assert!(config.should_retry(2));
        assert!(!config.should_retry(3));
    }

    #[test]
    fn test_strategies_none() {
        let config = strategies::none();
        assert_eq!(config.max_retries, 0);
    }

    #[test]
    fn test_strategies_quick() {
        let config = strategies::quick();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay_ms, 1000);
    }

    #[test]
    fn test_strategies_standard() {
        let config = strategies::standard();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.base_delay_ms, 2000);
    }

    #[test]
    fn test_strategies_aggressive() {
        let config = strategies::aggressive();
        assert_eq!(config.max_retries, 10);
        assert_eq!(config.base_delay_ms, 5000);
    }

    #[test]
    fn test_strategies_llm() {
        let config = strategies::llm();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.base_delay_ms, 10000);
    }
}
