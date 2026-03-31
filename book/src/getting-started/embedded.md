# Embedded Usage

RaisinDB is designed to be embedded directly into your Rust application. This gives you full control over the database lifecycle and configuration.

## Adding to Your Project

Add RaisinDB to your `Cargo.toml`:

```toml
[dependencies]
raisin-core = { git = "https://github.com/yourusername/raisindb" }
raisin-rocksdb = { git = "https://github.com/yourusername/raisindb" }
raisin-models = { git = "https://github.com/yourusername/raisindb" }
tokio = { version = "1", features = ["full"] }
```

## Basic Setup

```rust
use raisin_core::RaisinConnection;
use raisin_rocksdb::RocksDBStorage;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize storage
    let storage = Arc::new(RocksDBStorage::new("./data")?);

    // Create connection (entry point for all operations)
    let conn = RaisinConnection::with_storage(storage);

    // Scope to tenant -> repository -> workspace -> nodes
    let nodes = conn.tenant("default")
        .repository("app")
        .workspace("content")
        .nodes();

    // You're ready to use RaisinDB!
    Ok(())
}
```

## Integration Patterns

See the [Embedding Guide](../guides/embedding-guide.md) for detailed integration patterns.
