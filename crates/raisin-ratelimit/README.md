# raisin-ratelimit

> **Status: Experimental** - This crate is a standalone prototype not yet integrated into RaisinDB.
>
> **Current limitations:**
> - Opens its own separate RocksDB instance (not shared with main storage)
> - Uses simple string keys (not the structured `KeyBuilder` format)
> - No column family isolation
> - No replication support
>
> **Future integration plan:**
> - Share the main `raisin-rocksdb` database instance
> - Use dedicated `CF_RATE_LIMIT` column family
> - Adopt structured keys: `{tenant}\0rate_limit\0{key}`
> - Evaluate replication needs (rate limits are often node-local by design)
>
> The `RateLimiter` trait is defined in `raisin-context` and used throughout the codebase with `NoOpRateLimiter` as the current default.

Persistent rate limiting for RaisinDB using RocksDB with a sliding window algorithm.

## Overview

This crate provides concrete implementations of the `RateLimiter` trait from `raisin-context`:

- **Sliding Window Algorithm**: Accurate request counting within time windows
- **Persistent Storage**: Rate limits survive restarts via RocksDB
- **Multi-Key Support**: Independent limits per tenant, user, or custom key
- **Automatic Cleanup**: Old timestamps pruned periodically

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Rate Limit Flow                         │
│                                                              │
│  Request ──> check_rate(key, limit, window)                 │
│                       │                                      │
│                       ▼                                      │
│              ┌─────────────────┐                            │
│              │ RocksRateLimiter │                           │
│              └────────┬────────┘                            │
│                       │                                      │
│         ┌─────────────┼─────────────┐                       │
│         ▼             ▼             ▼                       │
│    Get Bucket    Count in      Record if                    │
│    from DB       Window        Allowed                      │
│         │             │             │                       │
│         └─────────────┴─────────────┘                       │
│                       │                                      │
│                       ▼                                      │
│              ┌─────────────────┐                            │
│              │  RateLimitInfo  │                            │
│              │  - allowed      │                            │
│              │  - current      │                            │
│              │  - remaining    │                            │
│              │  - reset_after  │                            │
│              └─────────────────┘                            │
└─────────────────────────────────────────────────────────────┘
```

## Sliding Window Algorithm

```
Time ────────────────────────────────────────────────────────>

Window (60s)
├──────────────────────────────────────────────┤

Requests:  [ts1]    [ts2]    [ts3]    [ts4]    [ts5]   now
              │       │        │        │        │      │
              └───────┴────────┴────────┴────────┴──────┘
                         Timestamps in bucket

Count = requests where timestamp >= (now - window)
Allowed = count < limit
```

## Usage

### Basic Rate Limiting

```rust
use raisin_ratelimit::RocksRateLimiter;
use raisin_context::RateLimiter;
use std::time::Duration;

// Open rate limiter database
let limiter = RocksRateLimiter::open("./rate-limit-db")?;

// Check rate limit (100 requests per minute)
let info = limiter
    .check_rate("tenant-123", 100, Duration::from_secs(60))
    .await;

if info.allowed {
    println!("Request allowed. {} remaining", info.remaining());
    // Process request...
} else {
    println!("Rate limited. Retry after {:?}", info.reset_after);
    // Return 429 Too Many Requests
}
```

### Per-User Limits

```rust
// Combine tenant and user for per-user limits
let key = format!("{}:{}", tenant_id, user_id);
let info = limiter
    .check_rate(&key, 1000, Duration::from_secs(3600))  // 1000/hour
    .await;
```

### API Endpoint Limits

```rust
// Rate limit by endpoint
let key = format!("{}:{}:{}", tenant_id, endpoint, user_id);
let info = limiter
    .check_rate(&key, 10, Duration::from_secs(1))  // 10/second
    .await;
