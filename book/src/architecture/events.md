# Event-Driven Architecture

RaisinDB uses an event-driven architecture to enable observable, decoupled, and extensible systems. Events are published when key operations occur (repository creation, node updates, commits, etc.), allowing handlers to react asynchronously.

## Overview

The event system consists of three main components:

1. **Events** - Structured data describing what happened
2. **EventBus** - Pub/sub system for routing events to handlers
3. **EventHandlers** - Subscribers that react to events

```
Operation → Event → EventBus → Handlers (async)
  ↓
Storage
```

## Event Types

Events are organized into three architectural levels matching the repository-first architecture:

### Repository Events

Repository-level operations and git-like workflows:

```rust
pub enum RepositoryEventKind {
    TenantCreated,    // Tenant registered for the first time
    Created,          // New repository created
    Updated,          // Repository config updated
    Deleted,          // Repository deleted
    CommitCreated,    // Transaction committed (creates revision)
    BranchCreated,    // New branch created
    BranchUpdated,    // Branch HEAD updated
    BranchDeleted,    // Branch deleted
    TagCreated,       // Tag created (immutable label)
    TagDeleted,       // Tag deleted
}
```

**RepositoryEvent Structure:**
```rust
pub struct RepositoryEvent {
    pub tenant_id: String,
    pub repository_id: String,
    pub kind: RepositoryEventKind,
    pub workspace: Option<String>,      // For commits
    pub revision_id: Option<String>,    // For commits
    pub branch_name: Option<String>,    // For branch operations
    pub tag_name: Option<String>,       // For tag operations
    pub message: Option<String>,        // Commit message
    pub actor: Option<String>,          // Who performed the operation
    pub metadata: Option<HashMap<String, JsonValue>>,
}
```

### Workspace Events

Workspace lifecycle events:

```rust
pub enum WorkspaceEventKind {
    Created,    // New workspace created
    Updated,    // Workspace config updated
    Deleted,    // Workspace deleted
}
```

### Node Events

Node-level CRUD and publishing operations:

```rust
pub enum NodeEventKind {
    Created,                                // Node created
    Updated,                                // Node updated
    Deleted,                                // Node deleted
    Reordered,                              // Node reordered among siblings
    Published,                              // Node published
    Unpublished,                            // Node unpublished
    PropertyChanged { property: String },   // Single property changed
    RelationAdded { relation_type: String, target_node_id: String },   // Relationship added
    RelationRemoved { relation_type: String, target_node_id: String }, // Relationship removed
}
```

**NodeEvent Structure:**
```rust
pub struct NodeEvent {
    pub tenant_id: String,
    pub repository_id: String,
    pub branch: String,
    pub workspace_id: String,
    pub node_id: String,
    pub node_type: Option<String>,
    pub revision: HLC,
    pub kind: NodeEventKind,
    pub path: Option<String>,
    pub metadata: Option<HashMap<String, JsonValue>>,
}
```

## EventBus API

### Publishing Events

Events are published from storage operations:

```rust
// Emit a RepositoryCreated event
let event = Event::Repository(RepositoryEvent {
    tenant_id: "acme".to_string(),
    repository_id: "website".to_string(),
    kind: RepositoryEventKind::Created,
    branch_name: Some("main".to_string()),
    workspace: None,
    revision_id: None,
    tag_name: None,
    message: None,
    actor: None,
    metadata: None,
});

storage.event_bus().publish(event);
```

Publishing is **fire-and-forget** - it returns immediately and handlers run asynchronously in background tasks.

### Subscribing Handlers

There are two ways to subscribe to events:

#### 1. Trait-Based Handlers (Recommended for complex logic)

```rust
use raisin_storage::{Event, EventHandler};
use std::pin::Pin;
use std::future::Future;

pub struct MyHandler {
    storage: Arc<dyn Storage>,
}

impl EventHandler for MyHandler {
    fn handle<'a>(&'a self, event: &'a Event) 
        -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> 
    {
        Box::pin(async move {
            match event {
                Event::Repository(repo_event) => {
                    if repo_event.kind == RepositoryEventKind::Created {
                        println!("New repository: {}", repo_event.repository_id);
                        // Do initialization work...
                    }
                }
                Event::Node(node_event) => {
                    // Handle node events...
                }
                _ => {}
            }
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "MyHandler"
    }
}

// Subscribe
let handler = Arc::new(MyHandler::new(storage.clone()));
storage.event_bus().subscribe(handler);
```

