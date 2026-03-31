# raisin-transport-ws

WebSocket transport layer for RaisinDB enabling real-time bidirectional communication.

## Overview

Provides a high-performance WebSocket interface for RaisinDB with:

- **MessagePack Serialization** - Efficient binary protocol for low-latency communication
- **JWT Authentication** - Secure token-based auth with access/refresh token pairs
- **Event Subscriptions** - Real-time change notifications with flexible filtering
- **Row-Level Security** - RLS enforcement on all operations and event forwarding
- **Concurrency Control** - Per-connection and global rate limiting with semaphores
- **Transaction Support** - Multi-operation atomic commits via WebSocket sessions
- **Anonymous Access** - Configurable anonymous mode with resolved permissions

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     WebSocket Transport                          │
│                                                                  │
│  ┌────────────────┐    ┌─────────────────┐   ┌───────────────┐  │
│  │  WsState       │    │ ConnectionState │   │  ConnectionRegistry │
│  │  - storage     │    │  - subscriptions│   │  - workspace index  │
│  │  - auth_svc    │    │  - credits      │   │  - connections map  │
│  │  - event_bus   │    │  - transaction  │   └───────────────┘  │
│  └───────┬────────┘    └────────┬────────┘                      │
│          │                      │                                │
│          ▼                      ▼                                │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │              websocket_handler (Axum)                        │ │
│  │   - Connection upgrade        - Message routing              │ │
│  │   - JWT extraction            - Response/Event channels      │ │
│  └──────────────────────────┬──────────────────────────────────┘ │
│                             │                                    │
│          ┌──────────────────┼──────────────────┐                 │
│          ▼                  ▼                  ▼                 │
│   ┌────────────┐    ┌────────────┐    ┌────────────┐            │
│   │   Nodes    │    │   Schema   │    │ Subscriptions│           │
│   │  handlers  │    │  handlers  │    │  handlers   │           │
│   └────────────┘    └────────────┘    └────────────┘            │
└─────────────────────────────────────────────────────────────────┘
```

## Protocol

All messages use MessagePack serialization with named fields.

### Request Envelope

```json
{
  "request_id": "uuid-v4",
  "type": "node_create",
  "context": {
    "tenant_id": "default",
    "repository": "my-repo",
    "branch": "main",
    "workspace": "content"
  },
  "payload": { ... }
}
```

### Response Envelope

```json
{
  "request_id": "uuid-v4",
  "status": "success",
  "result": { ... },
  "metadata": {
    "chunk": 1,
    "has_more": true
  }
}
```

### Event Message

```json
{
  "event_id": "uuid-v4",
  "subscription_id": "sub-123",
  "event_type": "node:created",
  "payload": { ... },
  "timestamp": "2024-01-15T10:30:00Z"
}
```

## Request Types

| Category | Operations |
|----------|-----------|
| **Auth** | `authenticate`, `authenticate_jwt`, `refresh_token` |
| **Nodes** | `node_create`, `node_update`, `node_delete`, `node_get`, `node_query` |
| **Tree** | `node_list_children`, `node_get_tree`, `node_get_tree_flat` |
| **Move/Copy** | `node_move`, `node_rename`, `node_copy`, `node_copy_tree`, `node_reorder` |
| **Properties** | `property_get`, `property_update` |
| **Relations** | `relation_add`, `relation_remove`, `relations_get` |
| **Translations** | `translation_update`, `translation_list`, `translation_delete` |
| **SQL** | `sql_query` |
| **Workspaces** | `workspace_create`, `workspace_get`, `workspace_list`, `workspace_delete` |
| **Branches** | `branch_create`, `branch_get`, `branch_list`, `branch_merge`, `branch_compare` |
| **Tags** | `tag_create`, `tag_get`, `tag_list`, `tag_delete` |
| **Schema** | `node_type_*`, `archetype_*`, `element_type_*` |
| **Repos** | `repository_create`, `repository_get`, `repository_list`, `repository_delete` |
| **Events** | `subscribe`, `unsubscribe` |
| **Transactions** | `transaction_begin`, `transaction_commit`, `transaction_rollback` |

## Usage

### Server Setup

```rust
use raisin_transport_ws::{WsConfig, WsState, websocket_handler};
use axum::{Router, routing::get};
use std::sync::Arc;

