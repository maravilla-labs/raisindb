# raisin-server

**Reference implementation of a RaisinDB server.**

This crate provides a complete, production-ready server binary that demonstrates how to build a RaisinDB server. It is designed as a **reference implementation** - RaisinDB's modular architecture allows for custom server implementations tailored to specific deployment needs.

## Key Concept: Modular Architecture

RaisinDB is architected with pluggable components:

```
┌─────────────────────────────────────────────────────────────────┐
│                       raisin-server                             │
│                  (Reference Implementation)                     │
├─────────────────────────────────────────────────────────────────┤
│  Transports (pluggable)     │  Storage (pluggable)              │
│  ├── HTTP REST API          │  ├── RocksDB (production)         │
│  ├── WebSocket              │  ├── In-Memory (testing)          │
│  └── PostgreSQL Wire        │  └── MongoDB (planned)            │
├─────────────────────────────────────────────────────────────────┤
│  Optional Features                                              │
│  ├── Replication (CRDT-based multi-node)                        │
│  ├── Full-text Search (Tantivy)                                 │
│  ├── Vector Search (HNSW)                                       │
│  ├── SQL/PGQ Graph Queries                                      │
│  ├── Starlark Functions                                         │
│  └── AI/ML (PDF processing, embeddings, OCR)                    │
└─────────────────────────────────────────────────────────────────┘
```

**Why is this a "reference implementation"?**

- **Transport layer is separate**: Use HTTP, WebSocket, pgwire, or build your own
- **Storage is pluggable**: RocksDB for production, in-memory for tests, or add custom backends
- **Features are optional**: Enable only what you need via Cargo features
- **Core logic is reusable**: All business logic lives in separate crates (`raisin-core`, `raisin-storage`, etc.)

You could build a custom server that:
- Uses only the pgwire transport for PostgreSQL compatibility
- Embeds RaisinDB in a larger application
- Adds custom authentication providers
- Integrates with existing infrastructure

## Quick Start

### Run with Default Features

```bash
cargo run -p raisin-server
```

### Run with Debug Logging

```bash
RUST_LOG=debug RUST_BACKTRACE=1 cargo run -p raisin-server
```

### Run with Specific Features

```bash
# Minimal: HTTP only, RocksDB storage
cargo run -p raisin-server --no-default-features --features storage-rocksdb

# Full: All transports, AI features
cargo run -p raisin-server --features "storage-rocksdb,websocket,pgwire,ai"
```

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `storage-rocksdb` | RocksDB storage backend with full indexing | ✅ |
| `store-memory` | In-memory storage (testing/development) | ❌ |
| `store-mongo` | MongoDB storage backend | ❌ |
| `websocket` | WebSocket transport for real-time events | ✅ |
| `pgwire` | PostgreSQL wire protocol (connect via psql) | ✅ |
| `ai` | AI/ML features (PDF, embeddings, OCR) | ✅ |
| `fs` | Local filesystem binary storage | ✅ |
| `s3` | AWS S3 binary storage | ❌ |

### AI Sub-Features

| Feature | Description |
|---------|-------------|
| `pdf` | PDF parsing support |
| `pdf-markdown` | PDF to Markdown conversion |
| `ocr` | Optical character recognition |
| `candle` | Local ML inference via Candle |
| `huggingface` | HuggingFace model integration |

## Configuration

The server is configured via command-line arguments or environment variables:

```bash
raisin-server [OPTIONS]

Options:
  --http-port <PORT>        HTTP server port [default: 3000]
  --ws-port <PORT>          WebSocket server port [default: 3001]
  --pgwire-port <PORT>      PostgreSQL wire protocol port [default: 5432]
  --data-dir <PATH>         Data directory [default: ./data]
  --config <PATH>           Configuration file path
  --cluster-id <ID>         Cluster node identifier
  --peers <URLS>            Comma-separated peer URLs for replication
```

### Configuration File

```toml
[server]
http_port = 3000
ws_port = 3001
pgwire_port = 5432

[storage]
data_dir = "./data"
backend = "rocksdb"

[replication]
enabled = true
node_id = "node-1"
peers = ["http://node-2:3000", "http://node-3:3000"]

[auth]
jwt_secret = "your-secret-key"
```

## Exposed Endpoints

### HTTP REST API (Port 3000)

- `GET /health` - Health check
- `POST /api/v1/repos` - Repository management
- `POST /api/v1/nodes` - Node CRUD operations
- `POST /api/v1/sql` - SQL query execution
- `POST /api/v1/graph` - Graph query execution
- See `raisin-transport-http` for full API documentation

### WebSocket (Port 3001)

- Real-time event streaming
- Live query subscriptions
- Replication event notifications

### PostgreSQL Wire Protocol (Port 5432)

```bash
psql -h localhost -p 5432 -U admin
```

Execute SQL and SQL/PGQ graph queries using standard PostgreSQL clients.

## Building Custom Servers

RaisinDB's modular design means you can create custom server implementations:

```rust
use raisin_core::RaisinCore;
use raisin_rocksdb::RocksDBStorage;
use raisin_transport_http::HttpTransport;

#[tokio::main]
async fn main() {
    // Initialize storage
    let storage = RocksDBStorage::new("./data").await?;

    // Initialize core with storage
    let core = RaisinCore::new(storage).await?;

    // Add only the transports you need
    let http = HttpTransport::new(core.clone());

    // Start your custom server
    axum::Server::bind(&"0.0.0.0:3000".parse()?)
        .serve(http.router().into_make_service())
        .await?;
}
```

## Testing

### Unit Tests

```bash
cargo test -p raisin-server
```

### Cluster Integration Tests

```bash
# Build server first
cargo build -p raisin-server --features storage-rocksdb

# Run 3-node cluster tests
cargo test -p raisin-server --test cluster_social_feed_test -- --ignored --nocapture
```

See `tests/CLUSTER_TESTS.md` for comprehensive cluster testing documentation.

## Dependencies

This server integrates the following RaisinDB crates:

| Crate | Purpose |
|-------|---------|
| `raisin-core` | Core business logic and node operations |
| `raisin-storage` | Storage abstraction layer |
| `raisin-rocksdb` | RocksDB storage implementation |
| `raisin-transport-http` | HTTP REST API handlers |
| `raisin-transport-ws` | WebSocket transport |
| `raisin-transport-pgwire` | PostgreSQL wire protocol |
| `raisin-replication` | CRDT-based multi-node replication |
| `raisin-sql-execution` | SQL query execution engine |
| `raisin-indexer` | Full-text and property indexing |
| `raisin-hnsw` | Vector similarity search |
| `raisin-embeddings` | Text embedding generation |
| `raisin-ai` | AI/ML processing features |
| `raisin-functions` | Starlark function runtime |

## License

Licensed under the Business Source License 1.1 (BSL-1.1).

See the [LICENSE](../../LICENSE) file in the repository root for details.
