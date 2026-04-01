# Serverless Functions and Workflows

RaisinDB includes a serverless function runtime (`raisin-functions`) and a stateful workflow engine (`raisin-flow-runtime`) for executing custom logic directly within the database.

## Serverless Functions

The `raisin-functions` crate provides sandboxed execution of user-defined functions in multiple languages.

### Supported Runtimes

| Runtime | Language | Engine |
|---------|----------|--------|
| QuickJS | JavaScript | QuickJS embedded runtime |
| Starlark | Python-like | Starlark interpreter |
| SQL | SQL | RaisinDB SQL engine |

### Function Definition

Functions are stored as `raisin:Function` nodes with metadata and code:

```javascript
// Metadata (stored as node properties)
{
  name: "process-order",
  title: "Process Order",
  description: "Validates and processes incoming orders",
  language: "javascript",          // javascript | starlark | sql
  execution_mode: "async",         // async | sync | both
  entry_file: "index.js:handler",  // format: "filename:function_name"
  version: 1,
  enabled: true
}
```

Function code is stored in a child `raisin:Asset` node. The `entry_file` field specifies which file and function to call (e.g., `"index.js:handler"` calls the `handler` export from `index.js`).

**Execution modes:**
- `async` (default) -- always runs via the job queue
- `sync` -- can run synchronously for web API responses with timeout
- `both` -- caller decides which mode to use

### JavaScript Functions

Functions are written as async handlers with access to the RaisinDB API:

```javascript
async function handler(input) {
    // Read a node
    const node = await raisin.nodes.get("default", input.path);

    // Update properties
    await raisin.nodes.update("default", input.path, {
        properties: { ...node.properties, status: "processed" }
    });

    // Execute SQL queries
    const results = await raisin.sql.query(
        "SELECT * FROM 'default' WHERE node_type = 'article'"
    );

    // Make HTTP requests (allowlisted)
    const response = await fetch("https://api.example.com/data");
    const data = await response.json();

    return { success: true, count: results.length };
}
```

### Execution Context

Every function receives an execution context accessible via `raisin.context`:

```javascript
async function handler(input) {
    const ctx = raisin.context;

    ctx.execution_id    // Unique execution identifier
    ctx.tenant_id       // Tenant identifier
    ctx.repo_id         // Repository identifier
    ctx.branch          // Branch name
    ctx.workspace_id    // Workspace (if applicable)
    ctx.actor           // User ID or "system"
    ctx.trigger_name    // Name of the trigger that invoked this function
    ctx.started_at      // ISO 8601 execution start time
    ctx.input           // Function input parameters
    ctx.metadata        // Custom metadata key-value pairs

    // Event data (for event-triggered functions)
    ctx.event_data.node_id
    ctx.event_data.node_path
    ctx.event_data.node_type
    ctx.event_data.type         // event type (e.g., "created", "updated")

    // HTTP request data (for HTTP-triggered functions)
    ctx.http_request.method
    ctx.http_request.path
    ctx.http_request.path_params    // e.g., { userId: "123" }
    ctx.http_request.query_params
    ctx.http_request.headers
    ctx.http_request.body
}
```

### Complete API Reference

#### Node Operations -- `raisin.nodes.*`

```javascript
// Read
const node = await raisin.nodes.get(workspace, path);
const node = await raisin.nodes.getById(workspace, id);
const children = await raisin.nodes.getChildren(workspace, parentPath, limit?);
const results = await raisin.nodes.query(workspace, queryObject);

// Write
const created = await raisin.nodes.create(workspace, parentPath, nodeData);
await raisin.nodes.update(workspace, path, updateData);
await raisin.nodes.delete(workspace, path);
await raisin.nodes.move(workspace, path, newParentPath);
await raisin.nodes.updateProperty(workspace, path, propertyPath, value);
```

#### SQL Operations -- `raisin.sql.*`

```javascript
// SELECT queries -- returns rows
const rows = await raisin.sql.query(sql, params?);

// INSERT/UPDATE/DELETE -- returns affected row count
const count = await raisin.sql.execute(sql, params?);
```

#### Transaction Operations -- `raisin.tx.*`

Functions can group multiple operations into atomic transactions:

