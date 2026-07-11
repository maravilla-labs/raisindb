# Virtual Nodes — Implementation Plan

> Companion to [`docs/concepts/virtual-nodes.md`](concepts/virtual-nodes.md). The concept doc
> describes *what* we're building; this document tells developers *exactly where and how* to
> build it in the current codebase. Where the concept doc's assumptions diverge from the real
> code, **this document wins** (see [Corrections to the concept doc](#corrections-to-the-concept-doc)).

---

## 0. Implementation Status (landed vs deferred)

**v1 backend + Google Drive adapter have landed.** The concept doc and reference contract now
describe shipped behavior; see also
[`docs/concepts/virtual-nodes-internals.md`](concepts/virtual-nodes-internals.md) and
[`docs/guides/building-an-adapter.md`](guides/building-an-adapter.md).

| Phase | Status | Notes |
|-------|--------|-------|
| **0** Shared secret encryption | ✅ Landed | New crate **`raisin-crypto`** (`SecretBox` = old `ApiKeyEncryptor`; wire format unchanged; `encrypt_json`/`decrypt_json`). Open Q#2 resolved: a **new crate**, not a `raisin-core` module (core doesn't depend on `ring`). Two loaders: `master_key()` and `master_key_with_embedding_fallback()` (preserves `EMBEDDING_MASTER_KEY`). |
| **1** Node types, workspace, job types | ✅ Landed | `raisin_integration.yaml`, `raisin_virtual_mount.yaml`; `system.yaml` allows both types + ships `/integrations` and `/mounts` folders. Workspace id is `raisin:system`. Three `JobType`s added (`VirtualMountSyncCheck`, `VirtualMountSync`, `IntegrationTokenRefresh`). |
| **2** Outbound OAuth + token refresh | ✅ Landed | `routes/integrations.rs` + `handlers/integrations/` (`start`, `callback`, `disconnect`, `mounts/{id}/sync`). Refresh is a Rust job (`integration_token_refresh.rs`), ~10-min bucket; refresh tokens never enter the sandbox. |
| **3** Sync engine | ✅ Landed | `virtual_mount_sync/` (`mod`, `check`, `config`, `adapter`, `materializer`, `delta`, `full`, `ephemeral`, `tests`). Full/delta, etag skip-write, `__external_id` rename match, ephemeral TTL, backoff, per-mount lease + fencing token. **Write-through (point 9) deferred** (see below). |
| **4** Google Drive adapter | ✅ Landed | `builtin-packages/google-drive-adapter/` — adapter + default mapper + disabled integration template; `network_policy` pinned to Google hosts; `get_content` implemented but engine syncs links only. |
| **5** Admin console | ✅ Landed | `Integrations.tsx` + `Mounts.tsx` pages, `IntegrationEditor` / `MountEditor`, `CapabilityChips`, `VirtualNodeBadge`. User-facing vocabulary is **"Connector"** (the Integration) and **"Mount"** — node types, routes, and properties are unchanged. Forms are **capability-driven**: the writeback control is gated on `capabilities.can_write` (and on `state.writeback_supported`, which the engine stamps `false` since write-through is unimplemented), and "Test connection" surfaces the probe result. Capabilities come from the cached `capabilities` property (`capabilitiesUnknown` prompts a probe when absent). |
| **Native host bindings** | ✅ Landed | **`raisin.imap.{fetchSince,listMailboxes,fetchMessage}`** — the first native *protocol* binding (real IMAP over TLS: `LOGIN` + `UID FETCH`), so an adapter no longer needs a JMAP-over-HTTP workaround for the "no raw socket in the sandbox" limitation. Rust owns the protocol (`crates/raisin-functions/src/runtime/imap/`), exposed to **both** runtimes from one impl: `FunctionApi::imap_*` (`api/traits.rs`, real `api/raisindb/imap.rs`, mock `api/mock/mod.rs`) → shared registry `runtime/bindings/methods/imap.rs` (Starlark/python/typescript) + QuickJS `runtime/quickjs/api_imap.rs` & `api_wrapper.js`. Egress is gated by the function `network_policy` (`imaps://host:port` must match `allowed_urls`) **before** any socket opens — regression test `imap_disallowed_host_refused_before_connect` (`api/raisindb/tests.rs`); credentials never logged. Deps added to `crates/raisin-functions/Cargo.toml`: `async-imap 0.11` (runtime-tokio), `mail-parser 0.11`, `tokio-rustls 0.26` (reuses the workspace rustls stack). Contributor guide: [`docs/guides/adding-a-native-host-capability.md`](guides/adding-a-native-host-capability.md). The `imap-adapter` package's `index.js` calls `raisin.imap.*` directly — no JMAP proxy remains. |
| **6** DX & event patterns | ✅ Landed | Docs (this plan + concept + internals + reference + guide) **plus**: the **`raisindb create adapter <name>` CLI scaffold** (`packages/raisindb-cli/src/commands/create.ts` + `templates/adapter.ts` — emits an installable package with `capabilities` + `list` stubbed and a disabled Integration template); the **IMAP adapter** (`builtin-packages/imap-adapter/`, ephemeral mailbox pattern); the **native `raisin.imap.*` host binding** (see below); the **`raisin.integrations.sync_now(mount_id, mode?)` host binding** wired into both QuickJS (`runtime/quickjs/api_integrations.rs` + `api_wrapper.js`) and Starlark (`runtime/bindings/methods/integrations.rs`), backed by `FunctionApi::integrations_sync_now`. Also a **"Test connection"** endpoint (`POST /api/integrations/{repo}/test`). Webhook-mode mounts work via the existing `/api/webhooks/{repo}/{id}` + a `raisin:Trigger` calling `sync_now`. |
| **First-class connectors (Experimental)** | ✅ Landed — **Preview** | **Gmail**, **Microsoft 365 (Graph)**, and **Google Calendar** connectors, plus the **`raisin:Event`** calendar mapping target. Shipped as an **experimental / preview** feature — validate against your own account before production. Packages: `builtin-packages/imap-adapter` (Gmail = real IMAP + SASL `XOAUTH2`, integration template `content/_raisin__system/integrations/gmail/`), `builtin-packages/ms-graph-adapter` (`resource: "mail"\|"calendar"`, mail + calendar mappers), `builtin-packages/google-calendar-adapter` (Calendar v3, read-only + `syncToken` deltas). `raisin:Event` = `crates/raisin-core/global_nodetypes/raisin_event.yaml`. Enabled by two engine fixes: **verbatim `sync_config` + `api_config` passthrough** to adapters and **`credential.username`** from the account subject (`virtual_mount_sync/adapter.rs`; see internals §12 and reference §1.2–1.3). All three integration templates ship `enabled: false` and store no secrets; each is marked preview (Google Calendar via an `experimental: true` property, Gmail and Microsoft 365 via their titles — "Gmail (Experimental)" / "Microsoft 365 (Preview)" — and header comments). |
| **Push / webhook subscription lifecycle (Experimental)** | ✅ Landed — **Preview** | Generic push: three **optional** adapter ops (`subscribe`/`renew`/`unsubscribe`) + `Capabilities.supports_push`, driven by `virtual_mount_sync/subscription.rs` (`ensure`/`renew`/`teardown`, per-mount unguessable `push_mount_token`, `MountState.push_*` fields in `config.rs`). One generic public endpoint `handlers/integrations/notifications.rs` (`GET\|POST /api/integrations/{repo}/notifications/{mount_token}`) — validation-echo + token→mount + constant-time secret check, all **shape-driven** (never provider identity); a ping only enqueues a normal delta sync (payload ignored). Renewal job `VirtualMountSubscriptionRenew` (~30-min bucket, 1-day headroom) in `main.rs`. `check.rs::is_due` bootstraps a webhook mount once to register the subscription, then it falls silent. Push wiring requires `RAISINDB_BASE_URL`. Generic JWT-verify primitive `raisin.crypto.verifyJwt` for signed (OIDC) push. Shipped connectors implement it: **Graph** (subscriptions; `clientState`), **Google Calendar** (`events.watch` channels), **Gmail** (`users.watch` Pub/Sub, only when `sync_config.pubsub_topic` is set). See internals §13 and reference §2.9. |
| **OneDrive files (Experimental)** | ✅ Landed — **Preview** | `ms-graph-adapter` `sync_config.resource: "files"` → `/me/drive/root/children` + `/me/drive/root/delta` (read-only, links only), push-capable via a driveItem Graph subscription. Shares the one Graph connector with mail/calendar. |
| **7** On-demand resolution | ⛔ Deferred **by decision** | No `VirtualPathResolver`; v1 is background-sync only. **Explicitly out of scope — do not build.** Intercepting read paths (`get_by_path` cache misses) to refresh stale nodes means **reads can invoke adapter functions**: unbounded read latency (a function call blocks up to 5 min) and auth implications (read-path function execution needs its own scoped-auth design). The cost/design round is not justified for v1; revisit only with a dedicated proposal. |

**Deferred / not implemented (summary):**

- **Write-through beyond the flag — NOT implemented in the engine.** `write_config.writeback`
  exists on the node type and the Drive adapter implements `create`/`update`/`delete`, but the
  **sync engine does not propagate local edits back to the provider** — there is no writeback
  trigger/path in `virtual_mount_sync/`. Default off; v1 mounts are read/reconcile-only.
  Conflict strategies (`remote_wins`/`error`) are unimplemented. The admin console **hides the
  writeback controls** (form visibility is driven by the cached `capabilities`; see Phase 6 /
  the internals doc §8), so operators are not offered a switch the engine can't honor.
- **On-demand / read-path resolution** (Phase 7) — deferred **by decision**, see the table
  above for the reason.
- **Content download into the binary store** — metadata + links only (risk #7 "links only in
  v1"); `get_content` is implemented in the adapters but never called by the engine.

**Notable divergences from this plan (documented in the code / internals doc):**

- **Default file mapping is `raisin:Node`, not `raisin:Asset`** (Asset needs a binary
  Resource; see `config.rs::default_mapping`). The Drive package ships a mapper that emits
  `raisin:Asset` for links.
- **Upsert-match is a full workspace scan**, not a property-index lookup — the reserved
  `__`-props are runtime-written and not declared in any node-type schema.
- **The periodic check driver takes no `vmount:check` lease** (plan §6.3 point 8 suggested
  one); the per-minute dedup bucket + per-mount `vmount-sync:{id}` dedup + per-mount lease
  suffice.
- **Manual `mounts/{id}/sync` hard-codes branch `main`** (the periodic check uses the repo's
  default branch).
- **OAuth `state` store is in-process** (single-node; multi-node must pin start/callback to one
  node).

## 1. Goal & Use Cases

Virtual Nodes turns RaisinDB into a **content hub**: external systems are mounted into
workspaces as regular nodes, kept in sync by adapter functions, and — because materialized
virtual nodes flow through the normal write path — every existing capability (node events,
triggers, workflows, SQL, fulltext, agents) works on them for free.

Target use cases (v1 should demo at least the first two):

| Use case | Pattern | What the user gets |
|----------|---------|--------------------|
| Mailbox sync (Gmail / Outlook / IMAP) | Ephemeral event stream | Incoming mail becomes nodes under `/inbox/messages/…`; `node_event` triggers fire workflows or agents per message; processed messages are archived or deleted |
| Google Drive / Google Docs sync | Persistent storage mount | Drive folder mirrored under e.g. `/documents/shared/…` as `raisin:Asset` / custom types; full CRUD write-through |
| SharePoint / OneDrive sync | Persistent storage mount | Same as Drive, different adapter package |
| Webhook event ingestion (any SaaS) | Ephemeral push | External system POSTs to existing `/api/webhooks/{repo}/{id}`; handler function materializes an event node; downstream triggers react |
| IoT device state (Hue, Nuki, sensors) | Polled device state | Device state nodes updated on an interval; `node_event` triggers drive automation |
| Agent inboxes | Any of the above | Agents subscribe via triggers and do work on whatever arrives — no new agent infrastructure needed |

**The critical design property:** the sync engine writes virtual nodes through the normal
transactional write path, so `node_event` triggers, fulltext indexing, SQL indexes, audit,
and replication all apply without modification. The only new machinery is: config node
types, an outbound-OAuth flow, a sync job handler, and adapter packages.

---

## 2. Corrections to the Concept Doc

These were verified against the codebase (July 2026). Developers must follow this table,
not the concept doc, where they conflict.

| # | Concept doc says | Reality | Consequence |
|---|------------------|---------|-------------|
| 1 | New routes in `crates/raisin-server/src/routes/oauth.rs` | HTTP routes live in **`crates/raisin-transport-http/src/routes/`**; `routes/oauth.rs` **already exists** and implements the OAuth 2.1 *authorization server* (for MCP clients) | Outbound-OAuth ("connect Google") endpoints go in a **new** module `routes/integrations.rs` under `raisin-transport-http`, namespaced `/api/integrations/…` to avoid colliding with the AS |
| 2 | Adapter signature `handler(event, context)` with `context.metadata.credential` | QuickJS calls the entrypoint with **one argument**: `handler(input)` (`runtime/quickjs/mod.rs:321`). `raisin.context` is a read-only global with `{tenant_id, repo_id, branch, workspace_id, actor, execution_id, event, trigger_name, input}`; the Rust-side `ExecutionContext.metadata` HashMap is **not** surfaced to JS | The adapter contract (§4) is defined over a single `input` object: `{ operation, params, credential, mount }` |
| 3 | "Store secrets in tenant vault" (`vault:google-oauth-secret`) | There is **no vault**. Existing pattern: AES-256-GCM via `ApiKeyEncryptor` (`crates/raisin-embeddings/src/crypto.rs`), master key from env `RAISIN_MASTER_KEY` (`raisin-functions/src/execution/ai_provider.rs:314`); OIDC configs already store `client_secret_encrypted: Option<Vec<u8>>` (`raisin-models/src/auth/config/provider_config.rs:50`) | Reuse this pattern. Phase 0 extracts the encryptor into a shared location so `raisin-embeddings`, auth, and the new integrations code share one implementation |
| 4 | Sync driven by trigger entries stored on the mount node | Cron scheduling today = a 60s loop in `raisin-server/src/main.rs:659` enqueuing `ScheduledTriggerCheck`, matched against `raisin:Trigger` nodes (functions workspace, branch `main` hard-coded). The cron matcher is flagged `TODO(v0.2)` / `dead_code` | Mount sync gets its own periodic driver mirroring `ScheduledTriggerCheck` (a `VirtualMountSyncCheck` job) rather than piggybacking on `raisin:Trigger` nodes — mount config stays on the mount node, per-mount intervals are honored, and we don't extend the half-wired trigger cron path |
| 5 | `raisin.functions.execute()` for adapter invocation from the sync function | `raisin.functions.execute` exists but is AI-tool-call flavored (creates `AIToolCall`/`AIToolResult` nodes). Plain calls = `raisin.functions.call(path, args)`. Both **block a job worker up to 5 minutes** (`callbacks/functions.rs:386`, `MAX_WAIT_MS = 300_000`) waiting on a nested `FunctionExecution` job | The sync engine is a **Rust job handler** that invokes the adapter function directly via `FunctionExecutorCallback` (same mechanism `FunctionExecutionHandler` uses) — no nested-job blocking, no worker-pool exhaustion. `raisin.functions.call` remains available for user-authored orchestration but is not on the hot path |
| 6 | Config paths `/system/integrations/`, `/system/mounts/` under "workspace = 'system'" | The `raisin:system` workspace exists (`global_workspaces/system.yaml`) with a **restricted `allowed_node_types` list** and an `initial_structure` | Correct home. Requires adding the two new node types to `allowed_node_types` and `integrations/` + `mounts/` folders to `initial_structure` |
| 7 | Webhook route "no new route definitions needed" | Confirmed: `/api/webhooks/{repo}/{webhook_id}` exists (`raisin-transport-http/src/routes/functions.rs:137-168`); `webhook_id` is a client-generated nanoid property on a `raisin:Trigger` node, looked up by linear scan (`handlers/webhooks/lookup.rs:15`) | Webhook-driven sync = the mount provisions a `raisin:Trigger` node pointing at a builtin refresh function. No route work; note the linear-scan lookup as a known perf ceiling |
| 8 | — (not mentioned) | Trigger-invoked functions run with **auth = None → system context, RLS bypassed** (`jobs/handlers/function_execution.rs:302-311`) | Adapter functions invoked by the sync engine inherit full access. Acceptable for v1 (mounts are admin-configured), but flagged in §9 as a hardening item |
| 9 | — (not mentioned) | The 60s scheduler loop is **not cluster-safe** (every node enqueues; comment at `main.rs:643-655`). However, **`raisin-locks` already provides exactly the cluster-safe primitive we need**: `LockManager::try_acquire(key, owner, ttl)` returning a monotonic fencing token, with `inprocess` and `redis` backends, one shared `Arc<dyn LockManager>` built in `main.rs` | Defense in depth: `register_job_idempotent` with time-bucketed dedup keys **plus** a per-mount lease lock around each sync run (see §6 Phase 3 point 8). Multi-node deployments must run the `redis` locks backend — same rule that already exists for inventory/locks |

---

## 3. Architecture Overview

```
                       ┌───────────────────────────────────────────────┐
                       │  raisin:system workspace (per repo)           │
                       │   /integrations/google-drive  raisin:Integration
                       │   /mounts/team-drive          raisin:VirtualMount
                       └───────────────┬───────────────────────────────┘
                                       │ read config + encrypted tokens
        60s loop (main.rs)             ▼
  VirtualMountSyncCheck ─▶ VirtualMountSync{mount_id} job (raisin-rocksdb handler)
                                       │
                                       │ FunctionExecutorCallback (direct, no nested job)
                                       ▼
                        adapter function  (functions ws, from .rap package)
                          input = { operation:"get_changes", params, credential, mount }
                                       │  raisin.http.fetch → external API
                                       ▼
                        mapping function (optional, same mechanism)
                                       │
                                       ▼
                        materializer: transactional upsert/delete into
                        target workspace with __virtual metadata
                                       │
                                       ▼
                 normal write path ⇒ node_event triggers, fulltext, SQL,
                 audit, replication — user workflows/agents react here
```

Crate touch map:

| Crate | Work |
|-------|------|
| `raisin-core` | New global node types + `system.yaml` workspace update; shared crypto module |
| `raisin-storage` | New `JobType` variants |
| `raisin-rocksdb` | `virtual_mount_sync` + `oauth_refresh` job handlers, dispatch wiring |
| `raisin-transport-http` | `routes/integrations.rs` + `handlers/integrations/` (outbound OAuth + mount admin ops) |
| `raisin-server` | Periodic sync-check enqueuer, handler wiring in startup, config section |
| `builtin-packages/` | `google-drive-adapter/`, `imap-adapter/` (stretch) |
| `admin-console` | Integrations + Mounts pages |

---

## 4. Contracts (freeze these first)

These interfaces are the package-facing API surface. They must be reviewed and frozen in
week 1, because adapter packages, the sync engine, and the admin console all build against
them. Put them in `docs/reference/virtual-node-adapters.md` as the canonical spec once agreed.

### 4.1 Adapter function contract

An adapter is a `raisin:Function` (JS or Starlark) whose entrypoint receives **one** object:

```javascript
// input shape (single handler argument)
{
    operation: "list" | "get" | "get_content" | "create" | "update" | "delete"
             | "get_changes" | "capabilities",
    params: { /* operation-specific, see table */ },
    credential: {           // decrypted by the sync engine just before invocation
        access_token: "...",
        account_id: "...",
        provider_type: "google-drive",
        // refresh_token is NEVER passed to the adapter
    },
    mount: {                // read-only snapshot of relevant mount config
        mount_id: "...",
        remote_root: "...",
        mount_path: "/documents/shared",
        sync_config: { ... }
    }
}
```

`raisin.context` is available as usual (tenant/repo/branch/workspace/execution_id) but the
adapter must not depend on `context.event` — it is invoked directly, not via a trigger.

Operations and their `params` / return shapes:

| operation | params | returns |
|-----------|--------|---------|
| `capabilities` | `{}` | `Capabilities` (below) |
| `list` | `{ folder_id?, cursor?, limit? }` | `{ items: ExternalItem[], next_cursor: string\|null }` |
| `get` | `{ item_id?, path? }` | `ExternalItem \| null` |
| `get_content` | `{ item_id }` | `{ content, mime_type }` |
| `create` | `{ parent_id, name, is_folder, content?, mime_type? }` | `ExternalItem` |
| `update` | `{ item_id, name?, content?, mime_type?, etag? }` | `ExternalItem` (throw `ConflictError` on etag mismatch) |
| `delete` | `{ item_id }` | `{ deleted: true }` |
| `get_changes` | `{ since_token: string\|null, folder_id? }` | `{ items: Change[], next_token: string }` |

```javascript
// ExternalItem — normalized external object
{
    external_id: string,
    name: string,
    mime_type: string | null,
    size_bytes: number | null,
    is_folder: boolean,
    parent_id: string | null,
    created_at: string,       // ISO 8601
    modified_at: string,      // ISO 8601
    etag: string | null,      // change-detection token
    web_url: string | null,
    download_url: string | null,
    metadata: object          // provider-specific passthrough (kept on the node)
}

// Change
{ type: "created" | "updated" | "deleted", item: ExternalItem, relative_path: string }

// Capabilities
{
    can_read: boolean, can_write: boolean, can_create_folders: boolean,
    supports_changes: boolean,   // has real delta API; if false, engine falls back to full listing diff
    supports_webhooks: boolean,
    supports_search: boolean,
    supports_push: boolean,      // event-driven providers
    default_ttl: number | null,  // suggested TTL for ephemeral nodes (seconds)
    max_file_size: number | null
}
```

Error convention: throw `Error` with a `code` property — `"auth_expired"` (engine marks the
account as needing re-auth and pauses the mount), `"rate_limited"` (engine backs off using
the job retry mechanism), `"conflict"` (write-through only, surfaced to caller), anything
else = transient failure, standard job retry.

### 4.2 Mapping function contract

Optional per-mount function; called once per external item:

```javascript
// input
{ external_item: ExternalItem, mount: { mount_id, mount_path, sync_config } }
// return
{ node_type: "raisin:Asset", name?: string, properties: { title: "...", ... } }
// return null → skip this item (filtering hook)
```

If `mapping_function` is unset, the engine applies the built-in default mapping
(folder → `raisin:Folder`, everything else → `raisin:Asset` with title/mimeType/size).
The default lives in Rust (materializer), not in a function, so the minimal sync path
has zero function invocations for mapping.

### 4.3 Reserved virtual metadata properties

Written by the materializer on every synced node:

| Property | Type | Meaning |
|----------|------|---------|
| `__virtual` | Boolean | Marks a mount-managed node |
| `__mount_id` | String | Owning mount node id |
| `__external_id` | String | Provider item id (stable key for upsert matching) |
| `__etag` | String | Provider change token at last sync |
| `__synced_at` | String | ISO timestamp of last sync write |

These are plain properties → SQL just works (`properties->>'__mount_id'::String = $1`,
per the JSON-property query conventions in `CLAUDE.md`). Add `index: [Property]` for
`__mount_id` and `__external_id` in the mapped node types we control; document that custom
types used with mounts should index them too.

---

## 5. Data Model

### 5.1 `raisin:Integration` — `crates/raisin-core/global_nodetypes/raisin_integration.yaml`

```yaml
name: raisin:Integration
description: Tenant/repo-level configuration for an external system integration
icon: plug
version: 1
strict: true
indexable: true
index_types: [Property]
properties:
  - name: title
    type: String
    required: true
    title: Title
  - name: provider_type          # google-drive, imap, sharepoint, custom…
    type: String
    required: true
    index: [Property]
  - name: adapter_function       # default adapter path in functions ws
    type: String
    required: true
  - name: oauth_config           # client_id, auth_url, token_url, scopes, redirect_uri
    type: Object                 # client_secret stored ONLY as client_secret_encrypted
  - name: client_secret_encrypted
    type: String                 # base64(nonce||ciphertext), AES-256-GCM, RAISIN_MASTER_KEY
  - name: api_config             # non-OAuth providers: endpoints, key refs
    type: Object
  - name: connected_accounts     # [{ id, label, subject, expires_at, tokens_encrypted }]
    type: Array
  - name: enabled
    type: Boolean
    default: true
    index: [Property]
versionable: false
publishable: false
auditable: true
```

`connected_accounts[].tokens_encrypted` holds base64 AES-256-GCM of
`{ access_token, refresh_token }` — **refresh tokens never appear in plaintext in any node
property, API response, or function input**. `expires_at` and `subject` (account email)
stay plaintext so the refresh job and admin UI can operate without decrypting.

### 5.2 `raisin:VirtualMount` — `crates/raisin-core/global_nodetypes/raisin_virtual_mount.yaml`

```yaml
name: raisin:VirtualMount
description: Mounts an external system subtree into a workspace path
icon: hard-drive
version: 1
strict: true
indexable: true
index_types: [Property]
properties:
  - name: title
    type: String
    required: true
  - name: integration_ref        # path of the raisin:Integration node
    type: String
    required: true
    index: [Property]
  - name: account_ref            # connected_accounts[].id
    type: String
  - name: target_workspace
    type: String
    required: true
    index: [Property]
  - name: mount_path             # path prefix inside target workspace
    type: String
    required: true
  - name: remote_root            # provider-side root (folder id, mailbox, …)
    type: String
  - name: adapter_function       # optional override of integration default
    type: String
  - name: mapping_function       # optional custom mapper
    type: String
  - name: sync_config
    type: Object
    # { mode: "poll"|"webhook"|"hybrid", interval_seconds: 300,
    #   include_patterns: [], exclude_patterns: [],
    #   ephemeral: false, ttl_seconds: null, max_items_per_sync: 500 }
  - name: write_config
    type: Object
    # { writeback: "off"|"write_through", conflict: "remote_wins"|"error" }
  - name: state                  # engine-managed, not user-edited
    type: Object
    # { last_sync_token, last_sync_at, last_error, consecutive_failures, status }
  - name: enabled
    type: Boolean
    default: true
    index: [Property]
versionable: false
publishable: false
auditable: true
```

### 5.3 Workspace changes — `crates/raisin-core/global_workspaces/system.yaml`

- Append `raisin:Integration`, `raisin:VirtualMount` to `allowed_node_types`.
- Add to `initial_structure.children`: folders `integrations` and `mounts`
  (`node_type: raisin:Folder`).

Mount/integration nodes therefore live at `/integrations/{name}` and `/mounts/{name}` in
the `raisin:system` workspace of each repo. (The concept doc's "tenant level vs database
level" split collapses to repo level for v1, matching the existing "Configuration scope =
repository level" design decision. Cross-repo sharing is a later concern.)

### 5.4 New `JobType` variants — `crates/raisin-storage/src/jobs/types/job_type.rs`

```rust
/// Periodic scan: find due mounts, enqueue per-mount sync jobs (like ScheduledTriggerCheck)
VirtualMountSyncCheck { tenant_id: Option<String>, repo_id: Option<String> },
/// Sync one mount (delta or full)
VirtualMountSync { mount_id: String, mode: String /* "delta" | "full" */ },
/// Refresh expiring OAuth tokens across all integrations
IntegrationTokenRefresh { tenant_id: Option<String> },
```

Follow the existing string round-trip serialization in that file and add sensible
`default_timeout_seconds` entries (`VirtualMountSync`: 600; the checks: 60).

---

## 6. Implementation Phases

Each phase is independently mergeable and ends with the listed acceptance criteria.
Estimated sizes assume one developer familiar with the codebase.

### Phase 0 — Shared secret encryption (prerequisite, ~1–2 days)

The AES-256-GCM `ApiKeyEncryptor` currently lives in `crates/raisin-embeddings/src/crypto.rs`
and is re-implemented conceptually in auth. Integrations need it too — extract once.

1. Create `crates/raisin-core/src/crypto/secrets.rs` (or a tiny new `raisin-crypto` crate if
   `raisin-core`'s dependency profile makes `ring` unwelcome): move `ApiKeyEncryptor`
   (rename `SecretBox` or similar), keep the `[nonce(12)][ciphertext+tag]` wire format
   **unchanged** so existing encrypted provider keys keep decrypting.
2. Move `get_master_key()` (env `RAISIN_MASTER_KEY`, 32-byte hex) next to it; single source
   of truth for key loading and error messages.
3. Re-export / adapt call sites: `raisin-embeddings`, `raisin-functions/src/execution/ai_provider.rs:173`,
   `raisin-auth` OIDC strategy init.
4. Add helpers `encrypt_json(&Value) -> String(base64)` / `decrypt_json(&str) -> Value`
   for token blobs.

**Accept:** existing tests pass; one shared implementation; round-trip unit test for the
base64 JSON helpers.

### Phase 1 — Node types, workspace, job types (~2 days)

1. Add the two YAML node types (§5.1, §5.2) and update `system.yaml` (§5.3).
2. Add the three `JobType` variants (§5.4).
3. Snapshot/registry plumbing if node types require registration anywhere beyond the YAML
   dir (verify against how `raisin_function.yaml` is loaded — it's the template to copy).

**Accept:** fresh server boot creates `/integrations` and `/mounts` folders in the system
workspace; both node types creatable via API with `strict` validation enforced;
`cargo test --workspace` green.

### Phase 2 — Outbound OAuth + token lifecycle (~1 week)

New module `crates/raisin-transport-http/src/routes/integrations.rs` +
`handlers/integrations/` (follow the module-per-domain pattern; register in
`routes/mod.rs` behind `#[cfg(feature = "storage-rocksdb")]` like `oauth_routes`).

Endpoints (all under `require_auth_middleware`, admin-gated — mutating integration config
is an admin operation):

| Route | Purpose |
|-------|---------|
| `POST /api/integrations/{repo}/oauth/start` | body `{ integration_path }` → `{ auth_url, state }`. State = signed nanoid persisted as a short-lived node or in-memory map with TTL; includes tenant/repo/integration path |
| `GET  /api/integrations/{repo}/oauth/callback` | `?code&state` → validates state, exchanges code at `token_url` (server-side `reqwest`), encrypts tokens (Phase 0 helpers), appends to `connected_accounts`, redirects to admin console success page |
| `POST /api/integrations/{repo}/oauth/disconnect` | body `{ integration_path, account_id }` → best-effort provider revoke, remove account entry |
| `POST /api/integrations/{repo}/mounts/{mount_id}/sync` | manual "sync now": enqueue `VirtualMountSync { mount_id, mode }` (idempotent, dedup on mount_id) |

Token refresh job — `crates/raisin-rocksdb/src/jobs/handlers/integration_token_refresh.rs`:

1. Handler scans repos for `raisin:Integration` nodes with `connected_accounts` whose
   `expires_at < now + 30min`, POSTs the refresh grant, re-encrypts, updates the node
   in a transaction with `AuthContext::system()` + actor `"integration-token-refresh"`.
2. Driver: extend the existing 60s scheduler block in `raisin-server/src/main.rs` (near the
   `ScheduledTriggerCheck` registration at `main.rs:659-700`) to also enqueue
   `IntegrationTokenRefresh` every 10 minutes, using `register_job_idempotent` with dedup
   key `token-refresh:{10min-bucket}` (cluster-safety, correction #9).
3. Refresh happens in the Rust handler, **not** in a function (the concept doc's JS refresh
   job would expose refresh tokens to the function sandbox — rejected).

**Accept:** end-to-end connect flow against a real Google OAuth client (manual test doc in
`docs/testing/virtual-nodes.md`); tokens at rest are ciphertext (verify via raw node read);
refresh job renews an artificially-expired token; disconnect removes the account.

### Phase 3 — Sync engine (~2 weeks; the core)

Job handler at `crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/` — mirror the
structure of `package_install/` (multi-file module, 300-line rule):

```
virtual_mount_sync/
├── mod.rs            # handler struct, dispatch entry, wiring types
├── check.rs          # VirtualMountSyncCheck: scan mounts, enqueue due syncs
├── config.rs         # load + validate mount/integration nodes into typed structs
├── adapter.rs        # AdapterInvoker trait + FunctionExecutorCallback-backed impl
├── materializer.rs   # NodeMaterializer trait + transactional upsert/delete impl
├── delta.rs          # get_changes-driven incremental sync
├── full.rs           # full-listing reconcile (initial sync + supports_changes=false)
└── ephemeral.rs      # TTL cleanup for ephemeral mounts
```

Key design points:

1. **Handler wiring.** `VirtualMountSyncHandler` registered in `JobHandlerRegistry`
   (`jobs/handlers/mod.rs`) with dispatch arms for the three new `JobType`s; constructed in
   `storage/jobs/init_system/` alongside the other handlers. It needs the
   `FunctionExecutorCallback` (same injection the `FunctionExecutionHandler` gets — see
   `init_system/flow_handlers.rs`) — dependency inversion, since `raisin-rocksdb` can't
   depend on `raisin-functions`.

2. **`check.rs`** handles `VirtualMountSyncCheck`: enumerate tenants → repos → system
   workspace → enabled `raisin:VirtualMount` nodes where
   `state.last_sync_at + sync_config.interval_seconds <= now` and `sync_config.mode != "webhook"`;
   enqueue `VirtualMountSync { mount_id, mode: "delta" }` via `register_job_idempotent`
   (dedup key = `vmount-sync:{mount_id}` — a mount never has two in-flight syncs).
   Driver: same `main.rs` scheduler block, every 60s, dedup-bucketed (correction #9).

3. **`adapter.rs`** defines:
   ```rust
   #[async_trait]
   pub trait AdapterInvoker: Send + Sync {
       async fn invoke(&self, tenant: &str, repo: &str, adapter_path: &str,
                       input: serde_json::Value) -> Result<serde_json::Value>;
   }
   ```
   Production impl wraps `FunctionExecutorCallback` (direct execution — no nested
   `FunctionExecution` job, per correction #5). It builds the §4.1 input: decrypts the
   account's `tokens_encrypted` (Phase 0 helpers), strips `refresh_token`, attaches the
   mount snapshot. Map adapter error `code`s to typed engine errors
   (`AdapterError::{AuthExpired, RateLimited, Conflict, Transient}`) in `raisin-error`.

4. **`materializer.rs`** defines:
   ```rust
   #[async_trait]
   pub trait NodeMaterializer: Send + Sync {
       async fn upsert(&self, scope: &MountScope, rel_path: &str, mapped: MappedNode,
                       virt: VirtualMeta) -> Result<()>;
       async fn delete(&self, scope: &MountScope, rel_path: &str) -> Result<()>;
       async fn list_virtual(&self, scope: &MountScope) -> Result<Vec<VirtualNodeRef>>; // for full reconcile
   }
   ```
   Impl opens a storage transaction, `tx.set_actor("virtual-mount-sync")`,
   `tx.set_auth_context(AuthContext::system())` (pattern:
   `builtin_package_init_handler.rs:230-233`). Upsert matches by
   `properties.__external_id` within the mount subtree first, falling back to path — so a
   rename on the provider side updates the existing node instead of duplicating.
   **Skip-write rule:** if the existing node's `__etag` equals the incoming etag, don't
   write — avoids revision churn and spurious `node_event` trigger storms.
   Deletes only touch nodes with `__mount_id == mount.id` (never delete user-created nodes
   that happen to sit under the mount path).

5. **`delta.rs` / `full.rs`.** Delta loop: call `get_changes(since_token)`, page until
   `next_token` stabilizes or `max_items_per_sync` reached, map each item (custom
   `mapping_function` via `AdapterInvoker`, else built-in Rust default per §4.2),
   materialize, then persist `state.last_sync_token/last_sync_at` on the mount node.
   Full reconcile (first sync, or `supports_changes: false`, or `mode: "full"` from the
   manual-sync endpoint): recursive `list`, upsert everything, then delete virtual nodes in
   `list_virtual()` not seen this pass. Apply `include_patterns`/`exclude_patterns` (glob)
   before mapping.

6. **Failure handling.** Adapter `auth_expired` → set `state.status = "auth_required"`,
   skip mount in future checks until reconnect, surface in admin console. Other errors →
   increment `state.consecutive_failures`, record `state.last_error`; after N (config,
   default 5) consecutive failures set `state.status = "degraded"` and back off to
   `interval * 2^min(failures,5)`. Success resets. Partial-progress safety: persist
   `last_sync_token` only after all changes of a page were materialized; re-running a page
   is safe because upsert is idempotent (etag skip-write).

7. **`ephemeral.rs`.** For mounts with `sync_config.ephemeral: true`: during each sync (and
   in the check pass), delete virtual nodes whose `__synced_at + ttl_seconds < now`. This
   gives the mailbox/webhook pattern its auto-cleanup without a separate job type.

8. **Cluster-safe execution via `raisin-locks` lease locks.** The idempotent-registration
   dedup (point 2) prevents *most* duplicate work, but on a multi-node cluster two nodes
   can still race the same mount (each node runs its own scheduler loop, and job dispatch
   is per-node). The existing `raisin-locks` primitive closes this properly:
   - At the start of every `VirtualMountSync` run, `lock_manager.try_acquire(key, owner, ttl)`
     with key `{tenant}\0{repo}\0{branch}\0vmount:{mount_id}` (caller-scoped key convention,
     same as the WS/HTTP lock surfaces), owner = `{node_id}:{job_id}`, ttl = the job
     timeout (600s). `None` → another node is syncing this mount: **exit successfully as a
     no-op** (not a retryable failure).
   - The returned **fencing token** is threaded through the sync run and written into
     `state.last_fencing_token` together with `last_sync_token`. The mount-state update
     rejects (skips) the write if the stored token is newer — a paused/GC-stalled sync
     resuming after its lease expired can no longer clobber a newer sync's cursor. This is
     precisely the stale-token pattern `raisin-locks` was built for.
   - Renew the lease (`renew`) between pages for long full-reconciles; release on completion.
   - The periodic drivers (`VirtualMountSyncCheck`, `IntegrationTokenRefresh`) take a
     short-ttl lock on `{tenant}\0_\0_\0vmount:check` / `…:token-refresh` so only one node's
     scheduler tick does the scan per interval.
   - **Deployment rule (already established for locks/inventory): multi-node clusters MUST
     configure the `redis` locks backend** (`[locks] backend = "redis"`, build with
     `--features locks-redis`); `inprocess` is single-node only and the server already
     warns on `inprocess` + replication. The sync engine inherits that rule — document it
     in the mount setup docs. If `[locks]` is not enabled at all, fall back to
     dedup-key-only behavior (current single-node semantics) and log a warning when
     replication is active.
   - Wiring: the `Arc<dyn LockManager>` is already built once in `main.rs` and shared
     across surfaces — inject it into `VirtualMountSyncHandler` at construction in
     `init_system/`, same pattern as the executor callback.

9. **Write-through (v1 = minimal).** `write_config.writeback: "write_through"`: a
   `node_event` on a virtual node from a non-sync actor triggers propagation to the
   provider. Implement as a builtin standalone trigger + Rust-side guard: v1 ships
   **`writeback: "off"` as default and only implements create/update/delete propagation for
   the Drive adapter behind the flag**, with `conflict: "remote_wins"` (local edit lost +
   warning event) as the only strategy. Full conflict handling is explicitly out of scope
   for v1 — document it. Loop prevention: the sync engine's actor is
   `"virtual-mount-sync"`; the writeback trigger filter must exclude events with that actor.

**Accept:** with a `MockAdapter` (test-only `AdapterInvoker`), unit tests cover: initial
full sync, delta add/update/delete, rename via `__external_id` match, etag skip-write,
pattern filters, ephemeral TTL cleanup, failure backoff, no-delete of non-virtual nodes,
lock-held → no-op exit, and stale-fencing-token rejection of the mount-state write
(use `InProcessLockManager` in tests — it has the full semantics).
Integration test (`crates/raisin-server/tests/virtual_mount_sync_test.rs`, `#[ignore]`
cluster-style) proving a synced node fires a user-defined `node_event` trigger.

### Phase 4 — Google Drive adapter builtin package (~1 week)

`builtin-packages/google-drive-adapter/`:

```
google-drive-adapter/
├── manifest.yaml
├── README.md
└── content/
    ├── functions/adapters/google-drive/        # index.js + .node.yaml
    ├── functions/mappers/google-drive-default/ # Docs/Sheets/Slides/file mapping
    └── system/integrations/google-drive/       # pre-configured Integration template
```

```yaml
# manifest.yaml — real fields per the package system
name: google-drive-adapter
version: 0.1.0
title: Google Drive
description: Mount Google Drive folders as virtual nodes
category: integrations           # discovery contract for the admin console
builtin: true
provides:
  functions:
    - /functions/adapters/google-drive
    - /functions/mappers/google-drive-default
  content:
    - /system/integrations/google-drive
```

1. Adapter `index.js` implements the §4.1 contract over Drive v3 using `raisin.http.fetch`
   (the concept doc's sample implementation is a solid starting point — adjust the entry
   signature to `handler(input)` and drop `ensureValidToken`/refresh logic, which the
   engine owns). Set the function node's `resource_limits.timeout_ms` generously
   (e.g. 120000) and declare an explicit `network_policy` allowing only
   `www.googleapis.com` / `oauth2.googleapis.com`.
2. Mapper maps Google Docs/Sheets/Slides to distinct types (per concept doc example) —
   ship them as `raisin:Asset` defaults with provider metadata; custom types are the
   user's mapper's job.
3. The pre-configured `raisin:Integration` template node ships `provider_type`,
   `adapter_function`, `oauth_config` endpoints + scopes, `enabled: false` — user fills in
   client_id/secret via admin console.
4. Verify `system` workspace materialization from packages works for the new types (the
   installer materializes `content/` per workspace — if the system workspace isn't
   currently a valid package content target, fix `install_content/` accordingly; check
   `workspace_patches` isn't needed since Phase 1 already allows the types).

**Accept:** fresh server bootstrap installs the package; manual e2e (documented in
`docs/testing/virtual-nodes.md`): connect a real Google account → create mount → files
appear as nodes → edit a file in Drive → delta sync updates the node → trigger fires.

### Phase 5 — Admin console (~1–1.5 weeks, frontend)

1. **Integrations page** (`/settings/integrations`): list installed adapter packages
   (query packages where `properties->>'category'::String = 'integrations'`), list
   `raisin:Integration` nodes, create/edit config (secret input write-only — POSTs to a
   small handler that encrypts and stores `client_secret_encrypted`, never echoes it),
   "Connect account" button driving the Phase 2 OAuth popup flow, connected-accounts list
   with disconnect.
2. **Mounts page** (`/settings/mounts`): CRUD for `raisin:VirtualMount` nodes with pickers
   for integration/account/workspace, sync config form; status column from `state`
   (`ok / syncing / auth_required / degraded` + `last_sync_at`, `last_error`); "Sync now"
   button → manual sync endpoint.
3. Virtual-node badge in the content tree/editor for nodes with `__virtual: true`
   (read-only hint when mount writeback is off).

**Accept:** the whole Drive e2e (Phase 4 accept) is doable from the UI without curl.

### Phase 6 — DX & event-driven patterns (~1 week)

1. **Adapter scaffold**: `raisin-cli` command (or skill) `raisin create adapter <name>` —
   emits the package skeleton with a stubbed adapter implementing `capabilities` + `list`,
   a passing local test, and the manifest. This is the "nice DX for custom virtual nodes"
   requirement.
2. **IMAP/email adapter package** (stretch, or fast-follow): `imap-adapter` builtin package
   demonstrating the ephemeral pattern — `get_changes` = UID-based fetch of new messages,
   mapper produces `raisin:Message`-ish nodes, mount configured `ephemeral: true`. This is
   the flagship "agents work the inbox" demo. **Update:** the flagship path is now the
   **native `raisin.imap.*` binding** (real IMAP over TLS), which supersedes the JMAP-over-HTTP
   workaround — see the "Native host bindings" row in §0 and
   [`docs/guides/adding-a-native-host-capability.md`](guides/adding-a-native-host-capability.md).
   The `imap-adapter` package's `index.js` now calls `raisin.imap.fetchSince` /
   `fetchMessage` / `listMailboxes` directly — no JMAP proxy remains.
3. **Webhook-driven refresh**: builtin function `/functions/lib/raisin/integrations/webhook-refresh`
   shipped in a `raisin-integrations` support package — receives provider webhooks
   (via a `raisin:Trigger` node with a nanoid `webhook_id` the admin console provisions per
   mount) and enqueues `VirtualMountSync { mount_id, mode: "delta" }` (expose a
   `raisin.integrations.sync_now(mount_id)` host binding, or have the function call the
   manual-sync HTTP endpoint on localhost — decide during Phase 3; a host binding is
   cleaner: add `integrations_sync_now` to the `FunctionApi` trait + registries).
4. Docs: `docs/reference/virtual-node-adapters.md` (frozen §4 contract),
   `docs/guides/building-an-adapter.md` (walkthrough using the scaffold),
   update `docs/concepts/virtual-nodes.md` to reference this plan's corrections.

### Phase 7 — On-demand resolution (optional, post-v1)

On-access refresh of stale nodes / `VirtualPathResolver` interception of `get_by_path`
cache misses. Deferred per the concept doc's design decision. Note for whoever picks it
up: intercepting reads means read paths can invoke functions — latency and auth
implications need their own design round; do not bolt on.

---

## 7. Transport Visibility & Authorization

Because virtual nodes are materialized as **regular rows through the normal write path**,
no per-transport work is needed for them to be *queryable* — all three transports see them
immediately, each under its own existing authentication and the same RLS/AuthContext
enforcement:

| Transport | How virtual nodes are visible | Auth model (existing, unchanged) |
|-----------|-------------------------------|----------------------------------|
| HTTP (`raisin-transport-http`) | Node API + SQL endpoints; `properties->>'__virtual'::String = 'true'` filters work | Auth middleware injects `AuthContext` (`middleware/auth.rs`); the OAuth 2.1 AS in `routes/oauth.rs` serves MCP-client auth |
| WebSocket (`raisin-transport-ws`) | `SqlQuery` request type (`handlers/nodes/sql_query.rs`) plus node subscriptions — **real-time events for synced changes come for free**, since materialization emits normal node events | Per-connection `Authenticate` / `AuthenticateJwt` → `AuthContext` on the connection (`handlers/auth.rs`) |
| PGWire (`raisin-transport-pgwire`) | Plain `psql`/BI-tool SQL over synced data | Password/API-key startup auth → per-connection `AuthContext` with resolved permissions (`auth/handler.rs`, `simple_query/session_commands.rs:75-76`) — RLS applies to SQL here too |

Two clarifications worth stating explicitly for reviewers:

1. **The outbound-integrations OAuth flow (Phase 2) is HTTP-only by nature** — it is a
   browser redirect dance (`/api/integrations/…/oauth/start` → provider → `…/callback`),
   used once at *configuration* time by an admin. It is a **separate concern** from the
   existing OAuth authorization server in `routes/oauth.rs` (which authenticates *inbound*
   MCP clients) and from WS/pgwire session auth. WS and pgwire never need integration
   endpoints; they just query the resulting nodes with their normal credentials.
2. **Authorization on virtual node *data* is uniform across transports**: it's enforced at
   the core/RLS layer via the per-connection/per-request `AuthContext`, not per transport.
   A user who can't see `/documents/shared` over HTTP can't see it over pgwire either.
   The only privileged paths are the admin-gated integration/mount config endpoints
   (HTTP) and the sync engine's own system-context writes (risk #1).

Nice consequence for the agent/workflow story: a WS client subscribed to the mount subtree
receives live events as external changes sync in — no polling on the client side.

---

## 8. Testing Plan

| Layer | What | Where |
|-------|------|-------|
| Unit | crypto round-trip, cron/dedup key bucketing, config parsing/validation, glob filters | in-crate `#[cfg(test)]` |
| Unit | sync engine against `MockAdapter` + in-memory storage: full/delta/rename/etag-skip/ephemeral/backoff/non-virtual-safety (Phase 3 accept list) | `virtual_mount_sync` module tests |
| Integration | OAuth start/callback/refresh with a stub token server (wiremock) | `crates/raisin-transport-http` or `raisin-server/tests/` |
| Integration | synced node → `node_event` trigger fires user function | `raisin-server/tests/virtual_mount_sync_test.rs` (`#[ignore]`) |
| Integration | package install of `google-drive-adapter` on bootstrap | extend existing builtin-package tests |
| Manual e2e | real Google account, documented script | `docs/testing/virtual-nodes.md` |
| Cluster | two nodes + redis locks backend: exactly one sync per interval per mount; stale holder cannot overwrite newer sync state (fencing) | `cluster_virtual_mount_test.rs` (`#[ignore]`) |

---

## 9. Risks & Known Constraints (read before coding)

1. **Trigger/system-auth blast radius (correction #8).** Sync-materialized writes run as
   system; downstream trigger functions invoked by those events also run with system
   context (existing behavior, `function_execution.rs:302-311`). An adapter package is
   therefore highly privileged code. v1 mitigations: adapters are installed by admins,
   `network_policy` on adapter function nodes restricted to provider hosts, refresh tokens
   never enter the sandbox. Post-v1: scoped auth contexts for function execution.
2. **Worker-pool exhaustion via nested function calls (correction #5).**
   `raisin.functions.call` blocks a worker up to 5 min per nesting level and there is no
   recursion-depth guard. The sync engine avoids this by direct invocation; adapter
   authors must be documented away from calling other functions in hot loops.
3. **Scheduler is not cluster-safe (correction #9) — mitigated with `raisin-locks`.** All
   periodic drivers added here MUST use `register_job_idempotent` with time-bucketed dedup
   keys, AND every sync run must hold a per-mount lease lock with its fencing token guarding
   the mount-state write (§6 Phase 3 point 8). This makes virtual-node sync cluster-safe
   **provided the `redis` locks backend is configured on multi-node deployments** — the same
   pre-existing rule as for inventory/lease locks (`inprocess` is single-node only). The
   generic leader-election TODO at `main.rs:643-655` remains, but virtual nodes no longer
   depend on it.
4. **Webhook lookup is a linear scan** over all `raisin:Trigger` nodes
   (`handlers/webhooks/lookup.rs:32-38`). Fine at current scale; becomes a hot path if
   thousands of mounts use webhook mode — flag for an index later.
5. **Function timeout defaults (30s)** are too small for big syncs. The engine pages with
   `max_items_per_sync` and per-call `get_changes` batches so each adapter invocation stays
   well under its limit; adapter function nodes ship with raised `resource_limits`.
6. **Trigger storms.** A first sync of a 10k-file Drive folder emits 10k node events.
   Etag skip-write prevents *repeat* storms; for initial sync consider (during Phase 3)
   whether a bulk/quiet write mode is needed, or at minimum document that mount-scoped
   trigger filters should be specific.
7. **Binary content.** v1 syncs metadata; file *content* download (`get_content`) into the
   binary store is only needed for write-through and preview. Decide in Phase 4 whether the
   Drive adapter stores content or just `web_url`/`download_url` links — recommendation:
   **links only in v1**, content sync as fast-follow (interacts with fs/s3 binary backends
   and size limits).
8. **`RAISIN_MASTER_KEY` becomes load-bearing.** It already gates AI provider keys; with
   integrations it gates all external-system access. Deployment docs must cover key
   provisioning and the (currently unsupported) rotation story.

---

## 10. Open Questions (decide in week 1, don't block Phase 0/1)

1. **Host binding vs HTTP for webhook-refresh** (Phase 6.3) — recommendation: host binding
   `raisin.integrations.sync_now`.
2. **Where does the shared crypto land** — `raisin-core` module vs new `raisin-crypto`
   crate. Depends on whether `raisin-core` already pulls `ring` transitively.
3. **Per-tenant vs per-repo integrations.** v1 = per-repo (matches existing scope
   decision). If tenant-level sharing is wanted later, an `raisin:Integration` in a
   tenant-shared repo + cross-repo `integration_ref` is the likely shape — keep
   `integration_ref` a string path, don't over-structure it now.
4. **Ephemeral cleanup actor semantics** — should TTL deletes fire `node_event` triggers
   (so "message expired" is observable) or be silent? Recommendation: fire them; they're
   filterable by actor `"virtual-mount-sync"`.

---

## 11. Suggested Sequencing & Estimate

```
Week 1:  Phase 0 + Phase 1 + freeze §4 contracts (review with team)
Week 2:  Phase 2 (OAuth + refresh job)
Week 3-4: Phase 3 (sync engine)
Week 5:  Phase 4 (Drive package) — admin console (Phase 5) starts in parallel
Week 6:  Phase 5 finish + Phase 6 (scaffold, docs, IMAP stretch)
```

~6 developer-weeks backend + ~1.5 weeks frontend for a v1 that demos: connect Google →
mount Drive folder → files as nodes → triggers/agents react → custom adapter scaffolded
in minutes.
