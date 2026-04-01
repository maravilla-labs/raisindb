# Standalone Server

Run RaisinDB as a standalone HTTP server.

## Overview

The `raisin-server` binary provides a complete HTTP REST API for RaisinDB. It's perfect for:

- **Development and Testing** - Quick local setup
- **Microservices** - Deploy as a standalone service
- **Prototyping** - Rapid application development
- **Production** - Deploy behind a reverse proxy with authentication

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/maravilla-labs/raisindb
cd raisindb

# Build the server
cargo build --release --bin raisin-server --features storage-rocksdb

# The binary will be in target/release/raisin-server
```

### Quick Start

Run with default settings:

```bash
cargo run --bin raisin-server --features storage-rocksdb
```

The server will start on `http://localhost:8080`

## Configuration

### Feature Flags

Configure storage backends at compile time:

```bash
# RocksDB storage (persistent, recommended for production)
cargo run --bin raisin-server --features storage-rocksdb

# In-memory storage (testing only, data lost on restart)
cargo run --bin raisin-server

# RocksDB + S3 for binary storage
cargo run --bin raisin-server --features "storage-rocksdb,s3"
```

### Storage Backends

#### RocksDB (Default with `storage-rocksdb`)

Persistent storage using RocksDB:

```bash
cargo run --bin raisin-server --features storage-rocksdb
```

Data stored in: `./.data/rocks`

#### In-Memory (Default without features)

Ephemeral storage for testing:

```bash
cargo run --bin raisin-server
```

**Warning**: All data is lost when the server stops.

### Binary Storage

#### Filesystem (Default)

Files stored in: `./.data/uploads`
Accessible at: `http://localhost:8080/files/*`

#### S3 (Optional)

Configure S3 via environment variables:

```bash
export AWS_ACCESS_KEY_ID=your_key
export AWS_SECRET_ACCESS_KEY=your_secret
export AWS_REGION=us-east-1
export S3_BUCKET=your-bucket-name

cargo run --bin raisin-server --features "store-rocks,s3"
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `RUST_LOG` | Logging level | `info` |
| `AWS_ACCESS_KEY_ID` | S3 access key | - |
| `AWS_SECRET_ACCESS_KEY` | S3 secret key | - |
| `AWS_REGION` | S3 region | - |
| `S3_BUCKET` | S3 bucket name | - |

### Logging

Control log verbosity:

```bash
# Info level (default)
RUST_LOG=info cargo run --bin raisin-server --features storage-rocksdb

# Debug level
RUST_LOG=debug cargo run --bin raisin-server --features storage-rocksdb

# Trace specific modules
RUST_LOG=raisin_transport_http=debug cargo run --bin raisin-server --features storage-rocksdb
```

## Quick Start Example

### 1. Start the Server

```bash
cargo run --bin raisin-server --features storage-rocksdb
```

Output:
```
2024-01-01T00:00:00Z  INFO raisin_server: listening on http://127.0.0.1:8080
```

### 2. Create a Workspace

```bash
curl -X PUT http://localhost:8080/workspaces/content \
  -H "Content-Type: application/json" \
  -d '{
    "name": "content",
    "description": "Website content",
    "allowed_node_types": [
      "raisin:Folder",
      "raisin:Page"
    ],
    "allowed_root_node_types": [
      "raisin:Folder"
    ]
  }'
```

### 3. Create a Node

```bash
curl -X POST http://localhost:8080/api/repository/content/ \
  -H "Content-Type: application/json" \
  -d '{
    "name": "homepage",
    "node_type": "raisin:Page",
    "properties": {
      "title": "Welcome to My Site"
    }
  }'
```

Response:
```json
{
  "id": "abc123",
  "name": "homepage",
  "path": "/homepage",
  "node_type": "raisin:Page",
  "properties": {
    "title": "Welcome to My Site"
  },
  "version": 1,
  "created_at": "2024-01-01T00:00:00Z"
}
```

### 4. Query the Node

```bash
curl http://localhost:8080/api/repository/content/homepage
```

Response:
```json
{
  "id": "abc123",
  "name": "homepage",
  "path": "/homepage",
  "node_type": "raisin:Page",
  "properties": {
    "title": "Welcome to My Site"
  }
}
```

### 5. List All Nodes

```bash
curl http://localhost:8080/api/repository/content/
```

Response:
```json
[
  {
    "id": "abc123",
    "name": "homepage",
    "path": "/homepage",
    "node_type": "raisin:Page",
    "properties": {
      "title": "Welcome to My Site"
    }
  }
]
```

## Production Deployment

### Running in Production

```bash
# Build release binary
cargo build --release --bin raisin-server --features storage-rocksdb

# Run the server
RUST_LOG=info ./target/release/raisin-server
```

### Systemd Service

Create `/etc/systemd/system/raisin-server.service`:

```ini
[Unit]
Description=RaisinDB Server
After=network.target