```javascript
const txId = await raisin.tx.begin();

try {
    await raisin.tx.create(txId, workspace, parentPath, nodeData);
    await raisin.tx.update(txId, workspace, path, updateData);
    await raisin.tx.delete(txId, workspace, path);

    // Read within transaction (sees uncommitted changes)
    const node = await raisin.tx.get(txId, workspace, id);
    const node = await raisin.tx.getByPath(txId, workspace, path);
    const children = await raisin.tx.listChildren(txId, workspace, parentPath);

    // Bulk operations
    await raisin.tx.add(txId, workspace, data);
    await raisin.tx.put(txId, workspace, data);
    await raisin.tx.upsert(txId, workspace, data);
    await raisin.tx.createDeep(txId, workspace, parentPath, data, parentNodeType);
    await raisin.tx.upsertDeep(txId, workspace, data, parentNodeType);

    // Move and property updates
    await raisin.tx.move(txId, workspace, path, newParentPath);
    await raisin.tx.updateProperty(txId, workspace, path, propertyPath, value);
    await raisin.tx.deleteById(txId, workspace, id);

    // Set transaction metadata
    await raisin.tx.setActor(txId, "user-123");
    await raisin.tx.setMessage(txId, "Batch import completed");

    await raisin.tx.commit(txId);
} catch (e) {
    await raisin.tx.rollback(txId);
    throw e;
}
```

#### HTTP Operations -- `raisin.http.*` and `fetch`

Functions have access to the W3C Fetch API and a convenience wrapper:

```javascript
// W3C Fetch API (global)
const response = await fetch("https://api.example.com/data", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ key: "value" })
});
const data = await response.json();

// Convenience wrapper
const result = await raisin.http.fetch(url, options);
```

HTTP access requires the function's `network_policy` to explicitly allowlist URLs. See [Network Policy](#network-policy) below.

Global Web APIs are also available: `Request`, `Response`, `Headers`, `AbortController`, `AbortSignal`.

#### AI Operations -- `raisin.ai.*`

```javascript
// LLM completion
const response = await raisin.ai.completion({
    model: "gpt-4o",
    messages: [
        { role: "system", content: "You are a helpful assistant." },
        { role: "user", content: "Summarize this article: " + text }
    ],
    temperature: 0.7,
    max_tokens: 500
});

// List available models for the tenant
const models = await raisin.ai.listModels();

// Get default model for a use case
const model = await raisin.ai.getDefaultModel("chat"); // chat | completion | embedding | agent

// Generate embeddings
const embedding = await raisin.ai.embed({
    input: "Text to embed",
    model: "text-embedding-3-small"
});
```

#### Event Operations -- `raisin.events.*`

```javascript
// Emit custom events (can trigger other functions)
await raisin.events.emit("order.completed", {
    orderId: "order-123",
    total: 99.99
});
```

#### Resource and PDF Operations

```javascript
// Get a resource from a node property
const node = await raisin.nodes.get(workspace, path);
const resource = node.getResource("attachment");

// Binary operations
const base64 = await resource.getBinary();
const dataUrl = await resource.toDataUrl();

// Image operations (via ImageMagick)
const resized = await resource.resize({ width: 800, height: 600 });

// PDF operations
const pageCount = await resource.getPageCount();
const image = await resource.toImage({ page: 1, dpi: 150 });
const extracted = await resource.processDocument({
    extract_text: true,
    ocr: true
});

// Upload a resource
await raisin.resources.addToNode(workspace, nodePath, propertyPath, {
    data: base64Data,
    filename: "report.pdf",
    content_type: "application/pdf"
});

// Direct PDF processing from storage
const result = await raisin.pdf.processFromStorage(storageKey, options);
```

#### Task Operations -- `raisin.tasks.*`

Create human tasks that appear in a user's inbox (used with workflow human-in-the-loop):

```javascript
// Create a task for human review
const task = await raisin.tasks.create({
    title: "Review content",
    assignee: "user-123",
    data: { nodeId: "node-456" }
});

// Update task
await raisin.tasks.update(taskId, { status: "in_progress" });

// Complete task with response
await raisin.tasks.complete(taskId, { approved: true });

// Query tasks
const pending = await raisin.tasks.query({ status: "pending", assignee: "user-123" });
```

#### Function-to-Function Calls -- `raisin.functions.*`

```javascript
// Execute another function (with full tool lifecycle)
const result = await raisin.functions.execute("/functions/validate", args, context);

// Direct function call (simpler, no lifecycle hooks)
const result = await raisin.functions.call("/functions/validate", args);
```

#### Date/Time Utilities -- `raisin.date.*`

```javascript
raisin.date.now()                    // ISO 8601 string
raisin.date.timestamp()              // Unix seconds
raisin.date.timestampMillis()        // Unix milliseconds
raisin.date.parse(dateStr, format?)  // Parse to timestamp
raisin.date.format(timestamp, fmt?)  // Format timestamp
raisin.date.addDays(timestamp, days) // Add days
raisin.date.diffDays(ts1, ts2)       // Difference in days
```

#### Crypto Utilities -- `raisin.crypto.*`

```javascript
const hash = raisin.crypto.md5(data);     // MD5 hex hash
const hash = raisin.crypto.sha256(data);  // SHA-256 hex hash
const id = raisin.crypto.uuid();          // Random UUID v4
```

