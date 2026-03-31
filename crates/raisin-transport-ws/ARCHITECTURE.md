# raisin-transport-ws Architecture

This document describes the internal architecture of the WebSocket transport layer.

## Overview

The WebSocket transport provides a real-time, bidirectional communication channel between clients and RaisinDB. It handles connection management, request routing, event subscriptions, and security enforcement.

## Component Diagram

```
                                 ┌─────────────────────────────┐
                                 │         Client              │
                                 │  (Browser/SDK/CLI)          │
                                 └─────────────┬───────────────┘
                                               │ WebSocket
                                               │ (MessagePack)
                                               ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                              raisin-transport-ws                              │
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                         websocket_handler                                │ │
│  │  ┌─────────────────┐  ┌──────────────────┐  ┌────────────────────────┐  │ │
│  │  │ Connection      │  │ Message          │  │ Response/Event         │  │ │
│  │  │ Upgrade (Axum)  │──│ Deserialization  │──│ Channel Management     │  │ │
│  │  └─────────────────┘  └──────────────────┘  └────────────────────────┘  │ │
│  └───────────────────────────────────┬─────────────────────────────────────┘ │
│                                      │                                       │
│  ┌───────────────────────────────────▼─────────────────────────────────────┐ │
│  │                          WsState (Shared)                                │ │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐ │ │
│  │  │   Storage    │  │ AuthService  │  │  EventBus   │  │   Config     │ │ │
│  │  │   (Arc<S>)   │  │  (JWT)       │  │  (pub/sub)  │  │   (limits)   │ │ │
│  │  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘ │ │
│  │  ┌───────────────────────────────────────────────────────────────────┐  │ │
│  │  │                     ConnectionRegistry                             │  │ │
│  │  │  ┌─────────────────┐     ┌──────────────────────────────────────┐ │  │ │
│  │  │  │ connections     │     │ workspace_subscribers                 │ │  │ │
│  │  │  │ (DashMap)       │     │ (workspace -> connection_ids)         │ │  │ │
│  │  │  └─────────────────┘     └──────────────────────────────────────┘ │  │ │
│  │  └───────────────────────────────────────────────────────────────────┘  │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                      │                                       │
│  ┌───────────────────────────────────▼─────────────────────────────────────┐ │
│  │                    ConnectionState (Per-Connection)                      │ │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐ │ │
│  │  │ Auth Context │  │ Subscriptions│  │ Transaction  │  │ Semaphores   │ │ │
│  │  │ (RLS)        │  │ (filters)    │  │ Context      │  │ (rate limit) │ │ │
│  │  └──────────────┘  └──────────────┘  └──────────────┘  └──────────────┘ │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                      │                                       │
│  ┌───────────────────────────────────▼─────────────────────────────────────┐ │
│  │                          Request Handlers                                │ │
│  │  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐            │ │
│  │  │   Auth    │  │   Nodes   │  │  Schema   │  │   Subs    │  ...       │ │
│  │  │  handlers │  │  handlers │  │  handlers │  │  handlers │            │ │
│  │  └───────────┘  └───────────┘  └───────────┘  └───────────┘            │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
│                                                                               │
│  ┌─────────────────────────────────────────────────────────────────────────┐ │
│  │                         WsEventHandler                                   │ │
│  │  ┌─────────────────────────────────────────────────────────────────┐    │ │
│  │  │  Event Bus Subscriber -> RLS Filter -> Subscription Match       │    │ │
│  │  │                          -> Forward to Connection Channels       │    │ │
│  │  └─────────────────────────────────────────────────────────────────┘    │ │
│  └─────────────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Core Components

### WsState

Global shared state for all WebSocket connections.

```rust
pub struct WsState<S, B> {
    pub storage: Arc<S>,           // Storage backend
    pub connection: Arc<RaisinConnection<S>>,
    pub ws_svc: Arc<WorkspaceService<S>>,
    pub bin: Arc<B>,               // Binary storage
    pub config: WsConfig,          // Rate limits, JWT secret
    pub auth_service: Arc<JwtAuthService>,
    pub global_semaphore: Option<Arc<Semaphore>>,
    pub event_bus: Arc<dyn EventBus>,
    pub connection_registry: Arc<ConnectionRegistry>,
}
```

### ConnectionState

Per-connection state tracking authentication, subscriptions, and resources.

```rust
pub struct ConnectionState {
    pub connection_id: String,     // UUID for this connection
    pub tenant_id: String,
    pub repository: Option<String>,
    pub user_id: Option<String>,   // Set after authentication
    auth_context: Option<AuthContext>,  // For RLS enforcement
    subscriptions: Arc<DashMap<String, SubscriptionFilters>>,
    operation_semaphore: Arc<Semaphore>,
    credits: Arc<RwLock<u32>>,     // Flow control
    transaction_context: Arc<Mutex<Option<TransactionalContext>>>,
}
```

### ConnectionRegistry

Thread-safe registry of all active connections with workspace indexing.

```rust
pub struct ConnectionRegistry {
    connections: DashMap<String, Arc<RwLock<ConnectionState>>>,
    workspace_subscribers: DashMap<String, DashSet<String>>,
}
```

The workspace index enables efficient event routing - instead of checking all connections for each event, we can quickly look up which connections are subscribed to a specific workspace.

## Request Flow

```
1. Client connects via WebSocket
   │
   ▼
2. websocket_handler() upgrades connection
   │
   ├─ Extract JWT from headers (if present)
   ├─ Authenticate or create deny-all context
   └─ Create ConnectionState
   │
   ▼
3. Spawn response/event sender task
   │
   ├─ Listen on response_rx channel
   └─ Listen on event_rx channel
   │
   ▼