[Service]
Type=simple
User=raisindb
WorkingDirectory=/opt/raisindb
Environment="RUST_LOG=info"
ExecStart=/opt/raisindb/raisin-server
Restart=on-failure
RestartSec=5s

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl enable raisin-server
sudo systemctl start raisin-server
sudo systemctl status raisin-server
```

### Reverse Proxy Setup

#### Nginx

```nginx
server {
    listen 80;
    server_name api.example.com;

    location / {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # For file uploads
        client_max_body_size 100M;
    }
}
```

#### Caddy

```
api.example.com {
    reverse_proxy localhost:8080

    # For file uploads
    request_body {
        max_size 100MB
    }
}
```

### TLS Termination

Let the reverse proxy handle TLS:

```nginx
server {
    listen 443 ssl http2;
    server_name api.example.com;

    ssl_certificate /etc/letsencrypt/live/api.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;

    location / {
        proxy_pass http://localhost:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

## Adding Authentication

The default server **does not include authentication**. Add authentication middleware for production:

### JWT Authentication Example

```rust
use axum::{
    middleware::{self, Next},
    extract::{Request, TypedHeader},
    headers::{Authorization, authorization::Bearer},
    http::StatusCode,
    response::Response,
};
use jsonwebtoken::{decode, DecodingKey, Validation};

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

async fn auth_middleware(
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let token = auth.token();

    // Validate JWT
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(b"secret"),
        &Validation::default(),
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    // Add user_id to request extensions
    req.extensions_mut().insert(token_data.claims.sub);

    Ok(next.run(req).await)
}

// Add to router
let app = Router::new()
    .route("/api/*", /* handlers */)
    .layer(middleware::from_fn(auth_middleware));
```

### API Key Authentication Example

```rust
async fn api_key_middleware(
    TypedHeader(api_key): TypedHeader<TypedHeader<HeaderValue>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let key = api_key
        .get("X-API-Key")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // Validate API key against database
    if !is_valid_api_key(key).await {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}
```

## Multi-Tenancy

The default server is single-tenant. Add tenant resolution middleware for multi-tenancy:

### Subdomain-Based Tenancy

```rust
use axum::{
    middleware::{self, Next},
    extract::{Host, Request},
    http::StatusCode,
    response::Response,
};
use raisin_context::TenantContext;

async fn tenant_middleware(
    Host(host): Host,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Extract subdomain from host
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() < 2 {
        return Err(StatusCode::BAD_REQUEST);
    }

    let subdomain = parts[0];
    if subdomain == "www" || subdomain == "api" {
        return Err(StatusCode::BAD_REQUEST);
    }

    // Create tenant context
    let ctx = TenantContext::new(subdomain, "production");
    req.extensions_mut().insert(ctx);

    Ok(next.run(req).await)
}

// Create scoped services based on context
let app = Router::new()
    .route("/api/*", /* handlers */)
    .layer(middleware::from_fn(tenant_middleware));
```

See [Building a Multi-Tenant SaaS](../guides/multi-tenant-saas.md) for complete multi-tenancy setup.

## Health Checks

Add a health check endpoint:

```rust
async fn health_check() -> &'static str {
    "OK"
}

let app = Router::new()
    .route("/health", get(health_check))
    // ... other routes
```

Test it:

```bash
curl http://localhost:8080/health
```

## Monitoring

### Prometheus Metrics

Add metrics collection:

```rust
use axum_prometheus::PrometheusMetricLayer;

let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

let app = Router::new()
    .route("/api/*", /* handlers */)
    .route("/metrics", get(|| async move { metric_handle.render() }))
    .layer(prometheus_layer);
```

### Logging Best Practices

Use structured logging:

```rust
use tracing::{info, error, warn};

info!(
    node_id = %node.id,
    workspace = %workspace,
    "Node created successfully"
);

error!(
    error = %e,
    node_type = %node_type,
    "Failed to create node"
);
```

## Troubleshooting

### Server Won't Start

Check if port 8080 is already in use:

```bash
lsof -i :8080
```

Change the port in `main.rs`:

```rust
let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000)); // Use port 3000
```

### Database Errors

Check storage path permissions:

```bash
ls -la ./.data/rocks
```

Ensure the directory is writable by the server process.

### File Upload Fails

Check `client_max_body_size` in your reverse proxy configuration.

For Nginx:

```nginx
client_max_body_size 100M;
```

### High Memory Usage

RocksDB can use significant memory. Configure block cache size:

```rust
let mut opts = rocksdb::Options::default();
opts.set_block_cache_size(256 * 1024 * 1024); // 256MB
```

## API Documentation

See the complete [REST API Reference](../api/rest-api.md) for all available endpoints.

## Next Steps

- [REST API Reference](../api/rest-api.md) - Complete API documentation
- [Multi-Tenant SaaS Guide](../guides/multi-tenant-saas.md) - Add multi-tenancy
- [Embedded Usage](embedded.md) - Use NodeService directly in your Rust app
- [Custom Storage Backends](../guides/custom-storage.md) - Implement custom storage
