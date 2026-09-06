---
name: raisindb-functions-triggers
description: "Server-side functions and event-driven triggers for RaisinDB. Covers function definitions, the raisin.* runtime API, transactions, trigger filters, event handling, and WebAssembly functions (language: wasm) written in Rust, Go or TypeScript. Use when adding server-side logic."
---

# Functions and Triggers

Functions are handlers stored as nodes inside a RAP package — JavaScript by default, or Starlark, SQL, or a WebAssembly component built in Rust/Go/TypeScript. Triggers watch for events (node changes, schedules, webhooks) and invoke functions when conditions match. Together they form the server-side logic layer of RaisinDB.

**BEFORE writing any server-side function code:**
1. Run `npm install` in the project root — this installs `@raisindb/functions-types` which contains `raisin.d.ts`, the COMPLETE TypeScript API for the function runtime. Read it before writing any code.
2. ONLY use methods defined in `raisin.d.ts` — this is NOT Node.js (no `Buffer`, `fs`, no npm modules). `fetch()` IS available. ES module imports with relative paths ARE supported (`import { foo } from './utils.js'`).

**MANDATORY**: After creating or modifying ANY `.yaml`, `.node.yaml`, or `.js` file in `package/`, immediately run:

    npm run validate

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
| `language` | yes | `javascript`, `starlark`, `sql`, or `wasm` (see WebAssembly below) |
| `entry_file` | yes | `filename:functionName` -- e.g. `index.js:handler` |
| `execution_mode` | yes | `async` (queued, retryable) or `sync` (immediate, blocking) |
| `enabled` | yes | `true` or `false` |
| `input_schema` | no | JSON Schema for input validation |
| `output_schema` | no | JSON Schema for output validation |
| `resource_limits` | no | `timeout_ms` and `max_memory_bytes` |

---

## Function Implementation

The exported function name must match `entry_file`.

> ⚠️ **The runtime is NOT CommonJS / ES modules at the top level.** Do **not**
> end the file with `module.exports = { handler }` (or `exports.handler = …` /
> `export default`) — they throw `module is not defined` at eval time and the run
> fails (shows as "JavaScript syntax error" in the execution log). Just declare
> the top-level function whose name matches `entry_file` (`index.js:myHandler` →
> `async function myHandler(context) { … }`); the runtime calls it directly.
> (Relative `import { x } from './utils.js'` IS still fine.)

> ⚠️ **Updating a function's CODE pushes via `sync`, not `deploy --install`.** A
> reinstall upserts *schema* but leaves existing *content* (which includes the
> function `.js`) untouched — so an edited handler won't take effect after
> `deploy --install` alone. Use `raisindb sync … --push` (or `--watch`) to push
> the new code live, then the next trigger run uses it.

### Trigger-Invoked (the event is wrapped in `flow_input`)

A `node_event` trigger does **NOT** hand your function a flat `{ path, node }`. The
event is wrapped in `flow_input`. The full payload:

```jsonc
{
  "flow_input": {
    "event": { "type": "Created|Updated|Deleted|…", "node_id": "…", "node_type": "ws:Type", "node_path": "/foo" },
    "node":  { "id": "…", "name": "foo", "path": "/foo", "node_type": "…", "properties": { … }, … },
    "workspace": "myworkspace"
  },
  "previous_results": { … }   // present even for a single function_path trigger
}
```

```javascript
async function handleTaskCompleted(context) {
  const { event, node, workspace } = context.flow_input;
  // event.type = "Created" | "Updated" | "Deleted" | ...
  // event.node_path, event.node_id, event.node_type

  // The CHANGED NODE (incl. properties) is already in the payload — use it directly.
  // This also avoids an index lag where a just-Created node isn't fetchable yet:
  const props = node?.properties ?? (await raisin.nodes.get(workspace, event.node_path))?.properties;
  if (!props) return { success: false, error: 'Not found' };

  // ... process ...
  return { success: true };
}
```

