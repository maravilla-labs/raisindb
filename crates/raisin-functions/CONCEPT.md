# Raisin Functions - Concept Document

## Overview

Raisin Functions brings serverless function capabilities to RaisinDB, enabling users to define custom logic that can be triggered by events, scheduled via cron, or invoked via REST API.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Trigger Sources                         │
├─────────────┬─────────────┬─────────────┬──────────────────┤
│ Node Events │  Schedule   │   HTTP API  │   SQL Call       │
│ (create,    │  (cron)     │   /api/     │   (future)       │
│  update...) │             │   functions │                  │
└──────┬──────┴──────┬──────┴──────┬──────┴────────┬─────────┘
       │             │             │               │
       └─────────────┴─────────────┴───────────────┘
                           │
                           ▼
                ┌─────────────────────┐
                │   Job Queue         │
                │   (Unified system)  │
                └──────────┬──────────┘
                           │
                           ▼
                ┌─────────────────────┐
                │  Function Executor  │
                │  - Load function    │
                │  - Check enabled    │
                │  - Select runtime   │
                │  - Execute          │
                └──────────┬──────────┘
                           │
            ┌──────────────┼──────────────┐
            ▼              ▼              ▼
      ┌──────────┐  ┌──────────┐  ┌──────────┐
      │ QuickJS  │  │ Starlark │  │   SQL    │
      │ Runtime  │  │ Runtime  │  │ Runtime  │
      │ (JS)     │  │ (Python) │  │ (basic)  │
      └────┬─────┘  └────┬─────┘  └────┬─────┘
           │             │             │
           └─────────────┼─────────────┘
                         │
                         ▼
              ┌─────────────────────┐
              │   Function API      │
              │   - raisin.nodes    │
              │   - raisin.sql      │
              │   - raisin.http     │
              │   - console.log     │
              └─────────────────────┘
```

## Data Model

### raisin:Function Node

Functions are stored as nodes in the `functions` workspace:

```
/functions/
  /lib/
    /raisin/           # Built-in functions
      /validate_user/
        - code (raisin:Asset)
  /apps/               # User application functions
    /my_app/
      /on_user_signup/
        - code (raisin:Asset)
  /triggers/           # Standalone triggers
    /notify_on_publish/
```

**Node Structure:**
```yaml
type: raisin:Function
properties:
  name: "validate_user"          # Unique identifier
  title: "Validate User"         # Display name
  language: "javascript"         # Runtime: javascript | starlark | sql
  execution_mode: "async"        # async | sync | both
  entrypoint: "handler"          # Function to call (default: "handler")
  enabled: true                  # Enable/disable function
  version: 1                     # Version number
  resource_limits:
    timeout_ms: 30000
    max_memory_bytes: 134217728  # 128MB
    max_stack_bytes: 1048576     # 1MB
  network_policy:
    http_enabled: true
    allowed_urls: ["https://api.example.com/*"]
    max_concurrent_requests: 5
    request_timeout_ms: 10000
  triggers:                      # Inline triggers array
    - name: "on_user_create"
      trigger_type: "node_event"
      event_kinds: ["Created"]
      filters:
        node_types: ["raisin:User"]
      enabled: true
      priority: 0
  input_schema: {}               # JSON Schema for input validation
  output_schema: {}              # JSON Schema for output validation
children:
  - code (raisin:Asset)          # Source code stored as child Asset
```

### raisin:Trigger Node (Standalone)

For complex scenarios, triggers can be separate nodes:

```yaml
type: raisin:Trigger
properties:
  name: "notify_admins"
  function_path: "/lib/raisin/send_notification"
  trigger_type: "node_event"     # node_event | schedule | http
  config:
    event_kinds: ["Deleted"]
    cron_expression: "0 3 * * *" # For schedule type
  filters:
    node_types: ["raisin:User"]
    workspaces: ["default"]
    paths: ["/users/admins/*"]
  enabled: true
  priority: 0
```

## Function API

Functions have access to a `raisin` global object that mirrors the raisin-client-js API:

### JavaScript Example

```javascript
async function handler(input) {
  // Access execution context
  const { tenant_id, branch, actor } = raisin.context;

  // Node operations
  const user = await raisin.nodes.get("default", input.user_path);

  await raisin.nodes.update("default", input.user_path, {
    properties: {
      ...user.properties,
      last_login: new Date().toISOString()
    }
  });

  // SQL queries
  const recentOrders = await raisin.sql.query(
    `SELECT * FROM nodes
     WHERE node_type = 'Order'
       AND properties->>'user_id' = $1
     ORDER BY created_at DESC
     LIMIT 10`,
    [user.id]
  );

  // HTTP requests (allowlisted URLs only)
  if (input.notify_external) {
    await raisin.http.fetch("https://api.example.com/webhook", {
      method: "POST",
      body: { user_id: user.id, event: "login" }
    });
  }

  // Logging
  console.log(`User ${user.id} logged in`);

  // Return result
  return {
    success: true,
    orders_count: recentOrders.row_count
  };
}
```

### API Reference

```javascript
// Node Operations
raisin.nodes.get(workspace, path)                    // Get node by path
raisin.nodes.getById(workspace, id)                  // Get node by ID
raisin.nodes.create(workspace, parentPath, data)     // Create node
raisin.nodes.update(workspace, path, data)           // Update node
raisin.nodes.delete(workspace, path)                 // Delete node
raisin.nodes.query(workspace, query)                 // Query nodes
raisin.nodes.getChildren(workspace, path, limit?)    // Get children

