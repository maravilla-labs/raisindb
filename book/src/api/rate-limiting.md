# Rate Limiting API

Rate limiting types and implementations.

## RateLimiter Trait

Interface for rate limiting implementations.

```rust
pub trait RateLimiter: Send + Sync {
    fn check_rate(
        &self,
        key: &str,
        limit: usize,
        window: Duration,
    ) -> impl Future<Output = RateLimitInfo> + Send;

    // Optional: record a request (default: no-op)
    fn record(&self, key: &str, window: Duration) -> impl Future<Output = ()> + Send;

    // Optional: reset a key (default: no-op)
    fn reset(&self, key: &str) -> impl Future<Output = ()> + Send;
}
```

## RateLimitInfo

Information about rate limit status.

```rust
pub struct RateLimitInfo {
    pub allowed: bool,
    pub current: usize,
    pub limit: usize,
    pub reset_after: Duration,
}
```

### Methods

#### `is_exceeded(&self) -> bool`

Check if limit was exceeded.

#### `remaining(&self) -> usize`

Get remaining requests in window.

## RocksRateLimiter

RocksDB-backed rate limiter using a sliding window algorithm.

```rust
use raisin_ratelimit::RocksRateLimiter;
use raisin_context::RateLimiter;
use std::time::Duration;

let limiter = RocksRateLimiter::open("./rate-limits")?;

// check_rate both checks AND records on success
let info = limiter
    .check_rate("tenant-123", 100, Duration::from_secs(60))
    .await;

if info.allowed {
    println!("Request allowed ({} remaining)", info.remaining());
} else {
    println!("Rate limit exceeded, reset in {:?}", info.reset_after);
}
```

### Methods

#### `open(path: impl AsRef<Path>) -> Result<Self, rocksdb::Error>`

Create or open a rate limiter RocksDB database.

#### Implementation of RateLimiter trait

The RocksDB implementation combines check and record in `check_rate()` - if the request is allowed, it is automatically recorded. The `record()` method can be used separately for manual recording. The `reset()` method deletes the key from the database.

## NoOpRateLimiter

No-op implementation that always allows requests.

```rust
use raisin_context::NoOpRateLimiter;

let limiter = NoOpRateLimiter;
let info = limiter.check_rate("key", 100, Duration::from_secs(60)).await;
assert!(info.allowed); // Always true
```

## Operation

Trackable operations for rate limiting.

```rust
pub enum Operation {
    CreateNode,
    UpdateNode,
    DeleteNode,
    Query,
    Upload,
    Custom(String),
}
```