> ⚠️ **#1 trigger mistake: reading the node/path from the wrong place.** From a
> trigger the path is **only** at `context.flow_input.event.node_path` and the node
> at `context.flow_input.node` — *not* `context.path`, `context.node`, `context.node_path`,
> or any flat field (those are the **directly-invoked** shape below). Symptom: the
> trigger fires and the function returns *success* in the admin console's
> trigger-evaluation view, but does nothing / returns `not_found` / reads `undefined`,
> because your handler looked at the top level. Write handlers that accept BOTH shapes
> if the same function is also invoked manually:
> `const path = input.equipment_path || input.flow_input?.event?.node_path || input.flow_input?.node?.path;`

### Directly-Invoked / AI Tool (receives a flat `input`)

A manual `db.functions().invokeSync(path, { ... })` (or AI-tool call) passes your
object **flat** — no `flow_input` wrapper.

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

## WebAssembly Functions (`language: wasm`)

A function can also be a **WebAssembly component** built in Rust, Go or
TypeScript and uploaded as `main.wasm`. Reach for it when you want a real
toolchain (types, a package ecosystem, a native test runner) or CPU-bound work.
Do **not** reach for it to edit code quickly: a wasm function has NO source on
the server — the artifact is the code, and every change is a local rebuild plus
an upload.

```yaml
node_type: raisin:Function
properties:
  name: greet
  language: wasm
  entry_file: main.wasm          # -> the guest handler named "default"
  execution_mode: both
  enabled: true
  resource_limits: { timeout_ms: 5000, max_memory_bytes: 67108864 }
```

**`entry_file` is name-routed.** The component exports exactly one WIT function,
`handler(name, input)`, and the suffix of `entry_file` picks the handler:

| `entry_file` | handler | note |
|---|---|---|
| `main.wasm` | `default` | the sibling artifact |
| `main.wasm:shout` | `shout` | same artifact, second handler |
| `../greet/main.wasm:shout` | `shout` | **another node's** artifact |

So ONE artifact backs N Function nodes — which is how a package of TypeScript
functions ships 12 MB instead of 200. The path must stay inside the functions
workspace; one that escapes it is refused. The host never validates the handler
NAME: an unknown one comes back as an error listing what the guest registered.

**Package layout.** Guest source lives OUTSIDE `content/` — `sync` maps every
non-YAML file under `content/` to a node, so a `Cargo.toml` there uploads as an
asset:

```
package/content/functions/lib/<ns>/<name>/.node.yaml   language: wasm
package/content/functions/lib/<ns>/<name>/main.wasm    the only thing that ships
package/wasm/<ns>/<name>/raisin.build.yaml             { lang, node_dir, artifact, command }
package/wasm/<ns>/<name>/{Cargo.toml|go.mod|package.json, src/…}
package/.rapignore                                     wasm/
```

**Commands** (all offline; they never talk to a server):

```bash
raisindb create function greet --lang rust|go|ts [--ns demo]
raisindb create function greet-shout --into greet --handler shout   # share the artifact
raisindb function build [path] [--all] [--watch] [--debug]
raisindb function doctor [path] [--json] [--strict]
raisindb deploy ./package --repo myapp --install
```

`function build` copies the artifact into the Function node and lists every node
whose `entry_file` resolves to it. `function doctor` checks toolchains, artifact
size against the 32 MiB server cap, `entry_file` resolution, and that the
handler name a node asks for is actually registered by the project.
`raisindb function run` / `test --server` do not exist yet — test natively
(`cargo test`, `go test ./...`, `vitest run`; every SDK ships a mock host) and
invoke a deployed function the normal way.

**The API is the same `raisin.*` surface**, reached through one generic gateway;
the typed wrappers in each SDK are generated from the server's registry. The
sandbox is not the same, though: no `wasi:sockets`, no `wasi:http`, no
filesystem, no timers. Egress is `raisin.http.*` only, gated by
`network_policy`. In TypeScript, `fetch` / `setTimeout` / `Resource.resize` are
NOT available — keep such a function in `javascript`.

