# Rate Limiting

RaisinDB includes built-in rate limiting to prevent abuse and ensure fair resource usage across tenants.

## Overview

The rate limiting system provides:

- **Per-tenant limits**: Different limits for different tenants
- **Sliding window**: Fair distribution over time
- **Persistent state**: Survives restarts (RocksDB backend)
- **Pluggable**: Implement custom rate limiters

## Rate Limiter Trait

```rust
use raisin_context::RateLimiter;
use std::time::Duration;

pub trait RateLimiter: Send + Sync {
    async fn check_rate(
        &self,
        key: &str,           // Usually tenant_id
        limit: usize,        // Max requests
        window: Duration,    // Time window
    ) -> RateLimitInfo;
}
```

## Built-in Implementation

### RocksDB Rate Limiter

**Crate**: `raisin-ratelimit`

```rust
use raisin_ratelimit::RocksRateLimiter;
use raisin_context::RateLimiter;
use std::time::Duration;

// Create limiter
let limiter = RocksRateLimiter::open("./rate-limits")?;

// Check rate limit
let info = limiter
    .check_rate("tenant-123", 100, Duration::from_secs(60))
    .await;

if info.allowed {
    println!("Request allowed");
    println!("Remaining: {}", info.remaining());
} else {
    println!("Rate limit exceeded");
    println!("Reset in: {:?}", info.reset_after);
}
```

### Algorithm

Uses a **sliding window** approach:

1. Store timestamps of recent requests
2. Remove timestamps outside the window
3. Count remaining timestamps
4. Allow if count < limit

**Benefits**:
- Fair distribution
- No burst allowance issues
- Predictable behavior

## Integration with Service Tiers

Combine with `TierProvider` for dynamic limits:

```rust
use raisin_context::{TierProvider, ServiceTier};

async fn check_tenant_rate_limit(
    limiter: &RocksRateLimiter,
    tier_provider: &impl TierProvider,
    tenant_id: &str,
) -> Result<(), StatusCode> {
    // Get tier for tenant
    let tier = tier_provider.get_tier(tenant_id).await;

    // Get rate limit from tier
    let limit = tier.rate_limit();

    // Check rate
    let info = limiter
        .check_rate(tenant_id, limit, Duration::from_secs(60))
        .await;

    if !info.allowed {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    Ok(())
}
```

## HTTP Middleware

Example with Axum:

```rust
use axum::{
    middleware::Next,
    http::{Request, StatusCode},
    response::Response,
};

async fn rate_limit_middleware<B>(
    Extension(limiter): Extension<Arc<RocksRateLimiter>>,
    Extension(ctx): Extension<TenantContext>,
    request: Request<B>,
    next: Next,
) -> Result<Response, StatusCode> {
    let info = limiter
        .check_rate(ctx.tenant_id(), 100, Duration::from_secs(60))
        .await;

    if !info.allowed {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    let mut response = next.run(request).await;

    // Add rate limit headers
    response.headers_mut().insert(
        "X-RateLimit-Limit",
        info.limit.into(),
    );
    response.headers_mut().insert(
        "X-RateLimit-Remaining",
        info.remaining().into(),
    );

    Ok(response)
}
```

## Multiple Rate Limits

Apply different limits to different operations:

```rust
// Per-minute limit
let per_minute = limiter
    .check_rate(&key, 100, Duration::from_secs(60))
    .await;

// Per-hour limit
let per_hour = limiter
    .check_rate(&format!("{}-hourly", key), 5000, Duration::from_secs(3600))
    .await;

if !per_minute.allowed || !per_hour.allowed {
    return Err("Rate limit exceeded");
}
```

## Custom Rate Limiters

Implement your own rate limiter:

```rust
use raisin_context::{RateLimiter, RateLimitInfo};

pub struct RedisRateLimiter {
    client: redis::Client,
}

impl RateLimiter for RedisRateLimiter {
    async fn check_rate(
        &self,
        key: &str,
        limit: usize,
        window: Duration,
    ) -> RateLimitInfo {
        // Implement using Redis
        todo!()
    }
}
```

## Best Practices

### 1. Separate Keys Per Operation

```rust
// Different limits for different operations
limiter.check_rate(&format!("{}-read", tenant_id), 1000, window).await;
limiter.check_rate(&format!("{}-write", tenant_id), 100, window).await;
```

### 2. Graceful Degradation

```rust
match limiter.check_rate(key, limit, window).await {
    info if !info.allowed => {
        // Return 429 with Retry-After header
        Err(StatusCode::TOO_MANY_REQUESTS)
    }
    info if info.remaining() < 10 => {
        // Warn client they're approaching limit
        warn!("Tenant {} approaching rate limit", tenant_id);
        Ok(())
    }
    _ => Ok(())
}
```

### 3. Monitoring

```rust
// Track rate limit hits
metrics::counter!(
    "rate_limit_exceeded",
    "tenant" => tenant_id
).increment(1);

// Track remaining capacity
metrics::gauge!(
    "rate_limit_remaining",
    "tenant" => tenant_id
).set(info.remaining() as f64);
```

## Performance

- **Check latency**: ~1ms with RocksDB
- **Memory usage**: Minimal (only stores timestamps in window)
- **Cleanup**: Automatic removal of old timestamps
- **Scalability**: Handles millions of keys

## Testing

```rust
#[tokio::test]
async fn test_rate_limiting() {
    let limiter = RocksRateLimiter::open(temp_dir()).unwrap();

    // First 5 requests should be allowed
    for _ in 0..5 {
        let info = limiter.check_rate("test", 5, Duration::from_secs(60)).await;
        assert!(info.allowed);
    }

    // 6th request should be denied
    let info = limiter.check_rate("test", 5, Duration::from_secs(60)).await;
    assert!(!info.allowed);
}
```