// Configure WebSocket transport
let config = WsConfig {
    max_concurrent_ops: 600,
    initial_credits: 500,
    jwt_secret: "your-secret-key".to_string(),
    require_auth: true,
    global_concurrency_limit: Some(1000),
    anonymous_enabled: false,
};

// Create shared state
let state = Arc::new(WsState::new(
    storage,
    connection,
    workspace_service,
    binary_storage,
    config,
));

// Mount WebSocket endpoint
let app = Router::new()
    .route("/sys/:tenant_id", get(websocket_handler))
    .route("/sys/:tenant_id/:repository", get(websocket_handler))
    .with_state(state);
```

### Event Subscriptions

```rust
// Subscribe to node events
{
  "type": "subscribe",
  "payload": {
    "filters": {
      "workspace": "content",
      "path": "/posts/*",
      "event_types": ["node:created", "node:updated"],
      "node_type": "Article",
      "include_node": true
    }
  }
}

// Events are automatically forwarded when matching filters
// RLS is applied - users only receive events for readable nodes
```

### Transactions

```rust
// Begin transaction
{ "type": "transaction_begin", "payload": { "message": "Batch update" } }

// Execute operations within transaction
{ "type": "node_create", ... }
{ "type": "node_update", ... }

// Commit atomically
{ "type": "transaction_commit" }

// Or rollback
{ "type": "transaction_rollback" }
```

## Modules

| Module | Description |
|--------|-------------|
| `handler.rs` | Main WebSocket handler, connection lifecycle, message routing |
| `protocol.rs` | Request/response envelopes, payload types, request type enum |
| `connection.rs` | Per-connection state, subscriptions, credits, transactions |
| `registry.rs` | Global connection registry with workspace-indexed lookups |
| `event_handler.rs` | Event bus integration, RLS-filtered event forwarding |
| `auth.rs` | JWT token generation, validation, extraction from headers |
| `error.rs` | WebSocket-specific error types with error codes |
| `handlers/` | Request handlers for all operation types |

## Handler Modules

| Handler | Operations |
|---------|-----------|
| `handlers/auth.rs` | Authentication and token refresh |
| `handlers/nodes.rs` | Node CRUD, SQL queries, property operations |
| `handlers/subscriptions.rs` | Event subscribe/unsubscribe |
| `handlers/transactions.rs` | Transaction begin/commit/rollback |
| `handlers/workspaces.rs` | Workspace management |
| `handlers/branches.rs` | Branch operations |
| `handlers/tags.rs` | Tag management |
| `handlers/node_types.rs` | NodeType schema operations |
| `handlers/archetypes.rs` | Archetype schema operations |
| `handlers/element_types.rs` | ElementType schema operations |
| `handlers/repositories.rs` | Repository management |
| `handlers/translations.rs` | Locale translations |

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `max_concurrent_ops` | 600 | Max concurrent operations per connection |
| `initial_credits` | 500 | Flow control credits per connection |
| `jwt_secret` | - | Secret for JWT signing/validation |
| `require_auth` | true | Require authentication for all requests |
| `global_concurrency_limit` | 1000 | Max concurrent operations globally |
| `anonymous_enabled` | false | Allow anonymous access with resolved permissions |

## Security Features

- **JWT Authentication** - Access tokens (1 hour) and refresh tokens (7 days)
- **Row-Level Security** - RLS applied to all operations and event forwarding
- **Auth Context** - System, user, and anonymous contexts with permission scopes
- **Rate Limiting** - Per-connection semaphores prevent resource exhaustion
- **Deny-All Default** - Unauthenticated connections get deny-all context

## Features

```toml
[dependencies]
raisin-transport-ws = { version = "0.1", features = ["storage-rocksdb"] }
```

| Feature | Description |
|---------|-------------|
| `default` | Includes `storage-rocksdb` |
| `storage-rocksdb` | RocksDB backend with SQL execution, indexer, HNSW |

## Integration

This crate is used by:

- `raisin-server` - WebSocket endpoint mounting

Depends on:

- `raisin-core` - Node services, permission resolution
- `raisin-storage` - Storage abstraction
- `raisin-events` - Event bus for subscriptions
- `raisin-models` - Node, auth context types
- `raisin-sql-execution` - SQL query execution (with storage-rocksdb)

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
