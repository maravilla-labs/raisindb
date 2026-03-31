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

//! Timeout handling for flow execution
//!
//! Provides configurable timeouts at flow and step levels.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;

/// Timeout configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutConfig {
    /// Step-level timeout in milliseconds (None = use flow default)
    pub step_timeout_ms: Option<u64>,
    /// Flow-level timeout in milliseconds (None = no timeout)
    pub flow_timeout_ms: Option<u64>,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            step_timeout_ms: Some(300000), // 5 minutes default
            flow_timeout_ms: None,
        }
    }
}

impl TimeoutConfig {
    /// Get effective step timeout
    pub fn effective_step_timeout(&self) -> Option<Duration> {
        self.step_timeout_ms.map(Duration::from_millis)
    }

    /// Get effective flow timeout
    pub fn effective_flow_timeout(&self) -> Option<Duration> {
        self.flow_timeout_ms.map(Duration::from_millis)
    }
}

/// Result of a timed operation
#[derive(Debug)]
pub enum TimedResult<T, E> {
    /// Operation completed successfully
    Ok(T),
    /// Operation failed with error
    Err(E),
    /// Operation timed out
    TimedOut,
}

/// Execute an async operation with timeout
pub async fn with_timeout<T, E, F>(duration: Duration, operation: F) -> TimedResult<T, E>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    match timeout(duration, operation).await {
        Ok(Ok(value)) => TimedResult::Ok(value),
        Ok(Err(err)) => TimedResult::Err(err),
        Err(_) => TimedResult::TimedOut,
    }
}

/// Execute an async operation with optional timeout
pub async fn with_optional_timeout<T, E, F>(
    timeout_ms: Option<u64>,
    operation: F,
) -> TimedResult<T, E>
where
    F: std::future::Future<Output = Result<T, E>>,
{
    match timeout_ms {
        Some(ms) => with_timeout(Duration::from_millis(ms), operation).await,
        None => match operation.await {
            Ok(value) => TimedResult::Ok(value),
            Err(err) => TimedResult::Err(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_timeout_success() {
        let result: TimedResult<i32, &str> =
            with_timeout(Duration::from_millis(100), async { Ok(42) }).await;

        match result {
            TimedResult::Ok(v) => assert_eq!(v, 42),
            _ => panic!("Expected Ok"),
        }
    }

    #[tokio::test]
    async fn test_timeout_exceeded() {
        let result: TimedResult<i32, &str> = with_timeout(Duration::from_millis(10), async {
            sleep(Duration::from_millis(100)).await;
            Ok(42)
        })
        .await;

        assert!(matches!(result, TimedResult::TimedOut));
    }

    #[tokio::test]
    async fn test_optional_timeout_none() {
        let result: TimedResult<i32, &str> = with_optional_timeout(None, async { Ok(42) }).await;

        match result {
            TimedResult::Ok(v) => assert_eq!(v, 42),
            _ => panic!("Expected Ok"),
        }
    }

    #[test]
    fn test_timeout_config_default() {
        let config = TimeoutConfig::default();
        assert_eq!(config.step_timeout_ms, Some(300000));
        assert_eq!(config.flow_timeout_ms, None);
    }

    #[test]
    fn test_effective_step_timeout() {
        let config = TimeoutConfig {
            step_timeout_ms: Some(5000),
            flow_timeout_ms: None,
        };
        assert_eq!(
            config.effective_step_timeout(),
            Some(Duration::from_millis(5000))
        );
    }

    #[test]
    fn test_effective_flow_timeout() {
        let config = TimeoutConfig {
            step_timeout_ms: None,
            flow_timeout_ms: Some(60000),
        };
        assert_eq!(
            config.effective_flow_timeout(),
            Some(Duration::from_millis(60000))
        );
    }

    #[tokio::test]
    async fn test_timeout_error() {
        let result: TimedResult<i32, &str> =
            with_timeout(Duration::from_millis(100), async { Err("error") }).await;

        match result {
            TimedResult::Err(e) => assert_eq!(e, "error"),
            _ => panic!("Expected Err"),
        }
    }

    #[tokio::test]
    async fn test_optional_timeout_some() {
        let result: TimedResult<i32, &str> = with_optional_timeout(Some(100), async {
            sleep(Duration::from_millis(200)).await;
            Ok(42)
        })
        .await;

        assert!(matches!(result, TimedResult::TimedOut));
    }
}