#### Timer APIs (Global)

```javascript
setTimeout(callback, delayMs);
clearTimeout(timeoutId);
setInterval(callback, intervalMs);
clearInterval(intervalId);
```

#### Console Logging

```javascript
console.log("Info message");    // Info level
console.warn("Warning");        // Warning level
console.error("Error occurred"); // Error level
```

Logs are captured per-execution and can be streamed in real-time via the `LogEmitter`.

### Admin Escalation

Functions can bypass Row-Level Security (RLS) when `allows_admin_escalation` is enabled on the execution context:

```javascript
// Normal access -- respects RLS
const node = await raisin.nodes.get(workspace, path);

// Admin access -- bypasses RLS
const node = await raisin.asAdmin().nodes.get(workspace, path);
const rows = await raisin.asAdmin().sql.query(sql, params);

// All node and SQL operations have admin variants
await raisin.asAdmin().nodes.query(workspace, query);
await raisin.asAdmin().nodes.update(workspace, path, data);
```

### Function Triggers

Functions can be triggered by:

#### Event Triggers

React to node lifecycle events:

```javascript
// Trigger configuration
{
  type: "event",
  events: ["created", "updated", "deleted", "published",
           "unpublished", "moved", "renamed"],
  filters: {
    workspaces: ["default"],
    paths: ["/content/**"],
    node_types: ["raisin:Page", "raisin:Article"],
    property_filters: { status: "draft" }
  }
}
```

Filters use glob patterns for workspaces and paths, allowing fine-grained control over which events trigger the function.

#### HTTP Triggers

Expose functions as API endpoints:

```javascript
// HTTP trigger configuration
{
  type: "http",
  methods: ["GET", "POST"],
  route: "/:userId/orders/:orderId",  // Matchit syntax with path params
  route_mode: "config",               // "config" (pattern) or "script" (function handles routing)
  default_sync: true                   // Run synchronously by default
}
```

Path parameters are available in `raisin.context.http_request.path_params`.

Functions can return custom HTTP responses:

```javascript
async function handler(input) {
    return {
        __http_response: true,
        status: 200,
        headers: { "Content-Type": "application/json" },
        body: { message: "Hello" }
    };
}
```

#### Scheduled Triggers

Run functions on a cron schedule:

```javascript
// Scheduled trigger
{
  type: "schedule",
  cron: "0 * * * *"  // Every hour
}
```

#### SQL Call Triggers

Call functions directly from SQL queries:

```sql
SELECT my_function(arg1, arg2) FROM 'workspace'
```

### Resource Limits

Every function has configurable resource limits:

| Setting | Default | Description |
|---------|---------|-------------|
| `timeout_ms` | 30,000 (30s) | Maximum execution time |
| `max_memory_bytes` | 128 MB | Memory limit |
| `max_instructions` | 100,000,000 | QuickJS instruction limit |
| `max_stack_bytes` | 1 MB | Stack size limit |

**Presets:**

| Preset | Timeout | Memory | Instructions | Stack |
|--------|---------|--------|-------------|-------|
| `minimal` | 5s | 32 MB | 10M | 512 KB |
| `default` | 30s | 128 MB | 100M | 1 MB |
| `generous` | 5min | 512 MB | 1B | 4 MB |

Concurrency is controlled by a global semaphore (configurable via `RAISIN_MAX_CONCURRENT_FUNCTIONS`, default: 15).

### Network Policy

HTTP access is disabled by default. To allow outbound requests, configure a network policy:

```javascript
// Network policy configuration
{
  http_enabled: true,
  allowed_urls: [
    "https://api.example.com/*",         // Entire domain
    "https://*.myservice.com/api/*"      // Wildcard subdomains
  ],
  max_concurrent_requests: 5,    // Default: 5
  request_timeout_ms: 10000,     // Default: 10s
  max_response_size_bytes: 10485760  // Default: 10 MB
}
```

URL patterns use glob syntax. Requests to URLs not matching any pattern are rejected with a `URL_NOT_ALLOWED` error.

### Execution Result

Every function execution produces a result with detailed stats:

```javascript
{
  execution_id: "abc123",
  success: true,
  output: { /* function return value */ },
  http_response: null,  // or custom HTTP response
  stats: {
    duration_ms: 245,
    memory_used_bytes: 4521984,
    instructions_executed: 892341,
    http_requests_made: 1,
    node_operations: 3,
    sql_queries: 1
  },
  logs: [
    { level: "info", message: "Processing started", timestamp: "2025-01-15T10:30:00Z" }
  ],
  error: null  // or { code: "TIMEOUT", message: "...", stack_trace: "..." }
}
```