Docs: `docs/website/docs/functions/wasm-functions.md` (and the per-language
guides beside it); the ABI contract is `docs/guides/wasm-function-abi.md`.

---

## The `raisin.*` Runtime API

### raisin.nodes

| Method | Description |
|--------|-------------|
| `get(workspace, path)` | Get node by path |
| `getById(workspace, id)` | Get node by ID |
| `create(workspace, parentPath, data)` | Create a child node |
| `createDeep(workspace, parentPath, data, parentNodeType?)` | Create node + missing ancestors (default `raisin:Folder`) |
| `upsertDeep(workspace, data, parentNodeType?)` | Create-or-update by path + missing ancestors |
| `update(workspace, path, data)` | Update node properties |
| `delete(workspace, path)` | Delete a node |
| `move(workspace, fromPath, toPath)` | Move a node |
| `beginTransaction()` | Start a transaction |

The `data` object: `{ name, node_type, properties: { ... } }`

### Node Resource API (Binary Files)

The function runtime has a built-in Resource API for processing binary files (images, PDFs). There is NO automatic thumbnail generation — you must call these methods yourself. There are no npm modules, no Node.js globals, no external services. Only the API below exists.

#### TypeScript Definitions

Install `@raisindb/functions-types` for full IDE autocomplete in function projects:

    npm install -D @raisindb/functions-types

Key interfaces (see package for complete definitions):

```typescript
// Returned by raisin.nodes.get(workspace, path) — has resource helper methods
interface RaisinNode {
  id: string; path: string; name: string; node_type: string;
  properties: Record<string, any>;
  getResource(propertyPath: string): Resource | null;    // e.g., './file'
  addResource(propertyPath: string, data: Resource | { base64: string; mimeType: string }): Promise<any>;
}

// Returned by node.getResource('./file') — has built-in resize/PDF processing
interface Resource {
  readonly mimeType: string;   // "image/jpeg", "application/pdf", etc.
  readonly size: number;
  readonly name: string;
  resize(opts: { maxWidth?: number; format?: 'jpeg'|'png'|'webp'; quality?: number }): Promise<Resource>;
  processDocument(opts?: { ocr?: boolean; generateThumbnail?: boolean; thumbnailWidth?: number }): Promise<DocumentResult>;
  toImage(opts?: { page?: number; maxWidth?: number; format?: string }): Promise<Resource>;
  getBinary(): Promise<string>;  // base64
}
```

#### The ONE correct way to create a thumbnail

```javascript
// Step 1: Get the node
const node = await raisin.nodes.get(workspace, event.node_path);

// Step 2: Get the Resource handle for the uploaded file
const resource = node.getResource('./file');

// Step 3: Call resize() — this runs server-side image processing
const thumbnail = await resource.resize({
  maxWidth: 200,
  format: 'jpeg',
  quality: 80,
});

// Step 4: Store the resized image as a Resource on the node
await node.addResource('./thumbnail', thumbnail);
```

For PDFs:

```javascript
const resource = node.getResource('./file');
const result = await resource.processDocument({
  generateThumbnail: true,
  thumbnailWidth: 200,
});
if (result.thumbnail) {
  await node.addResource('./thumbnail', result.thumbnail);
}
```

#### What IS available (beyond raisin.*)

- `fetch()`, `Request`, `Response`, `Headers` — W3C Fetch API (built-in, no import needed)
- `setTimeout`, `clearTimeout`, `setInterval`, `clearInterval` — timers
- `import { foo } from './utils.js'` — ES module imports with relative paths
- `console.log/debug/warn/error` — logging

#### FORBIDDEN — these produce runtime errors

