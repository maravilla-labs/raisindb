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
    const response = await raisin.http.get("https://api.example.com/data");

    return { success: true, count: results.length };
}
```

### Sandboxed Execution

Functions run in a sandboxed environment with configurable resource limits:

```rust
use raisin_functions::runtime::{Sandbox, SandboxConfig};

let config = SandboxConfig {
    max_memory_bytes: 128 * 1024 * 1024,  // 128 MB
    max_execution_time_ms: 30_000,         // 30 seconds
    max_concurrent: 15,                     // concurrent executions
    // ...
};
```

Concurrency is controlled by a global semaphore (configurable via `RAISIN_MAX_CONCURRENT_FUNCTIONS` environment variable, default: 15).

### RaisinDB API Access

Functions have access to:

| API | Operations |
|-----|------------|
| `raisin.nodes` | `get`, `getById`, `getChildren`, `create`, `update`, `delete`, `query` |
| `raisin.sql` | `query`, `execute` |
| `raisin.http` | `get`, `post`, `put`, `delete` (allowlisted endpoints) |
| `raisin.events` | `emit` (publish events) |

### Function Triggers

Functions can be triggered by:

- **Events** -- react to node changes, workspace updates, etc.
- **HTTP** -- exposed as API endpoints
- **Schedule** -- cron-based execution
- **SQL** -- called as SQL functions in queries

### Function Loading

The `FunctionLoader` resolves function code from storage:

```rust
use raisin_functions::FunctionLoader;

let loader = FunctionLoader::new(storage);
let function = loader.load("tenant1", "repo1", "main", "/functions/my-func").await?;
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
