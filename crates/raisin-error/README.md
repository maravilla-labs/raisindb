# raisin-error

Common error types for RaisinDB.

## Overview

This crate provides the unified `Error` enum and `Result` type alias used throughout all RaisinDB crates. It serves as the foundation for error handling across the entire project.

## Usage

```rust
use raisin_error::{Error, Result};

fn get_node(id: &str) -> Result<Node> {
    if id.is_empty() {
        return Err(Error::Validation("ID cannot be empty".into()));
    }

    storage.get(id)?
        .ok_or_else(|| Error::NotFound(format!("Node {}", id)))
}
```

## Error Variants

| Variant | Description | HTTP Status |
|---------|-------------|-------------|
| `NotFound(String)` | Resource not found | 404 |
| `AlreadyExists(String)` | Resource already exists | 409 |
| `Validation(String)` | Input validation failed | 400 |
| `Conflict(String)` | Concurrent modification conflict | 409 |
| `Backend(String)` | Storage/backend error | 500 |
| `Unauthorized(String)` | Authentication required | 401 |
| `Forbidden(String)` | Access denied | 403 |
| `PermissionDenied(String)` | Insufficient permissions | 403 |
| `Lock(String)` | Mutex/lock acquisition failed | 500 |
| `Encoding(String)` | Serialization/encoding error | 500 |
| `InvalidState(String)` | Unexpected state condition | 500 |
| `Internal(String)` | Internal invariant violation | 500 |
| `Other(anyhow::Error)` | Wrapped external error | 500 |

## Helper Methods

```rust
// Storage errors
Error::storage("RocksDB write failed")

// Lock errors
Error::lock("Failed to acquire mutex")

// Encoding errors
Error::encoding("Invalid UTF-8 sequence")

// Invalid state errors
Error::invalid_state("Transaction already committed")

// Internal errors
Error::internal("Unexpected None value")
```

## Result Type

The crate exports a convenient `Result` type alias:

```rust
pub type Result<T, E = Error> = std::result::Result<T, E>;
```

## Crate Usage

Used by 25+ crates including:
- `raisin-core`
- `raisin-storage`
- `raisin-rocksdb`
- `raisin-server`
- `raisin-transport-http`
- `raisin-sql-execution`
- `raisin-embeddings`
- `raisin-auth`
- And more...

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
