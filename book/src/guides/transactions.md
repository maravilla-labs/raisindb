# Working with Transactions

Transactions enable **atomic multi-node operations** and create **immutable revisions** for version control. This guide covers practical transaction patterns.

## Quick Start

### Basic Transaction

```rust
use raisin_core::{RaisinConnection, Node};
use std::collections::HashMap;

// Create a connection and get a transaction
let conn = RaisinConnection::new(storage);
let mut tx = conn
    .tenant("acme-corp")
    .repository("website")
    .workspace("dev")
    .nodes()
    .branch("main")
    .transaction();

// Add operations (synchronous - queued in memory)
tx.create(Node::folder("blog", "/", HashMap::new()));
tx.create(Node::document("post1", "/blog", HashMap::from([
    ("title".to_string(), serde_json::json!("First Post")),
    ("content".to_string(), serde_json::json!("Hello world!")),
])));

// Commit atomically
let revision = tx.commit("Create blog structure", "alice").await?;
println!("Created revision {}", revision);
```

## Transaction Lifecycle

### 1. Start Transaction

Transactions are scoped to a specific **branch** and **workspace**:

```rust
let mut tx = workspace.nodes().branch("main").transaction();
```

At this point, the transaction is empty—no operations queued.

### 2. Queue Operations

Add operations without applying them:

```rust
// These are queued, not applied yet (methods are synchronous)
tx.create(node1);
tx.update("node-123".to_string(), serde_json::json!({"status": "published"}));
tx.delete("node-456".to_string());
tx.move_node("node-789".to_string(), "/new/parent".to_string());
tx.rename("node-abc".to_string(), "new-name".to_string());
tx.copy("/source/path".to_string(), "/target/parent".to_string(), None);
tx.copy_tree("/source/path".to_string(), "/target/parent".to_string(), None);
```

**Important**: Operations are stored in memory until commit. They don't affect the workspace yet.

### 3. Commit or Rollback

**Commit** applies all operations atomically:

```rust
let revision = tx.commit("Batch update", "system").await?;
// All operations applied, new revision created
```

**Rollback** discards all pending operations:

```rust
tx.rollback();
// All operations discarded, no changes made
```

Rollback is automatic if you drop the transaction without calling `commit()`.

## Operation Types

### Create Node

Add a new node to the workspace:

```rust
use raisin_core::Node;

tx.create(Node::document("readme", "/docs", HashMap::from([
    ("title".to_string(), serde_json::json!("README")),
    ("content".to_string(), serde_json::json!("# Welcome")),
])));
```

**Requirements**:
- Node ID must be unique
- Parent path must exist
- Node type must be registered

### Update Properties

Modify an existing node's properties:

```rust
tx.update("node-id-123".to_string(), serde_json::json!({
    "status": "published",
    "updated_at": chrono::Utc::now().to_rfc3339(),
}));
```

**Behavior**:
- Properties are **merged** with existing properties
- To remove a property, set it to `null`
- System fields (`created_at`, `created_by`) are preserved

### Delete Node

Remove a node and all its children:

```rust
tx.delete("node-id-456".to_string());
```

**Warning**: This recursively deletes all descendant nodes. Use with caution.

### Move Node

Change a node's parent:

```rust
tx.move_node("node-id-789".to_string(), "/new/parent/path".to_string());
```

**Effects**:
- Updates node's `path` field
- Preserves node properties and children
- Updates all descendant paths

## HTTP API Examples

### Create Multiple Nodes

```bash
POST /api/repository/myrepo/main/dev/raisin:cmd/commit
Content-Type: application/json

{
  "message": "Create project structure",
  "actor": "alice",
  "operations": [
    {
      "type": "create",
      "node": {
        "type": "folder",
        "name": "projects",
        "path": "/",
        "properties": {}
      }
    },
    {
      "type": "create",
      "node": {
        "type": "folder",
        "name": "assets",
        "path": "/projects",
        "properties": {}
      }
    },
    {
      "type": "create",
      "node": {
        "type": "document",
        "name": "index",
        "path": "/projects",
        "properties": {
          "title": "Project Index",
          "status": "draft"
        }
      }
    }
  ]
}
```