// SQL Operations
raisin.sql.query(sql, params)                        // Run query, return results
raisin.sql.execute(sql, params)                      // Run statement, return affected rows

// HTTP Operations (allowlisted)
raisin.http.fetch(url, options)                      // Make HTTP request

// Events
raisin.events.emit(eventType, data)                  // Emit custom event

// Context (read-only)
raisin.context.tenant_id                             // Current tenant
raisin.context.repo_id                               // Current repository
raisin.context.branch                                // Current branch
raisin.context.workspace_id                          // Current workspace
raisin.context.actor                                 // User/actor ID
raisin.context.execution_id                          // This execution's ID

// Logging
console.log(message)                                 // Info log
console.debug(message)                               // Debug log
console.warn(message)                                // Warning log
console.error(message)                               // Error log
```

## Trigger Types

### 1. Node Events

Triggered when nodes are created, updated, deleted, or published.

```yaml
triggers:
  - name: on_content_update
    trigger_type: node_event
    event_kinds:
      - Created
      - Updated
      - Deleted
      - Published
    filters:
      workspaces: ["default"]
      paths: ["/content/**"]
      node_types: ["raisin:Page", "raisin:Asset"]
    enabled: true
    priority: 0
```

**Event input structure:**
```json
{
  "event": {
    "type": "Created",
    "node_id": "abc123",
    "node_type": "raisin:Page",
    "node_path": "/content/my-page"
  },
  "node": { /* full node data */ }
}
```

### 2. HTTP Triggers

Functions can be invoked directly via REST API.

**Endpoint:** `POST /api/functions/{repo}/{function_name}/invoke`

**Request:**
```json
{
  "input": { "user_id": "123" },
  "sync": true,
  "timeout_ms": 30000
}
```

**Response (sync):**
```json
{
  "execution_id": "abc123",
  "sync": true,
  "result": { "success": true }
}
```

**Response (async):**
```json
{
  "execution_id": "abc123",
  "sync": false,
  "job_id": "job-xyz"
}
```

### 3. Scheduled Triggers (Cron)

Run functions on a cron schedule.

```yaml
triggers:
  - name: daily_cleanup
    trigger_type: schedule
    cron_expression: "0 3 * * *"  # 3 AM daily
    enabled: true
```

**Supported cron formats:**
- Standard 5-field: `minute hour day month day_of_week`
- Special strings: `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`
- Step values: `*/15` (every 15 minutes)
- Ranges: `1-5` (Monday through Friday)
- Lists: `1,3,5` (specific values)

**Scheduled event input:**
```json
{
  "event": {
    "type": "Scheduled",
    "trigger_name": "daily_cleanup",
    "scheduled_time": 1700000000,
    "scheduled_time_iso": "2024-11-14T12:00:00Z"
  }
}
```

## Execution Modes

### Async (Default)

- Function queued as background job
- Returns immediately with execution_id
- Results retrieved via executions API
- Recommended for:
  - Event-triggered functions
  - Long-running operations
  - Operations that modify data

### Sync

- Function executed immediately
- Blocks until complete or timeout
- Returns result directly
- Only for functions with `execution_mode: "sync"` or `"both"`
- Recommended for:
  - HTTP API endpoints needing immediate response
  - Quick validation functions

## Security Model

### Enable/Disable Functions

Functions can be enabled or disabled via the `enabled` property:

```sql
-- Disable a function
UPDATE nodes
SET properties = properties || '{"enabled": false}'
WHERE path = '/apps/my-function'
AND workspace = 'functions';
```

When disabled:
- REST API invocations return `400 Validation Error`
- Event triggers skip the function
- Scheduled triggers skip the function

### Sandboxing

1. **Memory Limits** - Default 128MB, configurable per function
2. **Time Limits** - Default 30s, configurable per function
3. **Stack Limits** - Default 1MB, prevents infinite recursion
4. **Instruction Limits** - For QuickJS runtime

### Network Access

Functions cannot make arbitrary HTTP requests. Network access is:

1. **Disabled by default** - `http_enabled: false`
2. **Allowlisted per-function** - Glob patterns for allowed URLs
3. **Limited concurrency** - Max concurrent requests (default 5)
4. **Timed out** - Request timeout separate from function timeout

### Data Access

- Functions operate within tenant/repo/branch context
- Cannot access other tenants' data
- Respect workspace boundaries
- Full audit trail of operations

## Job Types

Functions use the RaisinDB unified job system:

```rust
// Execute a function
JobType::FunctionExecution {
    function_path: String,      // e.g., "/apps/my-function"
    trigger_name: Option<String>, // e.g., "on_user_create"
    execution_id: String,       // Unique execution ID
}