**Error codes:** `TIMEOUT`, `RUNTIME_ERROR`, `SYNTAX_ERROR`, `URL_NOT_ALLOWED`, `MEMORY_LIMIT`, `INSTRUCTION_LIMIT`.

### Function Flows

Multiple functions can be composed into flows with sequential and parallel execution:

```javascript
// Function flow definition
{
  steps: [
    {
      name: "validate",
      function: "/functions/validate-order",
      input: { orderId: "${input.orderId}" }
    },
    {
      name: "charge",
      function: "/functions/charge-payment",
      depends_on: ["validate"],
      input: { amount: "${steps.validate.output.total}" }
    },
    {
      name: "notify",
      function: "/functions/send-notification",
      depends_on: ["charge"],
      retry: { max_attempts: 3, backoff_ms: 1000 }
    }
  ],
  error_strategy: "fail_fast",  // or "continue"
  timeout_ms: 60000
}
```

Steps are topologically sorted by dependencies and executed in order. Steps without dependencies can run in parallel.

### Function Loading

The `FunctionLoader` resolves function code from storage:

```rust
use raisin_functions::FunctionLoader;

let loader = FunctionLoader::new(storage);
let function = loader.load("tenant1", "repo1", "main", "/functions/my-func").await?;
// function.metadata -- FunctionMetadata
// function.code -- source code string
// function.files -- all module files
```

## Workflow Engine

The `raisin-flow-runtime` crate provides a stateful workflow execution engine for complex, long-running processes.

### Key Features

- **AI agent loops** with tool calls
- **Human-in-the-loop** workflows that pause for user input
- **Decision trees** with branching logic
- **Saga-based compensation** for rollback on failure
- **Pause and resume** across process restarts

### Architecture

The flow runtime uses a hybrid batching model:

- **Synchronous steps** execute continuously without persistence
- **Async operations** (functions, AI calls, human tasks) create jobs and pause execution
- **State is persisted** at async boundaries with optimistic concurrency control

### Flow Definition

Flows are defined as a graph of steps:

```rust
use raisin_flow_runtime::FlowInstance;

let instance = FlowInstance::new(
    "/flows/content-review".to_string(),  // flow_ref
    1,                                     // flow_version
    flow_definition_snapshot,              // serialized flow definition (serde_json::Value)
    input_data,                            // initial input (serde_json::Value)
    "start".to_string(),                   // start_node_id (entry step)
);
```

### Step Handlers

Each step type has a dedicated handler:

| Handler | Purpose |
|---------|---------|
| `FunctionStepHandler` | Execute a serverless function |
| `AiContainerHandler` | Run an AI/LLM operation |
| `AgentStepHandler` | Execute an AI agent loop with tool calling |
| `ChatStepHandler` | Interactive chat with an AI model |
| `HumanTaskHandler` | Pause for human input/approval |
| `DecisionHandler` | Evaluate conditions and branch |
| `ParallelHandler` | Execute multiple steps concurrently |

### Error Handling

Steps define error behavior:

```rust
use raisin_flow_runtime::handlers::{OnErrorBehavior, ErrorClass};

// Options: Retry, Skip, Abort, Compensate
let on_error = OnErrorBehavior::Retry { max_attempts: 3 };
```

### Flow Compilation

The `FlowCompiler` validates and compiles flow definitions before execution:

```rust
use raisin_flow_runtime::FlowCompiler;

let compiler = FlowCompiler::new();
let compiled = compiler.compile(&flow_definition)?;
// compiled.metadata contains step count, validation results, etc.
```

### Integration with Events

Flows can be triggered by system events:

```rust
use raisin_flow_runtime::integration::{FlowTriggerEvent, FlowResumeReason};

// Trigger a flow when a node is created
let trigger = FlowTriggerEvent::NodeCreated {
    node_id: "node-123".to_string(),
    workspace: "default".to_string(),
};

// Resume a paused flow
let resume = FlowResumeReason::HumanTaskCompleted {
    task_id: "task-456".to_string(),
    result: approval_data,
};
```

### Raisin Expression Language (REL)

Flow conditions use the Raisin Expression Language (`raisin-rel`), a simple expression evaluator:

```rust
use raisin_rel::{parse, evaluate, EvalContext, Value};

let expr = parse("input.value > 10 && input.status == 'active'").unwrap();

let mut ctx = EvalContext::new();
ctx.set("input", Value::Object(/* ... */));

let result = evaluate(&expr, &ctx).unwrap();
// result: Value::Boolean(true)
```

REL supports:
- Comparison operators: `==`, `!=`, `>`, `<`, `>=`, `<=`
- Logical operators: `&&`, `||`, `!`
- Property access: `input.value`, `context.user.name`
- Array indexing: `input.tags[0]`
- Functions: `contains()`, `startsWith()`, `endsWith()`
