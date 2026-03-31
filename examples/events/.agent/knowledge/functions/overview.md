# RaisinDB Functions

Server-side functions that run in a sandboxed environment with access to the `raisin.*` API.

## Function Definition (.node.yaml)

Every function lives in its own folder with a `.node.yaml` descriptor and an entry file.

```yaml
node_type: raisin:Function
properties:
  name: my-function
  title: My Function
  description: What this function does
  language: javascript        # javascript | Starlark
  entry_file: index.js:handler  # file:exportedFunction
  execution_mode: async       # async | sync
  enabled: true
  version: 1
  input_schema:
    type: object
    properties:
      name:
        type: string
  output_schema:
    type: object
    properties:
      success:
        type: boolean
  resource_limits:
    timeout_ms: 5000
    max_memory_bytes: 33554432
```

Key fields:
- **language** -- `javascript` or `Starlark` (alias: `python`)
- **entry_file** -- `<filename>:<functionName>`, e.g. `index.js:handler` or `index.py:handler`
- **execution_mode** -- `async` (queued, retryable) or `sync` (immediate, blocking)
- **input_schema / output_schema** -- JSON Schema for validation
- **resource_limits** -- optional timeout and memory cap

## raisin.* API Reference

All functions have access to the global `raisin` object.

### raisin.nodes

| Method | Description |
|--------|-------------|
| `get(workspace, path)` | Get a node by workspace and path |
| `getByPath(workspace, path)` | Alias for get |
| `create(workspace, parentPath, data)` | Create a child node |
| `createDeep(workspace, parentPath, data)` | Create node and any missing ancestors |
| `update(workspace, path, data)` | Update a node's properties |
| `delete(workspace, path)` | Delete a node |
| `move(workspace, fromPath, toPath)` | Move a node to a new parent |
| `beginTransaction()` | Start a transaction (JS only) |

### raisin.sql

| Method | Description |
|--------|-------------|
| `query(sql, params?)` | Execute a parameterized SQL query |

Parameters use `$1`, `$2`, etc. for positional binding. Workspace names with colons must be quoted: `FROM "raisin:access_control"`.

### raisin.http

| Method | Description |
|--------|-------------|
| `get(url, options?)` | HTTP GET request |
| `post(url, body, options?)` | HTTP POST request |
| `put(url, body, options?)` | HTTP PUT request |
| `delete(url, options?)` | HTTP DELETE request |

### raisin.ai

| Method | Description |
|--------|-------------|
| `chat(messages, options?)` | Send messages to an AI model |
| `embed(text)` | Generate an embedding vector |

### raisin.events

| Method | Description |
|--------|-------------|
| `emit(eventType, payload)` | Emit a custom event |

### raisin.date

| Method | Description |
|--------|-------------|
| `now()` | Current ISO-8601 timestamp |
| `parse(dateString)` | Parse a date string |
| `format(timestamp)` | Format a timestamp |
| `timestamp()` | Current Unix timestamp (seconds) |

### raisin.crypto

| Method | Description |
|--------|-------------|
| `hash(data)` | Hash a string |
| `randomUUID()` | Generate a UUID v4 |

### raisin.log / log / console

| Method | Description |
|--------|-------------|
| `debug(msg)` | Debug-level log |
| `info(msg)` | Info-level log |
| `warn(msg)` | Warning-level log |
| `error(msg)` | Error-level log |

In JavaScript use `console.log` / `console.error`. In Starlark use `log.info` / `log.debug` or `print()`.

## Transactions

Transactions group multiple writes into an atomic unit. Available in JavaScript only.

```javascript
const tx = await raisin.nodes.beginTransaction();
try {
  await tx.create(workspace, parentPath, data1);
  await tx.update(workspace, path, data2);
  await tx.commit();
} catch (e) {
  await tx.rollback();
  throw e;
}
```

Transaction objects expose the same methods as `raisin.nodes`: `create`, `createDeep`, `update`, `delete`, `move`, plus `commit()` and `rollback()`.

## SQL Query Patterns

```javascript
// Parameterized query with $1, $2
const rows = await raisin.sql.query(
  `SELECT id, path, properties FROM "raisin:access_control"
   WHERE node_type = 'raisin:User' AND properties->>'email'::String = $1`,
  [email]
);

// Handle result (may be array or { rows: [...] })
const results = Array.isArray(rows) ? rows : (rows?.rows || []);
```

## Language Guides

- See **javascript.md** for JavaScript-specific patterns (async/await, context object)
- See **starlark.md** for Starlark-specific patterns (synchronous API, fail() errors)