4. Message receive loop
   │
   ├─ Deserialize MessagePack → RequestEnvelope
   ├─ Check authentication requirements
   ├─ Acquire global semaphore (rate limit)
   ├─ Acquire connection semaphore (rate limit)
   └─ Route to appropriate handler
   │
   ▼
5. Request handler execution
   │
   ├─ Parse payload to typed struct
   ├─ Build NodeService/WorkspaceService with auth context
   ├─ Execute operation with RLS enforcement
   └─ Return ResponseEnvelope
   │
   ▼
6. Response sent via channel → WebSocket
```

## Event Subscription Flow

```
1. Client sends Subscribe request
   │
   ▼
2. handle_subscribe()
   │
   ├─ Parse SubscriptionFilters
   ├─ Add to ConnectionState.subscriptions (with deduplication)
   └─ Update ConnectionRegistry workspace index
   │
   ▼
3. Database operation triggers event
   │
   ├─ NodeService emits event to EventBus
   └─ WsEventHandler.handle() is called
   │
   ▼
4. WsEventHandler processing
   │
   ├─ Get connections from registry (workspace-indexed lookup)
   ├─ For each connection:
   │   ├─ Check subscription filters match
   │   ├─ RLS check: can user read this node?
   │   └─ Build EventMessage with optional node data
   └─ Send to connection's event channel
   │
   ▼
5. Event delivered to client via WebSocket
```

## Authentication Flow

### JWT Authentication (Header)

```
1. Client connects with Authorization: Bearer <token>
   │
   ▼
2. websocket_handler extracts token
   │
   ▼
3. authenticate_with_token()
   │
   ├─ Try validate as WebSocket JWT (admin users)
   │   └─ Success: AuthContext::system()
   │
   └─ Try decode as Identity JWT (identity users)
       └─ Success: AuthContext::for_user() with email, home
   │
   ▼
4. ConnectionState created with auth context
```

### Anonymous Access

```
1. Client connects without token
   │
   ▼
2. Check is_anonymous_enabled()
   │
   ├─ Repo-level config (highest priority)
   ├─ Tenant-level config
   └─ Global config (lowest priority)
   │
   ▼
3a. Anonymous enabled:
    │
    ├─ Resolve anonymous user permissions
    ├─ Generate anonymous JWT for HTTP API calls
    └─ Set AuthContext::for_user("anonymous")

3b. Anonymous disabled:
    │
    └─ Set AuthContext::deny_all()
```

## Transaction Flow

```
1. Client: transaction_begin
   │
   ▼
2. handle_transaction_begin()
   │
   ├─ storage.begin_context()
   ├─ Set tenant, repo, branch, actor
   ├─ Set auth context for RLS
   └─ Store in ConnectionState.transaction_context
   │
   ▼
3. Client: node_create, node_update, ...
   │
   ▼
4. Handlers detect active transaction
   │
   ├─ Use TransactionalContext instead of NodeService
   └─ Operations are batched (no commits)
   │
   ▼
5a. Client: transaction_commit
    │
    ├─ Take transaction context from ConnectionState
    └─ ctx.commit() - atomic write

5b. Client: transaction_rollback
    │
    ├─ Take transaction context from ConnectionState
    └─ ctx.rollback() - discard changes
```

## Concurrency Control

### Per-Connection Limiting

Each connection has a semaphore limiting concurrent operations:

```rust
let _permit = match operation_semaphore.try_acquire() {
    Ok(permit) => permit,
    Err(_) => return Err(WsError::RateLimitExceeded),
};
```

### Global Limiting

Optional global semaphore prevents server overload:

```rust
let _global_permit = if let Some(ref semaphore) = state.global_semaphore {
    semaphore.try_acquire()?
} else {
    None
};
```

### Flow Control Credits

Clients have a credit budget for backpressure:

```rust
// Consumer operations decrement credits
connection.try_consume_credits(1);

// Clients can replenish credits
connection.add_credits(amount);
```

## Subscription Matching

Subscriptions support flexible filtering with wildcards:

```rust
SubscriptionFilters {
    workspace: Some("content"),     // Exact match
    path: Some("/posts/*"),         // Wildcard patterns
    event_types: Some(vec!["node:created"]),
    node_type: Some("Article"),
    include_node: true,             // Include full node in event
}
```

Path matching supports:
- Exact: `/folder/file.txt`
- Single level wildcard: `/folder/*`
- Recursive wildcard: `/folder/**`
- SQL LIKE style: `/posts/%` (converted to `*`)

## RLS Enforcement

Row-Level Security is applied at multiple levels:

1. **Request handlers** - NodeService created with auth context
2. **Event forwarding** - RLS check before sending events
3. **Transactions** - Auth context propagated to transaction context

```rust
// In event handler
if !rls_filter::can_perform(node, Operation::Read, auth, &scope) {
    continue; // Skip this connection
}
```

## Error Handling

Errors are converted to WsError with standard codes:

```rust
pub enum WsError {
    AuthError(_) => "AUTH_ERROR",
    NotAuthenticated => "NOT_AUTHENTICATED",
    PermissionDenied => "PERMISSION_DENIED",
    RateLimitExceeded => "RATE_LIMIT_EXCEEDED",
    InvalidRequest(_) => "INVALID_REQUEST",
    StorageError(_) => "STORAGE_ERROR",
    // ...
}
```

## Performance Considerations

1. **Workspace indexing** - O(1) lookup for event routing
2. **Subscription deduplication** - Identical filters reuse subscription ID
3. **MessagePack** - Smaller payloads than JSON
4. **Semaphore-based rate limiting** - No lock contention
5. **DashMap** - Concurrent map with sharded locks
6. **Channel-based messaging** - Non-blocking response sending