// Evaluate triggers for a node event
JobType::TriggerEvaluation {
    event_type: String,         // Created, Updated, Deleted, Published
    node_id: String,
    node_type: String,
}

// Check scheduled triggers (runs periodically)
JobType::ScheduledTriggerCheck {
    tenant_id: Option<String>,
    repo_id: Option<String>,
}
```

**Event Flow:**
1. Event occurs (node created, cron fires, HTTP request)
2. `TriggerEvaluation` or `ScheduledTriggerCheck` job enqueued
3. Job finds matching triggers, filters disabled functions
4. `FunctionExecution` job enqueued for each match
5. Handler checks `enabled` status
6. Function executed by QuickJS runtime
7. Result stored, job completed

## REST API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/functions/{repo}` | List all functions |
| GET | `/api/functions/{repo}/{name}` | Get function details |
| POST | `/api/functions/{repo}/{name}/invoke` | Invoke function |
| GET | `/api/functions/{repo}/{name}/executions` | List executions |
| GET | `/api/functions/{repo}/{name}/executions/{id}` | Get execution status |

## Configuration

### Resource Limits

| Setting | Default | Description |
|---------|---------|-------------|
| timeout_ms | 30,000 | Max execution time |
| max_memory_bytes | 128MB | Max memory usage |
| max_stack_bytes | 1MB | Stack size limit |

### Network Policy

| Setting | Default | Description |
|---------|---------|-------------|
| http_enabled | false | Allow HTTP requests |
| allowed_urls | [] | URL patterns allowed |
| max_concurrent_requests | 5 | Concurrent HTTP limit |
| request_timeout_ms | 10,000 | Per-request timeout |

## Examples

### Webhook on Node Publish

```javascript
// Function: /apps/my_app/notify_on_publish
async function handler(input) {
  const node = await raisin.nodes.get("default", input.event.node_path);

  await raisin.http.fetch("https://hooks.slack.com/...", {
    method: "POST",
    body: {
      text: `Page published: ${node.properties.title}`
    }
  });

  return { notified: true };
}
```

Trigger configuration:
```yaml
triggers:
  - name: on_publish
    trigger_type: node_event
    event_kinds: [Published]
    filters:
      node_types: [raisin:Page]
    enabled: true
```

### Data Validation (Sync)

```javascript
// Function: /lib/validate_user
// execution_mode: "both"
async function handler(input) {
  const errors = [];

  if (!input.email || !input.email.includes('@')) {
    errors.push('Invalid email');
  }

  if (!input.name || input.name.length < 2) {
    errors.push('Name too short');
  }

  return {
    valid: errors.length === 0,
    errors
  };
}
```

Invoke synchronously:
```bash
curl -X POST /api/functions/myrepo/validate_user/invoke \
  -H "Content-Type: application/json" \
  -d '{"input": {"email": "test@example.com", "name": "Jo"}, "sync": true}'
```

### Scheduled Cleanup

```javascript
// Function: /apps/maintenance/cleanup_old_drafts
async function handler(input) {
  const cutoff = new Date();
  cutoff.setDate(cutoff.getDate() - 30);

  const result = await raisin.sql.execute(`
    DELETE FROM nodes
    WHERE node_type = 'Draft'
      AND created_at < $1
  `, [cutoff.toISOString()]);

  console.log(`Deleted ${result} old drafts`);

  return { deleted: result };
}
```

Trigger configuration:
```yaml
triggers:
  - name: daily_cleanup
    trigger_type: schedule
    cron_expression: "0 3 * * *"  # 3 AM daily
    enabled: true
```

## Implementation Status

| Component | Status | Notes |
|-----------|--------|-------|
| **Core Types** | ✅ Complete | Function, Trigger, ExecutionContext, ResourceLimits, NetworkPolicy |
| **Runtime Trait** | ✅ Complete | FunctionRuntime trait with async execute/validate |
| **FunctionApi Trait** | ✅ Complete | Full API: nodes, sql, http, events, logging, context |
| **QuickJS Runtime** | ✅ Complete | JavaScript execution with full API bindings |
| **Starlark Runtime** | ✅ Complete | Python-like syntax with full API bindings |
| **SQL Runtime** | ⚠️ Basic | SQL passthrough only (no context/API access) |
| **NodeType: raisin:Function** | ✅ Complete | Full schema with triggers, limits, network policy |
| **NodeType: raisin:Trigger** | ✅ Complete | Standalone trigger nodes |
| **Workspace: functions** | ✅ Complete | With /lib, /apps, /triggers structure |
| **Job: FunctionExecution** | ✅ Complete | With enabled check |
| **Job: TriggerEvaluation** | ✅ Complete | Node event matching |
| **Job: ScheduledTriggerCheck** | ✅ Complete | Cron expression evaluation |
| **REST API: List/Get** | ✅ Complete | /api/functions endpoints |
| **REST API: Invoke** | ✅ Complete | Sync and async modes |
| **REST API: Executions** | ✅ Complete | Execution status tracking |
| **Enabled/Disabled Check** | ✅ Complete | All trigger types respect enabled flag |
| **SQL Integration** | 🔲 Planned | CREATE FUNCTION syntax |