**Response:**

```json
{
  "revision": 42,
  "operations_count": 3
}
```

### Bulk Update

```bash
POST /api/repository/blog/main/prod/raisin:cmd/commit
Content-Type: application/json

{
  "message": "Publish all drafts",
  "actor": "publish-bot",
  "operations": [
    {
      "type": "update",
      "node_id": "post-123",
      "properties": { "status": "published" }
    },
    {
      "type": "update",
      "node_id": "post-456",
      "properties": { "status": "published" }
    },
    {
      "type": "update",
      "node_id": "post-789",
      "properties": { "status": "published" }
    }
  ]
}
```

### Move and Update

```bash
POST /api/repository/docs/main/work/raisin:cmd/commit
Content-Type: application/json

{
  "message": "Reorganize docs",
  "actor": "bob",
  "operations": [
    {
      "type": "move",
      "node_id": "guide-123",
      "new_parent_path": "/advanced"
    },
    {
      "type": "update",
      "node_id": "guide-123",
      "properties": {
        "category": "advanced",
        "difficulty": "expert"
      }
    }
  ]
}
```

## Advanced Patterns

### Conditional Commits

Check workspace state before committing:

```rust
let mut tx = workspace.nodes().branch("main").transaction();

// Check if a node exists
let node = workspace.nodes().get("/critical/data").await?;
if node.properties.get("locked") == Some(&serde_json::json!(true)) {
    tx.rollback();
    return Err("Cannot modify locked content".into());
}

// Proceed with changes
tx.update(node.id, serde_json::to_value(new_props)?);
tx.commit("Update unlocked content", "system").await?;
```

### Batch Processing

Process large datasets in chunks:

```rust
let items = fetch_items_from_api().await?;

for chunk in items.chunks(100) {
    let mut tx = workspace.nodes().branch("main").transaction();
    
    for item in chunk {
        tx.create(Node::document(
            &item.id,
            "/imports",
            item.to_properties(),
        ));
    }
    
    let revision = tx.commit(
        &format!("Import batch ({}–{})", chunk[0].id, chunk.last().unwrap().id),
        "import-script",
    ).await?;
    
    println!("Imported batch to revision {}", revision);
}
```

### Error Handling

```rust
let mut tx = workspace.nodes().branch("main").transaction();

// Add operations
tx.create(node1);
tx.create(node2);

// Commit with error handling
match tx.commit("Bulk create", "system").await {
    Ok(revision) => {
        println!("Success: Created revision {}", revision);
    }
    Err(e) => {
        eprintln!("Commit failed: {}", e);
        // Transaction automatically rolled back
        // Retry logic here...
    }
}
```

## Best Practices

### ✅ DO

- **Use transactions for related changes**: Group logically related operations
- **Provide meaningful commit messages**: Describe what and why
- **Keep transactions focused**: Avoid mixing unrelated operations
- **Handle errors gracefully**: Assume commits can fail

### ❌ DON'T

- **Don't commit empty transactions**: Include at least one operation
- **Don't hold transactions open**: Commit as soon as operations are queued
- **Don't nest transactions**: One transaction per logical unit of work
- **Don't commit without a message**: Always explain the change

## Comparison with Drafts

| Aspect | Draft Operations | Transaction Commits |
|--------|------------------|---------------------|
| **Speed** | Instant | Slightly slower (snapshot) |
| **Revision** | No revision created | Creates revision |
| **Rollback** | Manual (undo) | Automatic (revert HEAD) |
| **Atomicity** | Single operation | Multiple operations |
| **Use Case** | Real-time editing | Deployments, releases |

**Rule of Thumb**: Use drafts for collaboration, use commits for checkpoints.

## Next Steps

- **[Versioning Overview](../architecture/versioning.md)**: Git-like architecture
- **[Branches API](../api/branches.md)**: Branch management
- **[Rollback Guide](../guides/rollback.md)**: Restore previous revisions
