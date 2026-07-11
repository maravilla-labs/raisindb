# Building a Virtual-Node Adapter

This walkthrough builds a virtual-node adapter from scratch: scaffold the package,
implement the required operations, test locally, install it, wire up an Integration
and a Mount, and watch it sync.

Two shipped adapters are the worked references throughout — read them alongside
this guide:

- **`google-drive-adapter`** (`builtin-packages/google-drive-adapter/`) — a
  persistent storage mount with a real delta API (`supports_changes: true`).
- **`imap-adapter`** (`builtin-packages/imap-adapter/`) — the ephemeral mailbox
  pattern, and an object lesson in the sandbox network constraint (see §9).

The full, frozen API you are implementing is
[`docs/reference/virtual-node-adapters.md`](../reference/virtual-node-adapters.md);
this guide is the "how", that document is the "what".

---

## 1. What an adapter is

An adapter is a `raisin:Function` that translates the engine's normalized
operations into calls against one external system. The engine invokes it
**directly** (no trigger, no nested job), hands it a decrypted `access_token` just
before the call, and materializes whatever it returns into nodes under a mount
path. You never touch nodes, cursors, transactions, or credentials-at-rest — the
engine owns all of that.

A complete adapter package ships three things:

| Path | Workspace | What |
|------|-----------|------|
| `content/functions/adapters/<name>/` | `functions` | the adapter function (`index.js` + `.node.yaml`) |
| `content/functions/mappers/<name>-default/` | `functions` | *(optional)* a default mapping function |
| `content/_raisin__system/integrations/<name>/` | `raisin:system` | *(optional)* a pre-configured `raisin:Integration` template |

Note the on-disk folder `_raisin__system` — that is how the `raisin:system`
workspace id (which contains a `:`) is encoded as a package content directory.

---

## 2. Scaffold the package

Use the CLI generator — it emits a **working, installable** package skeleton whose
`capabilities` operation passes with no edits:

```
raisindb create adapter <name>
```

- `<name>` is a lower-kebab-case slug (`dropbox`, `box-drive`); the command rejects
  anything else.