```javascript
// ERROR: npm modules not available (no require())
const sharp = require('sharp');

// ERROR: "Buffer is not defined" (not Node.js)
const buf = Buffer.from(data);

// ERROR: "fs is not defined" (no filesystem access)
const data = fs.readFileSync(path);

// WRONG — does not resize, just copies the reference
await raisin.nodes.update(workspace, path, {
  properties: { thumbnail: node.properties.file }
});

// WRONG — there is NO built-in auto-processing or "AssetProcessing job"
// Thumbnails do NOT appear automatically. You must call resource.resize().
```

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
| `raisin.crypto.uuid()` | Generate UUID v4 (there is no `randomUUID`) |
| `raisin.crypto.randomBytes(n)` | `n` CSPRNG bytes as base64; `n` in `1..=64` |
| `raisin.crypto.hash(input, alg?)` | Lowercase hex digest; `alg` = `"sha256"` (default) or `"sha512"`. No MD5/SHA-1 |
| `raisin.crypto.generateKeyPair(alg?)` | `{ alg, publicJwk, privateJwk }`; `alg` = `"ES256"` (P-256) only |
| `raisin.crypto.signJwt(claims, privateJwk, opts?)` | Compact JWS. `opts = { alg?, kid?, expiresInSec? }`. Signature is JOSE `r\|\|s`, base64url, unpadded |
| `raisin.crypto.verifyJwt(token, opts)` | `opts = { jwks_url, issuer?, audience?, algorithms? }` -> `{ valid, claims?, error? }`; `jwks_url` must pass `network_policy` |

All crypto bindings are async -- `await` them. Keep a `privateJwk` in the secret
store (`raisin.secrets.get`), never in a node property or a log line.

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

## Precomputed Views Pattern

Instead of running expensive queries on every page load, use triggers to **precompute results and store them as nodes**. The frontend fetches the precomputed node with a simple path lookup.

**When to use**: overview lists, dashboards, feeds, statistics, tag clouds, "latest articles" — any data read frequently but changed infrequently.

**Example**: rebuild a "latest articles" summary whenever an article is created or updated.

**Trigger** (`triggers/on-article-change/.node.yaml`):

```yaml
node_type: raisin:Trigger
properties:
  title: Rebuild Latest Articles
  name: on-article-change
  enabled: true
  trigger_type: node_event
  config:
    event_kinds: [Created, Updated, Deleted]
  filters:
    workspaces: [content]
    node_types: [myapp:Article]
    property_filters:
      status: published
  priority: 5
  max_retries: 3
  function_path: /lib/myapp/rebuild-latest
```

**Function** (`lib/myapp/rebuild-latest/index.js`):

```javascript
async function handler(context) {
  const { workspace } = context.flow_input;

  // Run the expensive query ONCE, server-side
  const articles = await raisin.sql.query(
    `SELECT id, path, name, properties->>'title'::String AS title,
            properties->>'excerpt'::String AS excerpt,
            properties->>'publishing_date'::String AS date
     FROM ${workspace}
     WHERE node_type = 'myapp:Article'
       AND properties->>'status'::String = 'published'
     ORDER BY properties->>'publishing_date' DESC
     LIMIT 10`,
    []
  );
  const rows = Array.isArray(articles) ? articles : (articles?.rows || []);

  // Store the result as a node — frontend reads this instead of querying
  await raisin.sql.query(
    `UPDATE ${workspace} SET properties = properties || $1::jsonb WHERE path = $2`,
    [JSON.stringify({ articles: rows, rebuilt_at: new Date().toISOString() }),
     `/${workspace}/computed/latest-articles`]
  );

  return { success: true, count: rows.length };
}
// No module.exports — the runtime finds `handler` from entry_file directly.
```

**Frontend**: simple single-node fetch instead of a complex query:

```typescript
const latest = await queryOne(`
  SELECT properties FROM content WHERE path = '/content/computed/latest-articles'
`);
// latest.properties.articles = [{ title, excerpt, date, path }, ...]
```

This pattern keeps page loads fast and moves computation to write-time.

---

## Validation

**MANDATORY** — run after every YAML or JS change in `package/`:

    npm run validate

Validates that all listed functions/triggers have matching folders with `.node.yaml`, `entry_file` references exist, `function_path`/`flow_path` point to registered targets, and YAML syntax is correct. Fix all errors before proceeding.
