//! Rate limiting trait and types

use std::future::Future;
use std::time::Duration;

/// Information about rate limit status
#[derive(Debug, Clone)]
pub struct RateLimitInfo {
    /// Whether the request is allowed
    pub allowed: bool,

    /// Current usage count in the window
    pub current: usize,

    /// Maximum allowed in the window
    pub limit: usize,

    /// Time until the limit resets
    pub reset_after: Duration,
}

impl RateLimitInfo {
    /// Check if the rate limit was exceeded
    pub fn is_exceeded(&self) -> bool {
        !self.allowed
    }

    /// Get the remaining requests in this window
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.current)
    }
}

/// Trait for rate limiting implementations
///
/// Implement this trait to provide custom rate limiting logic.
/// The built-in implementation in `raisin-ratelimit` uses RocksDB
/// with a token bucket algorithm.
///
/// # Examples
///
/// ```rust
/// use raisin_context::{RateLimiter, RateLimitInfo};
/// use std::time::Duration;
///
/// struct SimpleRateLimiter;
///
/// impl RateLimiter for SimpleRateLimiter {
///     async fn check_rate(&self, key: &str, limit: usize, window: Duration) -> RateLimitInfo {
///         // Simple implementation - always allow
///         RateLimitInfo {
///             allowed: true,
///             current: 0,
///             limit,
///             reset_after: window,
///         }
///     }
/// }
/// ```
pub trait RateLimiter: Send + Sync {
    /// Check if a request is allowed under the rate limit
    ///
    /// # Arguments
    /// * `key` - Unique identifier for the rate limit (e.g., tenant_id, user_id, IP)
    /// * `limit` - Maximum number of requests allowed in the window
    /// * `window` - Time window for the rate limit
    ///
    /// # Returns
    /// Information about the rate limit status, including whether the request is allowed
    fn check_rate(
        &self,
        key: &str,
        limit: usize,
        window: Duration,
    ) -> impl Future<Output = RateLimitInfo> + Send;

    /// Record a successful request (increment counter)
    ///
    /// Some implementations may combine check and record in a single operation
    fn record(&self, key: &str, window: Duration) -> impl Future<Output = ()> + Send {
        async move {
            // Default implementation does nothing
            let _ = (key, window);
        }
    }

    /// Reset rate limit for a key (useful for testing or admin overrides)
    fn reset(&self, key: &str) -> impl Future<Output = ()> + Send {
        async move {
            let _ = key;
        }
    }
}

/// A no-op rate limiter that always allows requests
#[allow(dead_code)]
pub struct NoOpRateLimiter;

impl RateLimiter for NoOpRateLimiter {
    async fn check_rate(&self, _key: &str, limit: usize, window: Duration) -> RateLimitInfo {
        RateLimitInfo {
            allowed: true,
            current: 0,
            limit,
            reset_after: window,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_noop_rate_limiter() {
        let limiter = NoOpRateLimiter;
        let info = limiter
            .check_rate("test-key", 100, Duration::from_secs(60))
            .await;

        assert!(info.allowed);
        assert_eq!(info.current, 0);
        assert_eq!(info.limit, 100);
        assert_eq!(info.remaining(), 100);
    }

    #[test]
    fn test_rate_limit_info() {
        let info = RateLimitInfo {
            allowed: false,
            current: 105,
            limit: 100,
            reset_after: Duration::from_secs(30),
        };

        assert!(info.is_exceeded());
        assert_eq!(info.remaining(), 0);
    }
}
