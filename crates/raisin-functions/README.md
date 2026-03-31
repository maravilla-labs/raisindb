# raisin-functions

Serverless functions for RaisinDB with JavaScript (QuickJS) and Starlark (Python-like) runtimes.

## Overview

Enables custom logic execution triggered by events, schedules, or HTTP API. Functions run in sandboxed environments with controlled access to RaisinDB operations, SQL queries, and external HTTP APIs.

## Features

- **Full-Featured Runtimes** - JavaScript (QuickJS), Starlark (Python-like) with complete API
- **SQL Passthrough** - Basic SQL execution (no context/API access)
- **Sandboxed Execution** - Resource limits, timeouts, memory constraints
- **RaisinDB API** - Node CRUD, SQL queries, event emission (JS/Starlark only)
- **Allowlisted HTTP** - Controlled external API access with URL patterns
- **Flexible Triggers** - Event-driven, cron scheduled, HTTP invocation
- **Async/Sync Modes** - Background jobs or immediate execution

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Trigger Sources                         │
├─────────────┬─────────────┬─────────────┬──────────────────┤
│ Node Events │   Schedule  │   HTTP API  │    SQL Call      │
│ (CRUD)      │   (cron)    │   /invoke   │                  │
└──────┬──────┴──────┬──────┴──────┬──────┴────────┬─────────┘
       └─────────────┴─────────────┴───────────────┘
                           │
                           ▼
                ┌─────────────────────┐
                │      Job Queue      │
                │   (unified system)  │
                └──────────┬──────────┘
                           │
                           ▼
                ┌─────────────────────┐
                │  Function Executor  │
                │  - Load function    │
                │  - Select runtime   │
                │  - Apply sandbox    │
                └──────────┬──────────┘
                           │
            ┌──────────────┼──────────────┐
            ▼              ▼              ▼
      ┌──────────┐  ┌──────────┐  ┌──────────┐
      │ QuickJS  │  │ Starlark │  │   SQL    │
      │ (JS)     │  │ (Python) │  │ (basic)  │
      └────┬─────┘  └────┬─────┘  └────┬─────┘
           │             │             │
           └──────┬──────┘             │ (passthrough)
                  ▼                    ▼
       ┌─────────────────────┐  ┌──────────────┐
       │    Function API     │  │ sql_query()  │
       │  raisin.nodes.*     │  │ only         │
       │  raisin.sql.*       │  └──────────────┘
       │  raisin.http.*      │
       │  raisin.ai.*        │
       │  console.log        │
       └─────────────────────┘
```

## Usage

### JavaScript Function

```javascript
async function handler(input) {
  // Access context
  const { tenant_id, branch } = raisin.context;

  // Node operations
  const user = await raisin.nodes.get("default", input.user_path);
  await raisin.nodes.update("default", input.user_path, {
    properties: { ...user.properties, last_login: new Date().toISOString() }
  });

  // SQL queries
  const orders = await raisin.sql.query(
    `SELECT * FROM nodes WHERE node_type = 'Order' LIMIT 10`
  );

  // HTTP requests (allowlisted URLs only)
  await raisin.http.fetch("https://api.example.com/webhook", {
    method: "POST",
    body: { user_id: user.id }
  });

  return { success: true, orders_count: orders.row_count };
}
```

### Starlark Function

```python
def handler(input):
    # Access context
    tenant = raisin.context.tenant_id

    # Node operations
    user = raisin.nodes.get("default", input["user_path"])

    # SQL queries
    orders = raisin.sql.query("SELECT * FROM nodes LIMIT 10")

    # Logging
    print("Processed user:", user["id"])

    return {"success": True}
```

### SQL Function (Basic)

SQL runtime is a **passthrough only** - no access to `raisin.context` or API methods.

```sql
-- Input: {"params": ["user-123", 10]}
-- Parameters accessed via $1, $2, etc.
SELECT * FROM nodes
WHERE properties->>'user_id' = $1
LIMIT $2
```

**SQL Runtime Limitations:**
- No `raisin.context` (tenant_id, branch, actor unavailable)
- No `raisin.nodes.*`, `raisin.http.*`, `raisin.ai.*` operations
- Input array maps to `$1`, `$2`, etc. placeholders
- Best for simple data transformations, not complex logic

## Function API

> **Note:** The Function API below is available in **JavaScript and Starlark only**. SQL runtime uses direct query execution.

| Namespace | Methods |
|-----------|---------|
| `raisin.nodes` | `get`, `getById`, `create`, `update`, `delete`, `query`, `getChildren` |
| `raisin.sql` | `query`, `execute` |
| `raisin.http` | `fetch` |
| `raisin.events` | `emit` |
| `raisin.ai` | `chat`, `embed`, `generateImage` |
| `raisin.context` | `tenant_id`, `repo_id`, `branch`, `workspace_id`, `actor` |
| `console` | `log`, `debug`, `warn`, `error` |

## Trigger Types

### Node Events

```yaml
triggers:
  - name: on_user_create
    trigger_type: node_event
    event_kinds: [Created, Updated]
    filters:
      node_types: ["User"]
      workspaces: ["default"]
```

### Scheduled (Cron)

```yaml
triggers:
  - name: daily_cleanup
    trigger_type: schedule
    cron_expression: "0 3 * * *"  # 3 AM daily
```

### HTTP Invocation

```bash
POST /api/functions/{repo}/{function_name}/invoke
{
  "input": { "user_id": "123" },
  "sync": true
}
```

## Modules

| Module | Description |
|--------|-------------|
| `api/` | Function API trait and callbacks (nodes, sql, http, ai) |
| `execution/` | Executor, code loader, AI provider integration |
| `runtime/` | QuickJS, Starlark, SQL runtime implementations |
| `types/` | Function, Trigger, ExecutionContext, ResourceLimits |
| `loader/` | Function code loading from storage |

### Runtime Components

| Component | Description |
|-----------|-------------|
| `runtime/quickjs.rs` | JavaScript runtime with full API bindings |
| `runtime/starlark.rs` | Starlark (Python-like) runtime with full API bindings |
| `runtime/sql.rs` | SQL passthrough runtime (basic, no context/API) |
| `runtime/sandbox.rs` | Resource limits enforcement |
| `runtime/fetch/` | HTTP client with allowlist, streaming |
| `runtime/timers/` | setTimeout/setInterval polyfills |
| `runtime/bindings/` | Unified method registry for all runtimes |

## Resource Limits

| Setting | Default | Description |
|---------|---------|-------------|
| `timeout_ms` | 30,000 | Max execution time |
| `max_memory_bytes` | 128MB | Memory limit |
| `max_stack_bytes` | 1MB | Stack size limit |

## Network Policy

| Setting | Default | Description |
|---------|---------|-------------|
| `http_enabled` | false | Allow HTTP requests |
| `allowed_urls` | [] | URL glob patterns |
| `max_concurrent_requests` | 5 | Concurrent HTTP limit |
| `request_timeout_ms` | 10,000 | Per-request timeout |

## Crate Usage

Used by:
- `raisin-server` - Function execution worker
- `raisin-transport-http` - REST API endpoints for invocation

## Documentation

See [CONCEPT.md](./CONCEPT.md) for detailed architecture, data model, and API reference.

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