```

### Manual Recording

```rust
// Record without checking (fire-and-forget scenarios)
limiter.record("analytics-key", Duration::from_secs(60)).await;
```

### Reset Limits (Admin Override)

```rust
// Clear rate limit for a key
limiter.reset("user-123").await;
```

## API Reference

### RateLimiter Trait (from raisin-context)

| Method | Description |
|--------|-------------|
| `check_rate(key, limit, window)` | Check and record if allowed |
| `record(key, window)` | Record request without checking |
| `reset(key)` | Clear rate limit for key |

### RateLimitInfo

| Field | Type | Description |
|-------|------|-------------|
| `allowed` | `bool` | Whether request is permitted |
| `current` | `usize` | Current count in window |
| `limit` | `usize` | Maximum allowed in window |
| `reset_after` | `Duration` | Time until oldest entry expires |

| Method | Description |
|--------|-------------|
| `remaining()` | Returns `limit - current` |
| `is_exceeded()` | Returns `!allowed` |

## Features

```toml
[dependencies]
raisin-ratelimit = { version = "0.1" }
```

| Feature | Default | Description |
|---------|---------|-------------|
| `rocksdb-backend` | Yes | RocksDB-backed rate limiter |

## Storage Format

Rate limits are stored in RocksDB with MessagePack serialization:

```
Key:   "tenant-123"
Value: RateBucket {
    timestamps: [1704067200, 1704067210, 1704067225, ...],
    last_cleanup: 1704067225
}
```

Cleanup occurs automatically when `now - last_cleanup > window/2`.

## Integration

### Current Architecture (Standalone)

```
┌─────────────────────────────────────────────────────────────┐
│                     raisin-context                           │
│  ┌──────────────────┐  ┌──────────────────┐                 │
│  │  RateLimiter     │  │  NoOpRateLimiter │ ← currently     │
│  │  (trait)         │  │  (always allow)  │   used          │
│  └────────┬─────────┘  └──────────────────┘                 │
└───────────┼─────────────────────────────────────────────────┘
            │ implements
            ▼
┌─────────────────────────────────────────────────────────────┐
│                    raisin-ratelimit                          │
│  ┌──────────────────┐     ┌──────────────────┐              │
│  │ RocksRateLimiter │────>│  Own RocksDB     │ ← SEPARATE   │
│  │  (experimental)  │     │  ./rate-limit-db │   DATABASE   │
│  └──────────────────┘     └──────────────────┘              │
│                                                              │
│  Key: "tenant-123"  (simple string)                         │
│  Value: RateBucket { timestamps, last_cleanup }             │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                    raisin-rocksdb                            │
│  ┌──────────────────┐     ┌──────────────────┐              │
│  │  Main Storage    │────>│  Shared RocksDB  │ ← MAIN DB    │
│  │  (production)    │     │  Column Families │              │
│  └──────────────────┘     │  + Replication   │              │
│                           └──────────────────┘              │
│  Key: {tenant}\0{repo}\0{branch}\0...  (structured)         │
└─────────────────────────────────────────────────────────────┘
```

### Future Architecture (Integrated)

```
┌─────────────────────────────────────────────────────────────┐
│                    raisin-rocksdb                            │
│                                                              │
│  Column Families:                                           │
│  ├── CF_NODES                                               │
│  ├── CF_PATH_INDEX                                          │
│  ├── CF_PROPERTY_INDEX                                      │
│  ├── ...                                                    │
│  └── CF_RATE_LIMIT  ← NEW                                   │
│                                                              │
│  Rate limit keys: {tenant}\0rate_limit\0{key}               │
│  Replication: TBD (rate limits may stay node-local)         │
└─────────────────────────────────────────────────────────────┘
```

### Current Status

The `RateLimiter` trait in `raisin-context` is used by many crates (raisin-core, raisin-server, raisin-storage, etc.), but they currently use `NoOpRateLimiter` which allows all requests.

This crate provides a working `RocksRateLimiter` implementation as a prototype. Future work:

1. **Integrate with raisin-rocksdb** - Share the main DB instance
2. **Add column family** - Isolate rate limit data in `CF_RATE_LIMIT`
3. **Structured keys** - Use `KeyBuilder` for consistent key encoding
4. **Evaluate replication** - Rate limits are often node-local by design

### Use Cases (when integrated)

- **HTTP API Layer**: Rate limit API requests per tenant/user
- **Function Execution**: Throttle serverless function invocations
- **Event Processing**: Control event emission rates

## Dependencies

```toml
[dependencies]
raisin-context = { path = "../raisin-context" }
rocksdb = { version = "0.24.0", optional = true }
tokio = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
rmp-serde = { workspace = true }
```

## Performance

- **Storage**: O(n) where n = requests in current window per key
- **Check**: O(n) scan of timestamps (typically small)
- **Persistence**: Survives restarts, shared across processes

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
