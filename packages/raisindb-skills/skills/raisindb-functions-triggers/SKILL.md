---
name: raisindb-functions-triggers
description: "Server-side JavaScript functions and event-driven triggers for RaisinDB. Covers function definitions, the raisin.* runtime API, transactions, trigger filters, and event handling. Use when adding server-side logic."
---

# Functions and Triggers

Functions are JavaScript handlers stored as nodes inside a RAP package. Triggers watch for events (node changes, schedules, webhooks) and invoke functions when conditions match. Together they form the server-side logic layer of RaisinDB.

**MANDATORY**: After creating or modifying ANY `.yaml`, `.node.yaml`, or `.js` file in `package/`, immediately run:

    raisindb package create ./package --check

## File Organization

```
content/functions/
├── lib/{namespace}/{function-name}/
│   ├── .node.yaml          # raisin:Function definition
│   └── index.js            # JavaScript implementation
└── triggers/{trigger-name}/
    └── .node.yaml           # raisin:Trigger definition
```

---

## Function Definition

Every function has a `.node.yaml` with `node_type: raisin:Function`.

### Minimal Example

```yaml
node_type: raisin:Function
properties:
  name: handle-read-receipt
  title: Handle Read Receipt
  description: Updates sender's message with read status
  execution_mode: async
  enabled: true
  language: javascript
  entry_file: index.js:handleReadReceipt
```

### With Input/Output Schemas

```yaml
node_type: raisin:Function
properties:
  name: kanban-move-card
  title: Move Kanban Card
  description: Move a card between columns or boards.
  execution_mode: async
  enabled: true
  language: javascript
  entry_file: index.js:handleMoveCard
  version: 1
  input_schema:
    type: object
    required: [board_path, card_uuid, to_column_id]
    properties:
      board_path: { type: string, description: "Full path to the board" }
      card_uuid: { type: string }
      to_column_id: { type: string }
      to_position: { type: integer, description: "0-based index. Omit to append." }
  output_schema:
    type: object
    properties:
      success: { type: boolean }
      error: { type: string }
```

### Key Fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Unique function identifier |
| `title` | yes | Human-readable name |
| `language` | yes | `javascript` (or `Starlark`) |
| `entry_file` | yes | `filename:functionName` -- e.g. `index.js:handler` |
| `execution_mode` | yes | `async` (queued, retryable) or `sync` (immediate, blocking) |
| `enabled` | yes | `true` or `false` |
| `input_schema` | no | JSON Schema for input validation |
| `output_schema` | no | JSON Schema for output validation |
| `resource_limits` | no | `timeout_ms` and `max_memory_bytes` |

---

## Function Implementation

The exported function name must match `entry_file`.

### Trigger-Invoked (receives `context`)

```javascript
async function handleTaskCompleted(context) {
  const { event, workspace } = context.flow_input;
  // event.type = "Created" | "Updated" | "Deleted" | ...
  // event.node_path, event.node_id, event.node_type

  const node = await raisin.nodes.get(workspace, event.node_path);
  if (!node) return { success: false, error: 'Not found' };

  // ... process ...
  return { success: true };
}
```

### Directly-Invoked / AI Tool (receives `input`)

```javascript
async function handleMoveCard(input) {
  const { board_path, card_uuid, to_column_id } = input;

  const result = await raisin.sql.query(
    `SELECT * FROM launchpad WHERE path = $1`, [board_path]
  );
  const rows = Array.isArray(result) ? result : (result?.rows || []);
  return { success: true };
}
```

---

## The `raisin.*` Runtime API

### raisin.nodes

| Method | Description |
|--------|-------------|
| `get(workspace, path)` | Get node by path |
| `getById(workspace, id)` | Get node by ID |
| `create(workspace, parentPath, data)` | Create a child node |
| `createDeep(workspace, parentPath, data)` | Create node + missing ancestors |
| `update(workspace, path, data)` | Update node properties |
| `delete(workspace, path)` | Delete a node |
| `move(workspace, fromPath, toPath)` | Move a node |
| `beginTransaction()` | Start a transaction |

The `data` object: `{ name, node_type, properties: { ... } }`

### raisin.sql

```javascript
// Parameters use $1, $2. Quote workspace names with colons.
// Cast JSON keys: properties->>'email'::String = $1
const result = await raisin.sql.query(
  `SELECT * FROM "raisin:access_control" WHERE properties->>'email'::String = $1`,
  [email]
);
// Results may be array or { rows: [...] } -- handle both:
const rows = Array.isArray(result) ? result : (result?.rows || []);
```

### raisin.http

`get(url, opts?)`, `post(url, body, opts?)`, `put(url, body, opts?)`, `delete(url, opts?)`

### raisin.ai

| Method | Description |
|--------|-------------|
| `completion({ model, messages, response_format? })` | Chat completion |
| `embed({ model, input, input_type? })` | Generate embeddings |

### raisin.events / raisin.functions

| Method | Description |
|--------|-------------|
| `raisin.events.emit(eventType, payload)` | Emit a custom event |
| `raisin.functions.execute(functionPath, args)` | Call another function |

### raisin.date / raisin.crypto

| Method | Description |
|--------|-------------|
| `raisin.date.now()` | Current ISO-8601 timestamp |
| `raisin.date.parse(str)` / `format(ts)` | Parse / format dates |
| `raisin.date.timestamp()` | Unix timestamp (seconds) |
| `raisin.crypto.hash(data)` | Hash a string |
| `raisin.crypto.randomUUID()` | Generate UUID v4 |

### Logging

`console.log()`, `console.error()`, `console.warn()` -- captured in server logs.

---

## Transactions

Transactions group multiple writes into an atomic unit. The transaction object exposes the same methods as `raisin.nodes` plus `commit()` and `rollback()`.