#### 2. Closure-Based Handlers with EventFilter (Simple cases)

Closure-based handlers support event filtering to receive only specific event types:

```rust
use raisin_storage::{EventBusExt, Event, EventFilter, RepositoryEventKind};

// Subscribe to specific event type
storage.event_bus().subscribe_fn(
    "repo_logger",
    EventFilter::Repository(RepositoryEventKind::Created),
    |event| {
        Box::pin(async move {
            if let Event::Repository(e) = event {
                println!("New repository: {}", e.repository_id);
            }
            Ok(())
        })
    }
);

// Subscribe to all repository events
storage.event_bus().subscribe_fn(
    "audit",
    EventFilter::AllRepository,
    |event| {
        Box::pin(async move {
            if let Event::Repository(e) = event {
                println!("Repository event: {:?}", e.kind);
            }
            Ok(())
        })
    }
);

// Subscribe to all events
storage.event_bus().subscribe_fn(
    "metrics",
    EventFilter::All,
    |event| {
        Box::pin(async move {
            println!("Event: {:?}", event);
            Ok(())
        })
    }
);

// Subscribe to specific node event
storage.event_bus().subscribe_fn(
    "publish_notifier",
    EventFilter::Node(NodeEventKind::Published),
    |event| {
        Box::pin(async move {
            if let Event::Node(e) = event {
                println!("Node published: {}", e.path.unwrap_or_default());
            }
            Ok(())
        })
    }
);
```

**Available EventFilters:**

- `EventFilter::All` - Match all events
- `EventFilter::AllRepository` - Match all repository events
- `EventFilter::Repository(kind)` - Match specific repository event kind
- `EventFilter::AllWorkspace` - Match all workspace events
- `EventFilter::Workspace(kind)` - Match specific workspace event kind
- `EventFilter::AllNode` - Match all node events
- `EventFilter::Node(kind)` - Match specific node event kind

### Handler Lifecycle

1. Handler is subscribed to EventBus
2. When an event is published:
   - EventBus spawns a tokio task for each handler
   - Handlers run concurrently (independent)
   - Failed handlers are logged but don't affect other handlers
3. Handlers remain subscribed until `clear_subscribers()` is called

## Common Patterns

### Pattern 1: Repository Initialization

Automatically initialize resources when a repository is created:

```rust
pub struct NodeTypeInitHandler {
    storage: Arc<dyn Storage>,
}

impl EventHandler for NodeTypeInitHandler {
    fn handle<'a>(&'a self, event: &'a Event) 
        -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> 
    {
        Box::pin(async move {
            if let Event::Repository(repo_event) = event {
                if repo_event.kind == RepositoryEventKind::Created {
                    // Initialize built-in NodeTypes
                    init_nodetypes_for_repository(
                        &repo_event.tenant_id,
                        &repo_event.repository_id,
                        repo_event.branch_name.as_deref().unwrap_or("main"),
                        self.storage.clone()
                    ).await?;
                }
            }
            Ok(())
        })
    }

    fn name(&self) -> &str { "NodeTypeInitHandler" }
}
```

### Pattern 2: Property Indexing

Maintain indexes for efficient queries:

```rust
impl EventHandler for PropertyIndexPlugin {
    fn handle<'a>(&'a self, event: &'a Event) 
        -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> 
    {
        Box::pin(async move {
            if let Event::Node(node_event) = event {
                match node_event.kind {
                    NodeEventKind::Created | NodeEventKind::Updated => {
                        // Extract properties from metadata
                        if let Some(metadata) = &node_event.metadata {
                            if let Some(props) = metadata.get("properties") {
                                self.index_properties(
                                    &node_event.node_id,
                                    props
                                ).await?;
                            }
                        }
                    }
                    NodeEventKind::Deleted => {
                        self.remove_from_index(&node_event.node_id).await?;
                    }
                    _ => {}
                }
            }
            Ok(())
        })
    }

    fn name(&self) -> &str { "PropertyIndexPlugin" }
}
```

### Pattern 3: Audit Logging

Log all operations for compliance:

```rust
storage.event_bus().subscribe_fn(
    "audit",
    EventFilter::All,
    |event| {
        Box::pin(async move {
            let log_entry = match event {
                Event::Repository(e) => format!(
                    "Repository {} {:?} by {:?}",
                    e.repository_id, e.kind, e.actor
                ),
                Event::Node(e) => format!(
                    "Node {} {:?} in {}/{}",
                    e.node_id, e.kind, e.repository_id, e.branch
                ),
                Event::Workspace(e) => format!(
                    "Workspace {} {:?}",
                    e.workspace, e.kind
                ),
            };

            // Write to audit log
            println!("[AUDIT] {}", log_entry);
            Ok(())
        })
    }
);
```

