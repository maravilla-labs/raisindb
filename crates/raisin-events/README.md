# raisin-events

Event types and in-process event bus for RaisinDB.

## Overview

This crate provides the event system infrastructure for building observable, event-driven systems in RaisinDB. It includes typed event structures for all major operations and an in-memory event bus with backpressure support.

## Features

- **Typed Events** - Strongly-typed events for nodes, repositories, workspaces, replication, and schema changes
- **Event Bus** - In-memory pub/sub with async handlers
- **Event Filtering** - Subscribe to specific event types or categories
- **Backpressure** - Semaphore-based concurrency limiting (default: 200 concurrent handlers)
- **Closure Handlers** - Subscribe with closures via `EventBusExt`

## Event Types

| Event | Description |
|-------|-------------|
| `NodeEvent` | Node CRUD, publish/unpublish, property changes, relations |
| `RepositoryEvent` | Repository lifecycle, branches, tags, commits |
| `WorkspaceEvent` | Workspace created/updated/deleted |
| `ReplicationEvent` | Operation batches applied during sync |
| `SchemaEvent` | NodeType, Archetype, ElementType changes |

### Node Event Kinds

- `Created`, `Updated`, `Deleted`
- `Reordered`, `Published`, `Unpublished`
- `PropertyChanged { property }`
- `RelationAdded { relation_type, target_node_id }`
- `RelationRemoved { relation_type, target_node_id }`

### Repository Event Kinds

- `TenantCreated`, `Created`, `Updated`, `Deleted`
- `CommitCreated`
- `BranchCreated`, `BranchUpdated`, `BranchDeleted`
- `TagCreated`, `TagDeleted`

## Usage

### Publishing Events

```rust
use raisin_events::{InMemoryEventBus, EventBus, Event, NodeEvent, NodeEventKind};

let bus = InMemoryEventBus::new();

let event = Event::Node(NodeEvent {
    tenant_id: "acme".into(),
    repository_id: "website".into(),
    branch: "main".into(),
    workspace_id: "content".into(),
    node_id: "article-123".into(),
    node_type: Some("Article".into()),
    revision: hlc,
    kind: NodeEventKind::Created,
    path: Some("/articles/hello-world".into()),
    metadata: None,
});

bus.publish(event);
```

### Subscribing with Closures

```rust
use raisin_events::{InMemoryEventBus, EventBusExt, EventFilter, NodeEventKind};

let bus = InMemoryEventBus::new();

// Subscribe to all node events
bus.subscribe_fn("logger", EventFilter::AllNode, |event| {
    Box::pin(async move {
        println!("Node event: {:?}", event);
        Ok(())
    })
});

// Subscribe to specific event kind
bus.subscribe_fn("indexer", EventFilter::Node(NodeEventKind::Created), |event| {
    Box::pin(async move {
        // Index the new node
        Ok(())
    })
});
```

### Implementing EventHandler Trait

```rust
use raisin_events::{EventHandler, Event};
use std::pin::Pin;
use std::future::Future;
use anyhow::Result;

struct MyHandler;

impl EventHandler for MyHandler {
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Handle event
            Ok(())
        })
    }

    fn name(&self) -> &str {
        "MyHandler"
    }
}
```

## Event Filters

| Filter | Matches |
|--------|---------|
| `EventFilter::All` | All events |
| `EventFilter::AllNode` | All node events |
| `EventFilter::Node(kind)` | Specific node event kind |
| `EventFilter::AllRepository` | All repository events |
| `EventFilter::Repository(kind)` | Specific repository event kind |
| `EventFilter::AllWorkspace` | All workspace events |
| `EventFilter::AllReplication` | All replication events |
| `EventFilter::AllSchema` | All schema events |

## Backpressure

The `InMemoryEventBus` uses a semaphore to limit concurrent handler executions:

```rust
// Default: 200 concurrent handlers
let bus = InMemoryEventBus::new();

// Custom limit
let bus = InMemoryEventBus::with_concurrency_limit(50);

// Check available permits
let available = bus.available_permits();
```

When the limit is reached, new handlers wait for permits. Events are never dropped.

## Crate Usage

Used by:
- `raisin-core` - Core services emit events
- `raisin-storage` - Storage layer event emission
- `raisin-rocksdb` - RocksDB event handlers
- `raisin-server` - Server-level event orchestration
- `raisin-indexer` - Indexing on node events
- `raisin-replication` - Replication event handling
- `raisin-functions` - Serverless function triggers
- `raisin-transport-ws` - WebSocket event streaming

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
