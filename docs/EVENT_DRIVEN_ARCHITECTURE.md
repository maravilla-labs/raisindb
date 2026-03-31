# Event-Driven Architecture Implementation

## Overview

RaisinDB now has a comprehensive event system integrated at the storage layer, enabling observable, event-driven operations across the entire platform.

## Architecture

### Three-Level Event Hierarchy

Events are organized by their architectural scope:

```
Event (enum)
├── Repository (RepositoryEvent)
│   ├── Created                 ← Repository lifecycle
│   ├── Updated
│   ├── Deleted
│   ├── CommitCreated          ← Git operations (repository-wide)
│   ├── BranchCreated
│   ├── BranchUpdated
│   ├── BranchDeleted
│   ├── TagCreated
│   └── TagDeleted
│
├── Workspace (WorkspaceEvent)
│   ├── Created                ← Workspace lifecycle
│   ├── Updated
│   └── Deleted
│
└── Node (NodeEvent)
    ├── Created                ← Individual node operations
    ├── Updated
    ├── Deleted
    ├── Published
    ├── Unpublished
    └── PropertyChanged
```

### Event Bus Integration

The event bus is integrated at the **storage layer**, making it accessible throughout the entire system:

```rust
pub trait Storage: Send + Sync {
    // ... other methods ...
    
    /// Get the event bus for subscribing to storage events
    fn event_bus(&self) -> Arc<dyn EventBus>;
}
```

Both `InMemoryStorage` and `RocksStorage` include an event bus instance that's created during initialization.

## Usage Patterns

### 1. Subscribing to Events

In `main.rs` (after storage initialization):

```rust
use raisin_storage::EventHandler;
use std::sync::Arc;

// Get the event bus from storage
let event_bus = storage.event_bus();

// Subscribe handlers
event_bus.subscribe(Arc::new(PropertyIndexPlugin::new()));
event_bus.subscribe(Arc::new(NodeTypeInitHandler::new(storage.clone())));
event_bus.subscribe(Arc::new(AuditLogger::new()));
```

### 2. Creating Event Handlers

Handlers implement the `EventHandler` trait:

```rust
use raisin_events::{Event, EventHandler, RepositoryEvent, RepositoryEventKind};
use std::future::Future;
use std::pin::Pin;

pub struct NodeTypeInitHandler {
    storage: Arc<dyn Storage>,
}

impl EventHandler for NodeTypeInitHandler {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Only handle repository creation events
            if let Event::Repository(repo_event) = event {
                if repo_event.kind == RepositoryEventKind::Created {
                    // Initialize NodeTypes for the new repository
                    self.init_nodetypes(&repo_event.tenant_id, &repo_event.repository_id).await?;
                }
            }
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "NodeTypeInitHandler"
    }
}
```

### 3. Emitting Events (Services)

Services emit events when performing operations:

```rust
// In NodeService::create_node()
let event = Event::Node(NodeEvent {
    workspace: workspace.to_string(),
    node_id: node.id.clone(),
    node_type: node.node_type.clone(),
    kind: NodeEventKind::Created,
    properties: Some(node.properties.clone()),
    old_value: None,
    new_value: None,
});

event_bus.publish(event);
```

```rust
// In RepositoryService::create_repository()
let event = Event::Repository(RepositoryEvent {
    tenant_id: tenant_id.to_string(),
    repository_id: repo_id.to_string(),
    kind: RepositoryEventKind::Created,
    workspace: None,
    revision_id: None,
    branch_name: Some("main".to_string()),
    tag_name: None,
    message: None,
    actor: None,
    metadata: None,
});

event_bus.publish(event);
```

## Event Flow

### Repository Creation Flow (Example)

```
1. User → POST /api/repositories
   ↓
2. HTTP Handler → RepositoryService::create_repository()
   ↓
3. RepositoryService → Creates repository in storage
   ↓
4. RepositoryService → Emits RepositoryEvent::Created
   ↓
5. Event Bus → Dispatches to all subscribers (async)
   ├── NodeTypeInitHandler → Creates raisin:Folder, raisin:Page, raisin:Asset
   ├── AuditLogger → Logs repository creation
   ├── NotificationService → Sends webhook
   └── SearchIndexer → Indexes new repository
```

### Node Update Flow (Example)

```
1. User → PUT /api/repository/{repo}/content/{path}
   ↓
2. HTTP Handler → NodeService::update_node()
   ↓
3. NodeService → Updates node in storage
   ↓
4. NodeService → Emits NodeEvent::Updated
   ↓
5. Event Bus → Dispatches to all subscribers
   ├── PropertyIndexPlugin → Updates property indexes
   ├── ReferenceIndexPlugin → Updates reference indexes
   ├── SearchIndexer → Re-indexes node
   └── CacheInvalidator → Clears cached data
```

## Benefits

### 1. **Decoupled Initialization**
- Repository creation no longer needs to know about NodeType initialization
- NodeTypes are created via event handler, not direct calls

### 2. **Observable Operations**
- All lifecycle events are visible to the system
- Easy to add new handlers without modifying core logic

### 3. **Real-Time Updates**
- SSE endpoints can stream events to connected clients
- Admin console can show live changes

### 4. **Extensible**
- Add new handlers without touching existing code
- Perfect for webhooks, notifications, search indexing, etc.

### 5. **Testable**
- Can inject mock event bus for testing
- Handlers can be tested independently

## Implementation Status

### ✅ Completed

- [x] Event type hierarchy (Repository, Workspace, Node)
- [x] EventBus trait with InMemoryEventBus implementation
- [x] EventHandler trait for async handlers
- [x] Storage trait integration
- [x] InMemoryStorage event bus support
- [x] RocksStorage event bus support
- [x] PropertyIndexPlugin using events
- [x] Event system tests passing

### 🔄 In Progress

- [ ] NodeTypeInitHandler implementation
- [ ] Service integration (emit events)
- [ ] SSE streaming of all events
- [ ] Admin console real-time updates

### ⏳ Planned

- [ ] Audit logging via events
- [ ] Search indexing via events
- [ ] Webhook support via events
- [ ] Notification service via events
- [ ] Cache invalidation via events

## File Locations

- Event types: `crates/raisin-events/src/lib.rs`
- Event bus implementation: `crates/raisin-events/src/bus.rs`
- Storage trait: `crates/raisin-storage/src/lib.rs`
- InMemory implementation: `crates/raisin-storage-memory/src/storage.rs`
- RocksDB implementation: `crates/raisin-storage-rocks/src/storage.rs`
- Example handler: `crates/raisin-indexer/src/property_index.rs`
- Server setup: `crates/raisin-server/src/main.rs`

## Next Steps

1. **Create NodeTypeInitHandler**
   - Listen for `RepositoryEvent::Created`
   - Initialize core NodeTypes (raisin:Folder, raisin:Page, raisin:Asset)

2. **Integrate Event Emission**
   - Add event publishing to NodeService
   - Add event publishing to RepositoryService
   - Add event publishing to TransactionService
   - Add event publishing to BranchService
   - Add event publishing to TagService

3. **Update SSE Endpoints**
   - Stream all event types (not just jobs)
   - Add filtering by event kind
   - Add tenant/repository scoping

4. **Refactor Startup**
   - Remove eager `init_global_nodetypes()` call
   - Subscribe NodeTypeInitHandler to event bus
   - Let repository creation trigger NodeType init

5. **Add More Handlers**
   - Audit logging
   - Search indexing
   - Cache invalidation
   - Webhook dispatch
   - Notifications
