# raisin-transport-inprocess

> **Status: Experimental / Not Yet Released**

Direct in-process API facade for RaisinDB, enabling zero-network-overhead database access.

## Overview

This crate provides a lightweight wrapper around `NodeService` and `WorkspaceService` from `raisin-core`, allowing Rust applications to interact with RaisinDB directly without going through HTTP, WebSocket, or PostgreSQL wire protocol transports.

Key features:

- **Zero Network Overhead** - Direct function calls to core services
- **Type-Safe API** - Full Rust type safety with `raisin-models`
- **Async Operations** - All methods are async-ready
- **Workspace & Node Operations** - Complete CRUD for workspaces and nodes

## Use Cases

| Use Case | Description |
|----------|-------------|
| **Embedded Mode** | Using RaisinDB as a library within another Rust application |
| **Testing** | Unit/integration tests without network setup |
| **CLI Tools** | Direct interaction with local RaisinDB instance |
| **Plugins/Extensions** | Internal subsystems that need direct DB access |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Your Application                          │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
              ┌───────────────────────┐
              │    InProcessApi<S>    │
              │  - workspace methods  │
              │  - node methods       │
              └───────────┬───────────┘
                          │
           ┌──────────────┴──────────────┐
           ▼                             ▼
┌─────────────────────┐     ┌─────────────────────┐
│  WorkspaceService   │     │    NodeService      │
│    (raisin-core)    │     │   (raisin-core)     │
└──────────┬──────────┘     └──────────┬──────────┘
           └──────────────┬────────────┘
                          ▼
              ┌───────────────────────┐
              │   Storage<S> Layer    │
              │   (RocksDB, etc.)     │
              └───────────────────────┘
```

## Usage

```rust
use raisin_transport_inprocess::InProcessApi;
use raisin_storage::RocksDbStorage;
use std::sync::Arc;

// Initialize with your storage backend
let storage = Arc::new(RocksDbStorage::open("./data")?);
let api = InProcessApi::new(storage);

// Workspace operations
let workspaces = api.list_workspaces("main").await?;
let ws = api.get_workspace("main", "default").await?;

// Node operations (scoped to workspace)
let node = api.get_node("default", "node-123").await?;
let children = api.list_children("default", "parent-id").await?;
let by_path = api.get_by_path("default", "/users/john").await?;

// Tree traversal
let nested = api.deep_children_nested("default", "parent-id", 3).await?;
let flat = api.deep_children_flat("default", "parent-id", 3).await?;

// Mutations
api.put_node("default", node).await?;
api.move_node("default", "node-id", "/new/path").await?;
api.rename_node("default", "/old/path", "new-name").await?;
api.delete_node("default", "node-id").await?;
```

## API Methods

### Workspace Operations

| Method | Description |
|--------|-------------|
| `list_workspaces(repo)` | List all workspaces in a repository |
| `get_workspace(repo, name)` | Get a specific workspace by name |
| `put_workspace(repo, ws)` | Create or update a workspace |

### Node Operations

| Method | Description |
|--------|-------------|
| `get_node(ws, id)` | Get node by ID |
| `get_by_path(ws, path)` | Get node by path |
| `put_node(ws, node)` | Create or update a node |
| `delete_node(ws, id)` | Delete node by ID |
| `delete_by_path(ws, path)` | Delete node by path |
| `list_all(ws)` | List all nodes in workspace |
| `list_root(ws)` | List root-level nodes |
| `list_children(ws, parent)` | List direct children of a node |
| `deep_children_nested(ws, parent, depth)` | Get nested tree structure |
| `deep_children_flat(ws, parent, depth)` | Get flattened tree |
| `move_node(ws, id, new_path)` | Move node to new path |
| `rename_node(ws, old_path, new_name)` | Rename a node |
| `reorder_child(ws, parent, child, pos)` | Reorder child position |
| `move_child_before(ws, parent, child, before)` | Move child before sibling |
| `move_child_after(ws, parent, child, after)` | Move child after sibling |

## Current Limitations

This crate is experimental and has the following limitations:

| Limitation | Current Behavior | Planned |
|------------|------------------|---------|
| **Hardcoded tenant** | Always uses `"default"` | Configurable context |
| **Hardcoded repo/branch** | Always uses `"main"/"main"` | Configurable context |
| **No authentication** | Bypasses auth layer | Optional auth support |
| **No transactions** | Individual operations only | Transaction support |
| **No SQL queries** | Node API only | SQL query passthrough |

## Planned Features

- Configurable `InProcessContext` for tenant/repo/branch
- Multi-tenant support
- Transaction API
- SQL query support via `QueryService`
- Edge/relationship operations

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
