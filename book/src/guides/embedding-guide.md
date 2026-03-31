# Embedding Guide

Learn how to embed RaisinDB into your Rust application.

## Basic Embedding

### 1. Add Dependencies

```toml
[dependencies]
raisin-core = { path = "path/to/raisin-core" }
raisin-rocksdb = { path = "path/to/raisin-rocksdb" }
raisin-models = { path = "path/to/raisin-models" }
raisin-error = { path = "path/to/raisin-error" }
tokio = { version = "1", features = ["full"] }
anyhow = "1.0"
```

### 2. Initialize Storage

```rust
use raisin_rocksdb::RocksDBStorage;
use std::sync::Arc;

// Open RocksDB at specified path (uses development config)
let storage = Arc::new(
    RocksDBStorage::new("./data/content")
        .expect("Failed to open storage")
);
```

### 3. Create a Connection

The recommended way to interact with RaisinDB is through the `RaisinConnection` API, which provides a fluent builder for scoping operations to a specific tenant, repository, branch, and workspace:

```rust
use raisin_core::RaisinConnection;

let connection = RaisinConnection::with_storage(storage.clone());

// Get a workspace-scoped node service
let node_service = connection
    .tenant("default")
    .repository("app")
    .workspace("content")
    .nodes();
```

### 4. Use in Your Application

```rust
use raisin_error::Result;
use raisin_models::nodes::Node;

// In your application logic
async fn create_content(
    connection: &RaisinConnection<RocksDBStorage>,
    parent: &str,
    name: &str,
) -> Result<String> {
    let service = connection
        .tenant("default")
        .repository("app")
        .workspace("content")
        .nodes();

    let node = Node {
        name: name.to_string(),
        node_type: "raisin:Folder".to_string(),
        ..Default::default()
    };

    let created = service.add_node(parent, node).await?;
    Ok(created.id)
}
```

## Single-Tenant Application

For a simple single-tenant application:

```rust
use raisin_core::RaisinConnection;
use raisin_rocksdb::RocksDBStorage;
use raisin_models::nodes::Node;
use raisin_error::Result;
use std::sync::Arc;

pub struct ContentManager {
    storage: Arc<RocksDBStorage>,
    workspace: String,
}

impl ContentManager {
    pub fn new(data_path: &str, workspace: String) -> Result<Self> {
        let storage = Arc::new(RocksDBStorage::new(data_path)?);

        Ok(Self {
            storage,
            workspace,
        })
    }

    fn connection(&self) -> RaisinConnection<RocksDBStorage> {
        RaisinConnection::with_storage(self.storage.clone())
    }

    pub async fn create_page(&self, name: &str) -> Result<String> {
        let service = self.connection()
            .tenant("default")
            .repository("app")
            .workspace(&self.workspace)
            .nodes();

        let node = Node {
            name: name.to_string(),
            node_type: "raisin:Page".to_string(),
            ..Default::default()
        };

        let created = service.add_node("/", node).await?;
        Ok(created.id)
    }

    pub async fn list_pages(&self) -> Result<Vec<Node>> {
        let service = self.connection()
            .tenant("default")
            .repository("app")
            .workspace(&self.workspace)
            .nodes();

        service.list_all().await
    }
}
```

## Multi-Tenant Application

For multi-tenant SaaS applications:

```rust
use raisin_core::RaisinConnection;
use raisin_rocksdb::RocksDBStorage;
use raisin_models::nodes::Node;
use raisin_error::Result;
use std::sync::Arc;

pub struct MultiTenantContentManager {
    storage: Arc<RocksDBStorage>,
}

impl MultiTenantContentManager {
    pub fn new(data_path: &str) -> Result<Self> {
        let storage = Arc::new(RocksDBStorage::new(data_path)?);
        Ok(Self { storage })
    }

    fn connection(&self) -> RaisinConnection<RocksDBStorage> {
        RaisinConnection::with_storage(self.storage.clone())
    }

    /// Create content for a specific tenant
    pub async fn create_page_for_tenant(
        &self,
        tenant_id: &str,
        name: &str,
    ) -> Result<String> {
        // Scope operations to tenant/repo/workspace via the connection API
        let service = self.connection()
            .tenant(tenant_id)
            .repository("production")
            .workspace("content")
            .nodes();

        let node = Node {
            name: name.to_string(),
            node_type: "raisin:Page".to_string(),
            ..Default::default()
        };

        let created = service.add_node("/", node).await?;
        Ok(created.id)
    }
}
```

## With HTTP Server (Axum)

Integrate with a web framework:

```rust
use axum::{
    extract::{State, Path},
    routing::{get, post},
    Json, Router,
};
use raisin_core::RaisinConnection;
use raisin_rocksdb::RocksDBStorage;
use raisin_models::nodes::Node;
use std::sync::Arc;

#[derive(Clone)]
struct AppState {
    storage: Arc<RocksDBStorage>,
}

impl AppState {
    fn connection(&self) -> RaisinConnection<RocksDBStorage> {
        RaisinConnection::with_storage(self.storage.clone())
    }
}

async fn create_node(
    State(state): State<AppState>,
    Json(node): Json<Node>,
) -> Json<Node> {
    let service = state.connection()
        .tenant("default")
        .repository("app")
        .workspace("content")
        .nodes();

    let created = service.add_node("/", node).await.unwrap();
    Json(created)
}

async fn list_nodes(
    State(state): State<AppState>,
) -> Json<Vec<Node>> {
    let service = state.connection()
        .tenant("default")
        .repository("app")
        .workspace("content")
        .nodes();

    let nodes = service.list_all().await.unwrap();
    Json(nodes)
}

#[tokio::main]
async fn main() {
    let storage = Arc::new(RocksDBStorage::new("./data").unwrap());
    let state = AppState { storage };

    let app = Router::new()
        .route("/nodes", post(create_node).get(list_nodes))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .unwrap();

    axum::serve(listener, app).await.unwrap();
}
```

## Best Practices

### 1. Use Arc for Sharing

Storage should be wrapped in `Arc` for cheap cloning across threads:

```rust
let storage = Arc::new(RocksDBStorage::new("./data")?);
// Create connections as needed - they are lightweight
let connection = RaisinConnection::with_storage(storage);
```

### 2. Error Handling

Use `anyhow` or `thiserror` for error handling:

```rust
use anyhow::Result;

async fn my_operation() -> Result<Node> {
    let node = service.get("workspace", "id").await?
        .ok_or_else(|| anyhow::anyhow!("Node not found"))?;
    Ok(node)
}
```

### 3. Workspace Strategy

Choose a workspace strategy:

- **Single workspace**: All content in "default"
- **Per-user workspaces**: Each user gets their own workspace
- **Per-project workspaces**: Workspace per project/organization

### 4. Graceful Shutdown

Ensure proper cleanup on shutdown:

```rust
impl Drop for ContentManager {
    fn drop(&mut self) {
        // Storage is automatically closed when Arc count reaches 0
        println!("Shutting down content manager");
    }
}
```

## Next Steps

- [Multi-Tenant SaaS Guide](multi-tenant-saas.md) - Build a SaaS with RaisinDB
- [Storage Backends](../architecture/storage-backends.md) - Choose your storage
- [Rate Limiting](../architecture/rate-limiting.md) - Add rate limits
- [API Reference](../api-reference/core-services.md) - Full API docs
