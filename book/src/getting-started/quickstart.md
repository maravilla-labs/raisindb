# Quick Start

Get up and running with RaisinDB in 5 minutes.

## Installation

### As a Library (Embedded)

Add RaisinDB to your `Cargo.toml`:

```toml
[dependencies]
raisin-core = { path = "path/to/raisin-core" }
raisin-rocksdb = { path = "path/to/raisin-rocksdb" }
raisin-models = { path = "path/to/raisin-models" }
raisin-context = { path = "path/to/raisin-context" }
tokio = { version = "1", features = ["full"] }
```

### As a Standalone Server

Clone and run the reference server:

```bash
git clone https://github.com/maravilla-labs/raisindb
cd raisindb
cargo run --bin raisin-server --features "storage-rocksdb,websocket,pgwire"
```

## Understanding the Setup Flow

RaisinDB requires a 3-step setup:

```
1. Create NodeTypes (define schemas)
    ↓
2. Create Workspaces (configure what's allowed)
    ↓
3. Create Nodes (actual data)
```

**Important**: You cannot create nodes until you've created NodeTypes and Workspaces.

**Validation**: NodeService automatically validates all nodes on `add_node()` and `put()` operations. You don't need to manually validate - the service checks NodeType existence, required properties, strict mode compliance, and unique constraints automatically.

## Your First Setup

### Step 0: Initialize Storage and Connection

```rust
use raisin_rocksdb::RocksDBStorage;
use raisin_core::RaisinConnection;
use std::sync::Arc;

let storage = Arc::new(RocksDBStorage::new("./data")?);
let conn = RaisinConnection::with_storage(storage.clone());
```

### Step 1: Create NodeTypes

Define schemas for your content:

```rust
use raisin_models::nodes::types::NodeType;
use raisin_models::nodes::properties::schema::{PropertyValueSchema, PropertyType};

// Create a Page type
let page_type = NodeType {
    name: "Page".to_string(),
    description: Some("A basic content page".to_string()),
    properties: Some(vec![
        PropertyValueSchema {
            name: Some("title".to_string()),
            property_type: PropertyType::String,
            required: Some(true),
            ..Default::default()
        },
        PropertyValueSchema {
            name: Some("content".to_string()),
            property_type: PropertyType::String,
            ..Default::default()
        },
    ]),
    versionable: Some(true),
    publishable: Some(true),
    ..Default::default()
};

// Store via the storage NodeType repository
storage.node_types().put(
    raisin_storage::scope::BranchScope::new("default", "default", "main"),
    page_type,
    None,
).await?;
```

**Note**: RaisinDB provides built-in types like `raisin:Folder`, `raisin:Page`, and `raisin:Asset` out of the box, so you can skip creating custom NodeTypes for simple use cases.

### Step 2: Create Workspaces

Configure which NodeTypes are allowed:

```rust
use raisin_core::WorkspaceService;
use raisin_models::workspace::Workspace;

let workspace_service = WorkspaceService::new(storage.clone());

let content_workspace = Workspace {
    name: "content".to_string(),
    description: Some("Website content".to_string()),
    allowed_node_types: vec![
        "raisin:Folder".to_string(),
        "raisin:Page".to_string(),
        "myapp:Page".to_string(),  // Custom type we just created
    ],
    allowed_root_node_types: vec![
        "raisin:Folder".to_string(),  // Only folders at root
    ],
    ..Default::default()
};

workspace_service.put("default", "default", content_workspace).await?;
```

### Step 3: Create Nodes

Now you can create actual content:

```rust
use raisin_models::nodes::Node;

// Get a node service scoped to the "content" workspace
let node_service = conn.tenant("default")
    .repository("default")
    .workspace("content")
    .nodes();

// Create a folder
let folder = Node {
    name: "pages".to_string(),
    node_type: "raisin:Folder".to_string(),
    ..Default::default()
};

let created_folder = node_service
    .add_node("/", folder)
    .await?;

println!("Created folder: {}", created_folder.path);

// Create a page inside the folder
let page = Node {
    name: "homepage".to_string(),
    node_type: "myapp:Page".to_string(),
    properties: {
        let mut props = std::collections::HashMap::new();
        props.insert("title".to_string(), "Welcome".into());
        props.insert("content".to_string(), "<p>Welcome to our site!</p>".into());
        props
    },
    ..Default::default()
};

let created_page = node_service
    .add_node("/pages", page)
    .await?;

println!("Created page: {}", created_page.path);
```

### Step 4: Query Nodes

```rust
// List all nodes in workspace
let all_nodes = node_service.list_all().await?;
println!("Total nodes: {}", all_nodes.len());

// Get by ID
let node = node_service.get(&created_page.id).await?;

// Get by path
let node = node_service
    .get_by_path("/pages/homepage")
    .await?;

// List children of a folder
let children = node_service
    .list_children("/pages")
    .await?;
```

## Complete Example

Here's everything together:

```rust
use raisin_core::{RaisinConnection, WorkspaceService};
use raisin_rocksdb::RocksDBStorage;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_error::Result;
use std::sync::Arc;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize storage and connection
    let storage = Arc::new(RocksDBStorage::new("./data")?);
    let conn = RaisinConnection::with_storage(storage.clone());

    // Step 1: Create Workspace
    // (Skipping custom NodeTypes - we'll use raisin:Folder and raisin:Page)
    let workspace_service = WorkspaceService::new(storage.clone());
    let workspace = Workspace {
        name: "content".to_string(),
        description: Some("My content workspace".to_string()),
        allowed_node_types: vec![
            "raisin:Folder".to_string(),
            "raisin:Page".to_string(),
        ],
        allowed_root_node_types: vec![
            "raisin:Folder".to_string(),
        ],
        ..Default::default()
    };
    workspace_service.put("default", "default", workspace).await?;
    println!("Created workspace");

    // Step 2: Create Nodes via the connection API
    let node_service = conn.tenant("default")
        .repository("default")
        .workspace("content")
        .nodes();

    let folder = Node {
        name: "pages".to_string(),
        node_type: "raisin:Folder".to_string(),
        ..Default::default()
    };
    let created_folder = node_service
        .add_node("/", folder)
        .await?;
    println!("Created folder: {}", created_folder.path);

    let page = Node {
        name: "homepage".to_string(),
        node_type: "raisin:Page".to_string(),
        properties: {
            let mut props = HashMap::new();
            props.insert("title".to_string(), "Home".into());
            props
        },
        ..Default::default()
    };
    let created_page = node_service
        .add_node("/pages", page)
        .await?;
    println!("Created page: {}", created_page.path);

    // Query
    let all_nodes = node_service.list_all().await?;
    println!("Total nodes: {}", all_nodes.len());

    for node in all_nodes {
        println!("  - {} ({})", node.path, node.node_type);
    }

    Ok(())
}
```

## Quick Start with Global Types

If you don't need custom schemas, you can skip creating NodeTypes and just create a workspace:

```rust
use raisin_core::{RaisinConnection, WorkspaceService};
use raisin_rocksdb::RocksDBStorage;
use raisin_models::{nodes::Node, workspace::Workspace};
use raisin_error::Result;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let storage = Arc::new(RocksDBStorage::new("./data")?);
    let conn = RaisinConnection::with_storage(storage.clone());

    // 1. Create workspace with built-in types
    let workspace_service = WorkspaceService::new(storage.clone());
    let workspace = Workspace {
        name: "content".to_string(),
        allowed_node_types: vec![
            "raisin:Folder".to_string(),
            "raisin:Page".to_string(),
        ],
        allowed_root_node_types: vec!["raisin:Folder".to_string()],
        ..Default::default()
    };
    workspace_service.put("default", "default", workspace).await?;

    // 2. Create nodes using built-in types
    let node_service = conn.tenant("default")
        .repository("default")
        .workspace("content")
        .nodes();
    let node = Node {
        name: "my-page".to_string(),
        node_type: "raisin:Folder".to_string(),
        ..Default::default()
    };
    let created = node_service.add_node("/", node).await?;

    println!("Created: {}", created.path);
    Ok(())
}
```

## Using the HTTP Server

If you're running the standalone server, it comes with pre-configured workspaces:

```bash
# Create a node
curl -X POST http://localhost:8080/api/repository/default/ \
  -H "Content-Type: application/json" \
  -d '{
    "name": "my-page",
    "node_type": "raisin:Folder"
  }'

# List nodes
curl http://localhost:8080/api/repository/default/

# Get a specific node
curl http://localhost:8080/api/repository/default/$ref/NODE_ID
```

## Common Errors

### Error: "Workspace does not exist"

```rust
// ❌ Forgot to create workspace
let node = Node { /* ... */ };
node_service.add_node("/", node).await?;
// Error: Workspace 'content' does not exist

// ✅ Create workspace first
let workspace = Workspace {
    name: "content".to_string(),
    allowed_node_types: vec!["raisin:Folder".to_string()],
    allowed_root_node_types: vec!["raisin:Folder".to_string()],
    ..Default::default()
};
workspace_service.put("default", "default", workspace).await?;

// Now it works
node_service.add_node("/", node).await?;
```

### Error: "NodeType not allowed in workspace"

```rust
// Workspace only allows Folders
let workspace = Workspace {
    allowed_node_types: vec!["raisin:Folder".to_string()],
    // ...
};

// ❌ Trying to add a Page
let page = Node {
    node_type: "raisin:Page".to_string(),
    // ...
};
node_service.add_node("/", page).await?;
// Error: NodeType 'raisin:Page' not allowed

// ✅ Add Page to allowed types
workspace.allowed_node_types.push("raisin:Page".to_string());
workspace_service.put("default", "default", workspace).await?;
```

## Next Steps

- [**Node System**](../architecture/node-system.md) - Create custom NodeTypes with schemas
- [**Workspace Configuration**](../architecture/workspace-configuration.md) - Advanced workspace setup
- [**Embedded Usage Guide**](embedded.md) - Integrate into your Rust app
- [**Multi-Tenant Setup**](../guides/multi-tenant-saas.md) - Build a SaaS
- [**Storage Backends**](../architecture/storage-backends.md) - Choose your storage