### Pattern 4: Cache Invalidation

Invalidate caches when data changes:

```rust
storage.event_bus().subscribe_fn(
    "cache-invalidator",
    EventFilter::All,
    |event| {
        Box::pin(async move {
            match event {
                Event::Node(e) if matches!(e.kind, NodeEventKind::Updated | NodeEventKind::Deleted) => {
                    cache.invalidate(&e.node_id).await?;
                }
                Event::Repository(e) if e.kind == RepositoryEventKind::BranchUpdated => {
                    cache.invalidate_branch(&e.repository_id, e.branch_name.as_deref().unwrap()).await?;
                }
                _ => {}
            }
            Ok(())
        })
    }
);
```

## Built-In Handlers

### NodeTypeInitHandler

**Purpose:** Automatically initializes built-in NodeTypes when repositories are created

**Location:** `raisin-server/src/nodetype_init_handler.rs`

**Listens for:** `RepositoryCreated`

**Actions:**
- Loads embedded YAML definitions (raisin:Folder, raisin:Page, raisin:Asset)
- Creates NodeTypes in the new repository
- Idempotent (checks versions, updates if newer)

**Subscription:**
```rust
let handler = Arc::new(NodeTypeInitHandler::new(storage.clone()));
storage.event_bus().subscribe(handler);
```

### PropertyIndexPlugin

**Purpose:** Maintains property indexes for efficient unique validation

**Location:** `raisin-indexer/src/property_index.rs`

**Listens for:** `NodeCreated`, `NodeUpdated`, `NodeDeleted`, `PropertyChanged`

**Actions:**
- Indexes node properties for O(1) lookup
- Removes indexes when nodes are deleted
- Updates indexes when properties change

## Event Flow Examples

### Example 1: Repository Creation

```
User: POST /api/repositories { repo_id: "website" }
  ↓
Handler: create_repository()
  ↓
Storage: InMemoryRepositoryManagement::create_repository()
  ├─ Create RepositoryInfo
  ├─ Store in repositories map
  └─ Publish Event::Repository(RepositoryCreated)
        ↓
EventBus: Dispatch to all subscribers
  ├─ NodeTypeInitHandler → Creates raisin:Folder, raisin:Page, raisin:Asset
  ├─ AuditLogger → Logs "Repository website created"
  └─ MetricsCollector → Increments repository count
```

### Example 2: Node Update with Property Change

```
User: PUT /api/repository/website/main/draft/content/page1
  ↓
NodeService: update_node()
  ↓
Storage: put_node()
  ├─ Update node in storage
  └─ Publish Event::Node(NodeUpdated)
        ↓
EventBus: Dispatch to all subscribers
  ├─ PropertyIndexPlugin → Re-index node properties
  ├─ CacheInvalidator → Clear cache for node
  ├─ SearchIndexer → Update search index
  └─ SSEBroadcaster → Stream to connected clients
```

### Example 3: Transaction Commit

```
User: POST /api/repository/website/main/draft/content/raisin:cmd/commit
  ↓
TransactionService: commit()
  ↓
Storage: commit_transaction()
  ├─ Create immutable revision snapshot
  ├─ Update branch HEAD
  └─ Publish Event::Repository(CommitCreated)
        ↓
EventBus: Dispatch to all subscribers
  ├─ AuditLogger → Log commit with message
  ├─ WebhookNotifier → Notify CI/CD pipeline
  └─ BackupScheduler → Trigger backup if threshold met
```

## Event Emission Status

The following events are currently emitted by RaisinDB operations:

### Repository Events ✅

| Event | Emitted From | Status |
|-------|-------------|--------|
| `RepositoryCreated` | `RepositoryManagement::create_repository()` | ✅ Implemented |
| `RepositoryUpdated` | `RepositoryManagement::update_repository_config()` | ✅ Implemented |
| `RepositoryDeleted` | `RepositoryManagement::delete_repository()` | ✅ Implemented |
| `BranchCreated` | `BranchRepository::create_branch()` | ✅ Implemented |
| `BranchUpdated` | `BranchRepository::update_head()` | ✅ Implemented |
| `BranchDeleted` | `BranchRepository::delete_branch()` | ✅ Implemented |
| `TagCreated` | `TagRepository::create_tag()` | ✅ Implemented |
| `TagDeleted` | `TagRepository::delete_tag()` | ✅ Implemented |
| `CommitCreated` | `TransactionService::commit()` | 📋 Planned |