```javascript
async function handleTaskCompleted(context) {
  const { event, workspace } = context.flow_input;
  const ACCESS_CONTROL = 'raisin:access_control';

  const message = await raisin.nodes.get(workspace, event.node_path);
  if (!message) return { success: false, error: 'Message not found' };

  const { body } = message.properties;
  let tx = null;
  let txFinalized = false;

  try {
    tx = await raisin.nodes.beginTransaction();

    const convName = `task-done-${Date.now()}`;
    const aiChatsPath = `${body.sender_path}/ai-chats`;

    // createDeep creates node + missing ancestor folders
    await tx.createDeep(ACCESS_CONTROL, aiChatsPath, {
      name: convName,
      node_type: 'raisin:AIConversation',
      properties: { title: `Task Complete: ${body.card_title}`, status: 'active' },
    });

    await tx.createDeep(ACCESS_CONTROL, `${aiChatsPath}/${convName}`, {
      name: `msg-${Date.now()}`,
      node_type: 'raisin:AIMessage',
      properties: { role: 'assistant', content: 'Task completed!' },
    });

    await tx.delete(workspace, event.node_path);
    await tx.commit();
    txFinalized = true;
    return { success: true };

  } catch (err) {
    if (tx && !txFinalized) {
      try { await tx.rollback(); } catch (e) { /* log */ }
    }
    return { success: false, error: err.message };
  }
}
```

### Transaction Methods

`tx.get(ws, id)`, `tx.getByPath(ws, path)`, `tx.create(...)`, `tx.createDeep(...)`, `tx.update(...)`, `tx.delete(...)`, `tx.move(...)`, `tx.commit()`, `tx.rollback()`

---

## Trigger Definition

Every trigger has a `.node.yaml` with `node_type: raisin:Trigger`.

### Trigger Types

| Type | Description |
|------|-------------|
| `node_event` | Fires on node Created, Updated, Deleted, Published, Unpublished, Moved, Renamed |
| `schedule` | Fires on a cron schedule |
| `http` | Fires on inbound HTTP webhook |

### Example: Asset Processing

```yaml
node_type: raisin:Trigger
properties:
  title: AI Asset Processing
  name: launchpad-asset-ai-processing
  description: Triggered when an asset upload completes.
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Created
  filters:
    workspaces:
      - launchpad
    node_types:
      - raisin:Asset
    paths:
      - "**"
    property_filters:
      "file.metadata.storage_key":
        $exists: true
  priority: 10
  max_retries: 3
  function_path: /lib/launchpad/process-asset
```

### Filters Reference

All filters are optional. When multiple are specified, they are ANDed.

| Filter | Description |
|--------|-------------|
| `workspaces` | Workspace names to watch |
| `paths` | Glob patterns (`*` = one segment, `**` = any depth) |
| `node_types` | Exact node type names to match |
| `property_filters` | Match on property values (see operators below) |

### Property Filter Operators

```yaml
property_filters:
  status: published                        # exact match
  "file.metadata.storage_key":             # nested dot-path
    $exists: true                          # existence check
  _source: { $ne: flow }                   # not-equal
  role: { $eq: user }                      # explicit equal
  message_type: { $in: [chat, dm] }        # in-list
  is_system_generated: { $ne: true }       # boolean not-equal
```

### function_path vs flow_path

```yaml
function_path: /lib/launchpad/handle-read-receipt   # call a function
flow_path: /flows/task-completed-chat                # start a flow
```

### Priority and Retries

```yaml
priority: 10       # Higher = runs first (default: 10)
max_retries: 3     # Retry on failure (default: 3)
```

---

## Registering in manifest.yaml

Functions and triggers must be listed in `manifest.yaml` under `provides`:

```yaml
provides:
  functions:
    - /lib/launchpad/handle-read-receipt
    - /lib/launchpad/kanban-move-card
    - /lib/launchpad/process-asset
    - /lib/launchpad/handle-task-completed
  triggers:
    - /triggers/on-read-receipt
    - /triggers/on-asset-ready
    - /triggers/on-task-completed
```

Paths match the folder path under `content/functions/`.

---

## Complete Example

Task completion flow: trigger fires on outbox message, function creates an AI chat via transaction.

**Trigger** (`triggers/on-task-completed/.node.yaml`):

```yaml
node_type: raisin:Trigger
properties:
  title: On Task Completed
  enabled: true
  trigger_type: node_event
  config:
    event_kinds: [Created]
  filters:
    workspaces: [raisin:access_control]
    paths: ["**/users/**/outbox/*"]
    node_types: [raisin:Message]
    property_filters:
      message_type: task_completed
      status: pending
  priority: 10
  max_retries: 3
  function_path: /lib/launchpad/handle-task-completed
```

**Function** (`lib/launchpad/handle-task-completed/.node.yaml`):

```yaml
node_type: raisin:Function
properties:
  name: handle-task-completed
  title: Handle Task Completed
  execution_mode: async
  enabled: true
  language: javascript
  entry_file: index.js:handleTaskCompleted
```

**Implementation** (`index.js`) -- see the Transactions section above for the full handler code with `beginTransaction`, `createDeep`, `commit`/`rollback`.

**Register** in `manifest.yaml`:

```yaml
provides:
  functions: [/lib/launchpad/handle-task-completed]
  triggers: [/triggers/on-task-completed]
```

---

## Validation

**MANDATORY** — run after every YAML or JS change in `package/`:

    raisindb package create ./package --check

Validates that all listed functions/triggers have matching folders with `.node.yaml`, `entry_file` references exist, `function_path`/`flow_path` point to registered targets, and YAML syntax is correct. Fix all errors before proceeding.