- Options: `--dir <path>` (defaults to `./<name>-adapter`), `--provider <slug>`
  (the integration's `provider_type`; defaults to `<name>`), `--description <text>`.
- It refuses to overwrite an existing `manifest.yaml`.

What it generates (`packages/raisindb-cli/src/commands/create.ts` +
`src/templates/adapter.ts`):

```
<name>-adapter/
├── manifest.yaml                       # category: integrations, builtin: false
├── README.md
└── content/
    ├── functions/
    │   ├── adapters/<name>/            # stub adapter: capabilities + list implemented
    │   │   ├── index.js
    │   │   └── .node.yaml
    │   └── mappers/<name>-default/
    │       ├── index.js
    │       └── .node.yaml
    └── _raisin__system/
        └── integrations/<name>/       # disabled raisin:Integration template, NO client secret
            └── .node.yaml
```

The generated adapter already implements `capabilities` and a stub `list`, so it
installs and passes a `capabilities` invocation immediately — you then fill in the
provider I/O. On completion the command prints the next steps: implement the TODOs
in `content/functions/adapters/<name>/index.js`, set `auth_url` / `token_url` /
`scopes` in the integration template, then deploy:

```
cd <name>-adapter && raisindb package deploy . --repo <repo> --install
```

Note the on-disk folder `_raisin__system` — that is how the `raisin:system`
workspace id (which contains a `:`) is encoded as a package content directory.

### `manifest.yaml`

The generator writes this for you; the load-bearing fields (also visible in
`builtin-packages/google-drive-adapter/manifest.yaml`):

- `category: integrations` — the **discovery contract**. The admin console's
  Connectors page and any tooling find adapters by querying installed packages where
  `properties->>'category'::String = 'integrations'`.
- `provides.functions` — the adapter + mapper paths (relative to the `functions`
  workspace, e.g. `/adapters/my-adapter`).
- `provides.content` — any pre-shipped nodes, e.g.
  `raisin:system/integrations/my-adapter`.
- `builtin: true` only if you want it auto-installed on repo creation. Third-party
  adapters ship as installable `.rap` files and omit this.
- `sync.filters` / `workspace_patches` — copy the Drive package's; they declare
  which paths the package owns and the default folder type for auto-created
  parents.

### Adapter `.node.yaml`

Copy `content/functions/adapters/google-drive/.node.yaml`. Critical fields:

- `node_type: raisin:Function`, `language: javascript`,
  `entry_file: index.js:handler`.
- `resource_limits.timeout_ms` — raise it well above the 30s default (Drive uses
  `120000`); a sync page can make several provider calls.
- **`network_policy.allowed_urls`** — pin outbound HTTP to your provider's hosts
  only. This bounds the blast radius of privileged adapter code (adapters run with
  a system context — see the [internals doc](../concepts/virtual-nodes-internals.md#6-security-model)).

---

## 3. Implement the handler

The entrypoint takes **one** argument and dispatches on `operation`:

```javascript
function handler(input) {
  var operation = input.operation;
  var params = input.params || {};
  var credential = input.credential;   // { access_token, account_id, provider_type } — NO refresh_token
  var mount = input.mount || {};       // { mount_id, remote_root, mount_path, sync_config }

  switch (operation) {
    case "capabilities": return opCapabilities();
    case "list":         return opList(credential, mount, params);
    case "get":          return opGet(credential, mount, params);
    case "get_changes":  return opGetChanges(credential, mount, params);
    // get_content, create, update, delete as your capabilities advertise
    default: throw new Error("Unsupported operation: " + operation);
  }
}
```

### Minimum viable adapter

To sync **read-only** you need only two operations:

1. **`capabilities`** — cheap, side-effect-free self-description. Set
   `supports_changes: false` if your provider has no delta API; the engine will
   then full-reconcile via `list` every time. **Report it honestly.** The sync loop
   does not call `capabilities` (it reads what it needs from the mount config), but
   the **"Test connection"** endpoint does, and the admin console **caches the
   result on the Integration node and drives connector-form visibility from it** —
   claiming `can_write: true` on a read-only provider shows the operator a
   write/writeback form your adapter can't honor, not a sync error. Only advertise
   `supports_changes`, `can_write`, `can_create_folders`, etc. that you actually
   implement.
2. **`list`** — enumerate the immediate children of `params.folder_id`
   (default `mount.remote_root`), returning
   `{ items: ExternalItem[], next_cursor }`. The engine recurses into folders
   itself.

Add **`get_changes`** to get incremental sync (and set `supports_changes: true`);
add `get`, `create`, `update`, `delete`, `get_content` for richer / write
scenarios.

### Building an `ExternalItem`

Every item you return must have a stable `external_id`, a `name`, and
`is_folder`. Two fields carry real semantic weight — study Drive's `toExternalItem`
for both:

- **`etag`** must be **stable when the item is unchanged** and change when it
  changes. The engine's *skip-write* compares it to the stored `__etag` and writes
  nothing when they match — this is what prevents revision churn and trigger
  storms. Drive uses the file's monotonic `version` counter.
- **`external_id`** must be **stable across renames/moves**. The engine matches on
  it to update the existing node in place rather than duplicate it.

### Errors — throw with a `code`

Never swallow an auth failure into an empty result — an empty `list` /
`get_changes` reads as "everything was deleted" and the reconcile removes your
mount's nodes. Throw instead:

```javascript
function coded(message, code) { var e = new Error(message); e.code = code; return e; }

if (resp.status === 401) throw coded("token rejected", "auth_expired");   // engine pauses the mount, refreshes
if (resp.status === 429) throw coded("throttled", "rate_limited");        // engine backs off + retries
// etag mismatch on a write:   throw coded("stale", "conflict");
// anything else: a plain Error → treated as transient, retried with backoff
```

**Do not refresh tokens yourself.** You never receive the refresh token; on
`auth_expired` the engine handles refresh/reconnect.

### Performance rules (mandatory)

- **Never call `raisin.functions.call` in a per-item path.** A nested call blocks a
  worker up to 5 minutes with no depth guard; per-item it exhausts the pool. Keep
  provider I/O and normalization inline.
- Keep `capabilities` network-free (it is polled), page efficiently, and make your
  cursors durable/resumable (the engine may re-run a page after a crash).

---

## 4. The mapping function (optional)

If you ship none, the engine's **built-in Rust default** maps folders to
`raisin:Folder` and everything else to **`raisin:Node`** (title + a `meta` object).
To emit richer node types, ship a mapper and reference it from the mount's
`mapping_function`. It is called once per item and must be **pure and fast**:

```javascript
function handler(input) {
  var item = input.external_item;              // ExternalItem
  if (!item || !item.external_id) return null; // return null → skip this item
  if (item.is_folder) return { node_type: "raisin:Folder", name: item.name, properties: { title: item.name } };
  return {
    node_type: "raisin:Asset",
    name: item.name,
    properties: { title: item.name, mimeType: item.mime_type, web_url: item.web_url },
  };
}
```

Do **not** set `__`-prefixed properties — the engine stamps
`__virtual/__mount_id/__external_id/__etag/__synced_at` on top of whatever you
return. (See the Drive mapper for a fuller example mapping Docs/Sheets/Slides.)

---

## 5. Test locally

Because the adapter contract is just "one JSON in, one JSON out", you can exercise
your operation dispatch as plain functions before any server is involved — e.g. a
small Node/`bun` harness that calls `handler({ operation: "capabilities" })`, then
`handler({ operation: "list", params: {...}, credential: {...}, mount: {...} })`
against sandbox provider credentials, asserting the `ExternalItem` shape.

Then test it inside RaisinDB:

1. Build the server (`cargo build --release --package raisin-server --features
   "storage-rocksdb,websocket,pgwire"`) and run it.
2. Deploy the function (via the CLI/package install below) and call it directly
   through the normal function-execution API with a hand-built `input` to confirm
   `capabilities` and `list` return the right shapes and that your `network_policy`
   permits the provider host.

### Use "Test connection" to debug

Once the Integration node exists (with an account connected, if the provider needs
one), the fastest debug loop is the **"Test connection"** endpoint:

```
POST /api/integrations/{repo}/test
     { "integration_path": "/integrations/<name>",
       "account_id": "<connected_accounts[].id>",   // omit → credential:null, auth: not_required
       "remote_root": "<optional folder id>" }
```

It runs your adapter's `capabilities` then a **bounded `list` probe** (≤10 items,
whole call under 30s) and returns a structured diagnostic — `ok`, `latency_ms`,
`auth` (`valid` / `expired` / `missing` / `not_required`), the resolved
`capabilities`, a `probe.sample` of item **names**, and a coded `error` on failure.
A failed connection is still HTTP `200`; read the `error.code` to see whether it was
`auth_expired`, a `timeout`, `adapter_not_found`, or a transient throw. On success
it **caches `capabilities` onto the Integration node** — so this is also how the
admin UI learns your adapter's shape. Secrets never appear in the response: the
credential has its `refresh_token` stripped and the probe sample omits URLs.

The engine-side behaviors (full reconcile, delta, etag skip-write, rename match,
ephemeral cleanup, backoff, lock/no-op, fencing) are all covered by the engine's
own unit tests using a `MockAdapter` in
`crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/tests.rs` — a good
reference for the exact shapes the engine feeds and expects.

---

## 6. Install the package

- **Built-in** (`builtin: true`): dropped into `builtin-packages/`, installed on
  repo creation.
- **Third-party**: package the directory as a `.rap` and install it through the
  normal package-install path. On install, the functions land in the `functions`
  workspace and any `content/_raisin__system/...` templates materialize into
  `raisin:system`.

---

## 7. Create an Integration and Mount

Create the `raisin:Integration` (or complete the shipped template) in the
`raisin:system` workspace under `/integrations/<name>`:

```yaml
node_type: raisin:Integration
properties:
  title: My Provider
  provider_type: my-adapter
  adapter_function: /adapters/my-adapter
  enabled: true
  oauth_config:            # omit for non-OAuth providers; use api_config instead
    auth_url: https://provider.example/authorize
    token_url: https://provider.example/token
    scopes: [read]
    redirect_uri: https://<your-host>/api/integrations/oauth/callback
```

The **client secret is never put in this node.** Store it encrypted:
`POST /api/integrations/{repo}/oauth/start` and the callback flow encrypt secrets
and tokens into `client_secret_encrypted` / `connected_accounts[].tokens_encrypted`
(AES-256-GCM). Connect an account:

```
POST /api/integrations/{repo}/oauth/start   { "integration_path": "/integrations/my-adapter" }
     → { auth_url, state }   → send the browser to auth_url
GET  /api/integrations/{repo}/oauth/callback?code=…&state=…   (provider redirect; engine stores the account)
```

Then create the `raisin:VirtualMount` under `/mounts/<name>`:

```yaml
node_type: raisin:VirtualMount
properties:
  title: My Mount
  integration_ref: /integrations/my-adapter
  account_ref: "<connected_accounts[].id>"     # optional; defaults to the first
  target_workspace: default
  mount_path: /external/my-data
  remote_root: "<provider root id>"
  # mapping_function: /mappers/my-adapter-default   # optional
  sync_config:
    mode: poll
    interval_seconds: 300
    max_items_per_sync: 500
    ephemeral: false
  enabled: true
```

---

## 8. Watch it sync

- The 60s scheduler enqueues `VirtualMountSyncCheck`, which enqueues a
  `VirtualMountSync` for your mount once it is due; or force it now:
  `POST /api/integrations/{repo}/mounts/{mount_id}/sync` (body `{ "mode": "full" }`
  for a full reconcile).
- First run is a **full reconcile**; nodes appear under `target_workspace` at
  `mount_path/...` with `__virtual: true`. Query them:

  ```sql
  SELECT * FROM 'default' WHERE properties->>'__mount_id'::String = '<mount_id>'
  ```

- Subsequent runs are **deltas** (if `supports_changes: true`).
- The mount's `state` (`last_sync_at`, `last_sync_token`, `status`,
  `consecutive_failures`, `last_error`) reflects progress. `status` cycles
  `ok` → `degraded` (repeated failures) → `auth_required` (token rejected).
- Because materialization is a normal write, any `node_event` trigger on the
  target subtree fires as items sync in — subscribe over WebSocket to see it live.

---

## 9. Gotchas

- **`raisin:Asset` needs a binary Resource.** The built-in default maps files to
  `raisin:Node` precisely because a link-only virtual node has no file bytes. Only
  map to `raisin:Asset` from a mapper if you understand that (the Drive mapper does
  it for links; content is not inlined in v1).
- **Adapters are privileged.** They run with a system context, RLS bypassed. Ship a
  tight `network_policy`. Treat installing one like installing a plugin.
- **Multi-node deployments must run the Redis locks backend** or mounts can
  double-sync — see the [internals doc](../concepts/virtual-nodes-internals.md#5-cluster-safety-fencing-tokens--lease-locks).
- **Write-through is deferred.** Even though you may implement `create`/`update`/
  `delete`, the engine does not yet propagate local edits back to the provider in
  v1.
- **HTTP egress, plus native protocol bindings.** The function sandbox
  (QuickJS/Starlark) has one general outbound primitive, the synchronous
  `raisin.http.fetch` — there is **no raw TCP socket** available to *function code*.
  So any HTTP(S) provider (REST/JMAP/GraphQL) is a pure-JS/Starlark adapter with no
  Rust. For a genuine non-HTTP *protocol*, RaisinDB adds the capability **natively in
  Rust** and exposes it as a `raisin.<ns>.*` host API. The first such binding is
  **native `raisin.imap.*`** (real IMAP over TLS) — so a real IMAP server is now
  reachable directly, no JMAP proxy required. The shipped `imap-adapter` still ships
  the JMAP-over-HTTP path (`builtin-packages/imap-adapter/README.md`) pending migration
  onto `raisin.imap`. If you hit a protocol `fetch` cannot express, do **not** try to
  reimplement it over `fetch` — add a native binding: see
  [Adding a Native Host Capability](./adding-a-native-host-capability.md).