### Node Events ✅

| Event | Emitted From | Status |
|-------|-------------|--------|
| `NodeCreated` | `NodeService::put()` (new node) | ✅ Implemented |
| `NodeUpdated` | `NodeService::put()` (existing node) | ✅ Implemented |
| `NodeDeleted` | `NodeService::delete()` | ✅ Implemented |
| `NodePublished` | `NodeService::publish()` | ✅ Implemented |
| `NodeUnpublished` | `NodeService::unpublish()` | ✅ Implemented |

### Workspace Events

| Event | Emitted From | Status |
|-------|-------------|--------|
| `WorkspaceCreated` | `InMemoryWorkspaceRepo::put()` (when new) | ✅ Implemented |
| `WorkspaceUpdated` | `InMemoryWorkspaceRepo::put()` (when exists) | ✅ Implemented |
| `WorkspaceDeleted` | N/A - No delete method | 📋 Planned |

### Example: Repository Lifecycle

```rust
// Creating a repository emits RepositoryCreated event
let repo = storage.repository_management()
    .create_repository("acme", "website", config)
    .await?;
// → Event::Repository(RepositoryCreated { tenant_id: "acme", repository_id: "website", ... })

// Updating config emits RepositoryUpdated event
storage.repository_management()
    .update_repository_config("acme", "website", new_config)
    .await?;
// → Event::Repository(RepositoryUpdated { ... })

// Deleting emits RepositoryDeleted event
storage.repository_management()
    .delete_repository("acme", "website")
    .await?;
// → Event::Repository(RepositoryDeleted { ... })
```

### Example: Workspace Lifecycle

```rust
use raisin_models::workspace::Workspace;

// Creating a workspace emits WorkspaceCreated event
let workspace = Workspace {
    name: "draft".to_string(),
    description: Some("Draft workspace for content".to_string()),
    ..Default::default()
};
storage.workspaces().put(workspace.clone()).await?;
// → Event::Workspace(WorkspaceCreated { workspace: "draft", ... })

// Updating the workspace emits WorkspaceUpdated event
workspace.description = Some("Updated description".to_string());
storage.workspaces().put(workspace).await?;
// → Event::Workspace(WorkspaceUpdated { workspace: "draft", ... })
```

### Example: Node Lifecycle

```rust
// Creating a node emits NodeCreated event
let node = Node {
    id: "page1".to_string(),
    node_type: "raisin:Page".to_string(),
    path: "/home".to_string(),
    // ... other fields
};
node_service.put(node.clone()).await?;
// → Event::Node(NodeCreated { node_id: "page1", kind: Created, ... })

// Updating the same node emits NodeUpdated event
node.properties.insert("title".to_string(), json!("New Title"));
node_service.put(node.clone()).await?;
// → Event::Node(NodeUpdated { node_id: "page1", kind: Updated, ... })

// Publishing emits NodePublished event
node_service.publish("/home").await?;
// → Event::Node(NodePublished { node_id: "page1", kind: Published, ... })

// Unpublishing emits NodeUnpublished event
node_service.unpublish("/home").await?;
// → Event::Node(NodeUnpublished { node_id: "page1", kind: Unpublished, ... })

// Deleting emits NodeDeleted event
node_service.delete("page1").await?;
// → Event::Node(NodeDeleted { node_id: "page1", kind: Deleted, ... })
```

### Example: Branch and Tag Operations

```rust
// Creating a branch emits BranchCreated event
let branch = storage.branches()
    .create_branch("acme", "website", "feature-x", "user1", None, false)
    .await?;
// → Event::Repository(BranchCreated { branch_name: "feature-x", ... })

// Updating branch HEAD emits BranchUpdated event
storage.branches()
    .update_head("acme", "website", "feature-x", 42)
    .await?;
// → Event::Repository(BranchUpdated { branch_name: "feature-x", revision_id: 42, ... })

// Creating a tag emits TagCreated event
let tag = storage.tags()
    .create_tag("acme", "website", "v1.0.0", 42, "user1", Some("Release 1.0".to_string()), false)
    .await?;
// → Event::Repository(TagCreated { tag_name: "v1.0.0", revision_id: 42, ... })
```

## Best Practices

