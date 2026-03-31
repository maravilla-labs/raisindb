# Single-Tenant Application Example

Complete example of a simple single-tenant application with proper setup.

## Overview

This example shows the complete flow for a single-tenant application:
1. Create NodeTypes (if using custom schemas)
2. Create Workspaces
3. Create Nodes

## Complete Code

```rust
use raisin_core::{RaisinConnection, WorkspaceService};
use raisin_rocksdb::RocksDBStorage;
use raisin_models::{nodes::Node, workspace::Workspace};
use raisin_error::Result;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize storage and connection
    let storage = Arc::new(RocksDBStorage::new("./data")?);
    let conn = RaisinConnection::with_storage(storage.clone());

    // Step 1: Create Workspace
    // Using built-in types (raisin:Folder, raisin:Page) so we skip NodeType creation
    let workspace_service = WorkspaceService::new(storage.clone());
    let workspace = Workspace {
        name: "content".to_string(),
        description: Some("Website content".to_string()),
        allowed_node_types: vec![
            "raisin:Folder".to_string(),
            "raisin:Page".to_string(),
        ],
        allowed_root_node_types: vec![
            "raisin:Folder".to_string(),  // Only folders at root
        ],
        ..Default::default()
    };
    workspace_service.put("default", "default", workspace).await?;
    println!("Created workspace: content");

    // Get node service scoped to tenant/repo/workspace
    let node_service = conn.tenant("default")
        .repository("default")
        .workspace("content")
        .nodes();

    // Step 2: Create folder structure
    let documents = Node {
        name: "documents".to_string(),
        node_type: "raisin:Folder".to_string(),
        properties: Default::default(),
        ..Default::default()
    };

    let created_folder = node_service
        .add_node("/", documents)
        .await?;

    println!("Created folder: {}", created_folder.path);

    // Step 3: Create a page inside the folder
    let page = Node {
        name: "readme".to_string(),
        node_type: "raisin:Page".to_string(),
        properties: {
            let mut props = std::collections::HashMap::new();
            props.insert("title".to_string(), "README".into());
            props.insert("content".to_string(), "<h1>Welcome</h1>".into());
            props
        },
        ..Default::default()
    };

    let created_page = node_service
        .add_node(&created_folder.path, page)
        .await?;

    println!("Created page: {}", created_page.path);

    // Query all nodes
    let all_nodes = node_service.list_all().await?;
    println!("\nAll nodes:");
    for node in all_nodes {
        println!("  - {} ({})", node.path, node.node_type);
    }

    // Get by path
    let retrieved = node_service
        .get_by_path("/documents/readme")
        .await?;

    if let Some(node) = retrieved {
        println!("\nRetrieved by path: {}", node.name);
    }

    Ok(())
}
```

## With Custom NodeTypes

If you need custom schemas with validation:

```rust
use raisin_core::{RaisinConnection, WorkspaceService};
use raisin_rocksdb::RocksDBStorage;
use raisin_models::{
    nodes::Node,
    nodes::types::NodeType,
    nodes::properties::schema::{PropertyValueSchema, PropertyType},
    workspace::Workspace,
};
use raisin_error::Result;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let storage = Arc::new(RocksDBStorage::new("./data")?);
    let conn = RaisinConnection::with_storage(storage.clone());

    // Step 1: Create custom NodeType
    let article_type = NodeType {
        name: "Article".to_string(),
        description: Some("A blog article".to_string()),
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
                required: Some(true),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("author".to_string()),
                property_type: PropertyType::String,
                required: Some(true),
                ..Default::default()
            },
            PropertyValueSchema {
                name: Some("published_date".to_string()),
                property_type: PropertyType::Date,
                ..Default::default()
            },
        ]),
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(true),
        ..Default::default()
    };
    // Store NodeType via the storage layer
    storage.node_types().put(
        raisin_storage::scope::BranchScope::new("default", "default", "main"),
        article_type,
        None,
    ).await?;
    println!("Created NodeType: Article");

    // Step 2: Create Workspace
    let workspace_service = WorkspaceService::new(storage.clone());
    let workspace = Workspace {
        name: "blog".to_string(),
        description: Some("Blog content".to_string()),
        allowed_node_types: vec![
            "raisin:Folder".to_string(),
            "myapp:Article".to_string(),
        ],
        allowed_root_node_types: vec![
            "raisin:Folder".to_string(),
        ],
        ..Default::default()
    };
    workspace_service.put("default", "default", workspace).await?;
    println!("Created workspace: blog");

    // Step 3: Create structure via connection API
    let node_service = conn.tenant("default")
        .repository("default")
        .workspace("blog")
        .nodes();

    let posts_folder = Node {
        name: "posts".to_string(),
        node_type: "raisin:Folder".to_string(),
        ..Default::default()
    };
    let folder = node_service
        .add_node("/", posts_folder)
        .await?;
    println!("Created folder: {}", folder.path);

    // Step 4: Create article
    let article = Node {
        name: "first-post".to_string(),
        node_type: "myapp:Article".to_string(),
        properties: {
            let mut props = std::collections::HashMap::new();
            props.insert("title".to_string(), "My First Post".into());
            props.insert("content".to_string(), "<p>Hello World!</p>".into());
            props.insert("author".to_string(), "John Doe".into());
            props
        },
        ..Default::default()
    };

    let created = node_service
        .add_node("/posts", article)
        .await?;
    println!("Created article: {}", created.path);

    // List all
    let all = node_service.list_all().await?;
    println!("\nAll blog content:");
    for node in all {
        println!("  - {} ({})", node.path, node.node_type);
    }

    Ok(())
}
```

## Key Points

- **Uses single storage instance** for all data
- **No tenant context needed** - single-tenant mode
- **All data stored in one location** - simple and straightforward
- **Perfect for single-organization apps** - internal tools, single customer
- **Must create workspaces first** before adding nodes
- **Can use global types** or custom NodeTypes

## When to Use

Use single-tenant mode when:

- **Internal tools** - Apps used within one organization
- **Single organization/customer** - Not a SaaS product
- **Prototyping** - Quick testing and development
- **Simple applications** - No need for multi-tenancy
- **Dedicated deployments** - Each customer gets their own instance

## Advantages

1. **Simplicity** - No tenant context to manage
2. **Performance** - No tenant isolation overhead
3. **Easy debugging** - All data in one place
4. **Straightforward** - Simple mental model

## Limitations

1. **No multi-tenancy** - Can't serve multiple customers from one instance
2. **Scaling** - Each customer requires separate deployment
3. **Resource usage** - Multiple instances for multiple customers

## Next Steps

- [Multi-Tenant SaaS Example](multi-tenant-saas.md) - Add multi-tenancy
- [Node System](../architecture/node-system.md) - Learn about NodeTypes
- [Workspace Configuration](../architecture/workspace-configuration.md) - Advanced workspace setup