### 1. Keep Handlers Fast and Focused

```rust
// ✅ Good - focused, async operations
storage.event_bus().subscribe_fn("quick-logger", EventFilter::All, |event| {
    Box::pin(async move {
        tokio::spawn(async move {
            write_to_log(event).await;
        });
        Ok(())
    })
});

// ❌ Bad - blocking operation
storage.event_bus().subscribe_fn("slow-logger", EventFilter::All, |event| {
    Box::pin(async move {
        std::thread::sleep(Duration::from_secs(5)); // Blocks!
        Ok(())
    })
});
```

### 2. Handle Errors Gracefully

```rust
impl EventHandler for MyHandler {
    fn handle<'a>(&'a self, event: &'a Event) -> ... {
        Box::pin(async move {
            match self.process(event).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    tracing::warn!("Handler failed: {}", e);
                    // Don't propagate - log and continue
                    Ok(())
                }
            }
        })
    }
}
```

### 3. Use Metadata for Extensibility

```rust
// Include relevant data in metadata for handlers
let event = Event::Node(NodeEvent {
    tenant_id: tenant_id.to_string(),
    repository_id: repo_id.to_string(),
    branch: branch.to_string(),
    node_id: node_id.to_string(),
    node_type: Some(node_type.to_string()),
    kind: NodeEventKind::Created,
    path: Some(path.to_string()),
    metadata: Some({
        let mut meta = HashMap::new();
        meta.insert("properties".to_string(), serde_json::to_value(properties)?);
        meta.insert("actor".to_string(), serde_json::json!(actor));
        meta.insert("timestamp".to_string(), serde_json::json!(Utc::now()));
        meta
    }),
});
```

### 4. Test Handlers in Isolation

```rust
#[tokio::test]
async fn test_nodetype_handler() {
    let storage = Arc::new(InMemoryStorage::default());
    let handler = NodeTypeInitHandler::new(storage.clone());

    let event = Event::Repository(RepositoryEvent {
        tenant_id: "test".to_string(),
        repository_id: "repo1".to_string(),
        kind: RepositoryEventKind::Created,
        branch_name: Some("main".to_string()),
        // ... other fields
    });

    handler.handle(&event).await.unwrap();

    // Verify NodeTypes were created
    let folder = storage.node_types()
        .get("test", "repo1", "main", "raisin:Folder")
        .await
        .unwrap();
    assert!(folder.is_some());
}
```

## Performance Considerations

### Event Dispatch Overhead

- **Fire-and-forget:** Publishing returns immediately
- **Async dispatch:** Each handler runs in separate tokio task
- **No blocking:** Failed handlers don't affect other handlers

### Scalability

- **Parallel execution:** Handlers run concurrently
- **Memory efficient:** Event cloning is cheap (Arc-based)
- **Bounded growth:** Use `clear_subscribers()` to clean up

### Monitoring

```rust
// Track event metrics
storage.event_bus().subscribe_fn("metrics", EventFilter::All, |event| {
    Box::pin(async move {
        match event {
            Event::Repository(_) => {
                REPOSITORY_EVENTS.inc();
            }
            Event::Node(_) => {
                NODE_EVENTS.inc();
            }
            Event::Workspace(_) => {
                WORKSPACE_EVENTS.inc();
            }
        }
        Ok(())
    })
});
```

## Comparison with Other Patterns

| Pattern | Use Case | Events | Direct Calls |
|---------|----------|--------|--------------|
| **Initialization** | Setup on create | ✅ Decoupled | ❌ Tightly coupled |
| **Indexing** | Background updates | ✅ Async | ❌ Blocks request |
| **Validation** | Pre-save checks | ❌ Too late | ✅ Immediate feedback |
| **Audit** | Compliance logging | ✅ Observable | ❌ Easy to forget |
| **Webhooks** | External notifications | ✅ Non-blocking | ❌ Unreliable |

## Next Steps

- See [Node Service Events](../guides/node-events.md) for emitting node events
- See [Repository Events](../guides/repository-events.md) for git-like operations
- See [Custom Handlers](../guides/custom-handlers.md) for building your own handlers

## Summary

The event-driven architecture in RaisinDB provides:

✅ **Decoupling** - Components don't need to know about each other
✅ **Observability** - All operations are visible through events  
✅ **Extensibility** - Add new handlers without modifying core code  
✅ **Scalability** - Async, non-blocking, parallel execution  
✅ **Testability** - Handlers can be tested in isolation
