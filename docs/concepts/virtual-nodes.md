# Virtual Nodes

> **Status:** shipped (v1). This document describes what is actually built. It
> supersedes the original design sketch; where the two disagreed, nine
> corrections were folded in (they are called out inline as **Correction**
> notes). Companion documents:
> - [`docs/reference/virtual-node-adapters.md`](../reference/virtual-node-adapters.md) — the frozen adapter contract (write an adapter from this alone).
> - [`docs/concepts/virtual-nodes-internals.md`](./virtual-nodes-internals.md) — the sync-engine architecture, cluster-safety, operations, and security model.
> - [`docs/guides/building-an-adapter.md`](../guides/building-an-adapter.md) — a hands-on walkthrough.

## Overview

Virtual Nodes turn RaisinDB into a **content hub**: an external system (Google
Drive, IMAP, an IoT hub, any SaaS API) is *mounted* into a workspace subtree and
kept in sync by an **adapter function**. Because the sync engine materializes
external items through the **normal transactional write path**, they become
ordinary nodes — so `node_event` triggers, workflows, agents, SQL, fulltext,
audit, and replication all work on them with **zero** additional machinery.

The only new components are:

1. Two config node types — `raisin:Integration` and `raisin:VirtualMount`.
2. An **outbound** OAuth flow (connect an external account) plus a token-refresh
   job.
3. A Rust **sync engine** job handler.
4. **Adapter packages** (`.rap`) — the extensibility surface. `google-drive-adapter`
   ships built-in.

**Core principles**

- **Adapters are functions.** An adapter is a `raisin:Function` (JavaScript/QuickJS
  or Starlark), so integrations are installable, sandboxed, and user-extensible.
- **Everything is a node.** Configuration, credentials, and the synced data are
  all nodes — manageable through the normal API and admin console.
- **Mount anywhere.** Any workspace, any path prefix, any depth.

---

## Configuration model

> **Correction (was: two-level tenant + database config).** The original
> "tenant-level integrations, database-level mounts" split collapses to a single
> **per-repo** scope in v1, matching RaisinDB's existing "configuration = repository
> level" decision. Both node types live in the **`raisin:system`** workspace of each
> repo. (Note the workspace id is literally `raisin:system`, not `system`.)

```
raisin:system workspace  (per repo)
  /integrations/{name}   raisin:Integration   — external system + credentials
  /mounts/{name}         raisin:VirtualMount   — a subtree mounted into a workspace
```

Both folders are created by the system-workspace bootstrap
(`crates/raisin-core/global_workspaces/system.yaml`), and both node types are in
that workspace's `allowed_node_types`.

### `raisin:Integration`

Defined in `crates/raisin-core/global_nodetypes/raisin_integration.yaml`. Holds a
provider's configuration and its connected accounts.

| Property | Type | Notes |
|----------|------|-------|
| `title` | String (req) | Display name. |
| `provider_type` | String (req) | `google-drive`, `imap`, `sharepoint`, `custom`, … (indexed). |
| `adapter_function` | String (req) | Default adapter path in the `functions` workspace. |
| `oauth_config` | Object | `client_id`, `auth_url`, `token_url`, `scopes`, `redirect_uri`, optional `revoke_url`. **No secret here.** |
| `client_secret_encrypted` | String | base64 AES-256-GCM of the OAuth client secret. |
| `api_config` | Object | Non-OAuth providers: endpoints / key refs. |
| `connected_accounts` | Array | `[{ id, label, subject, expires_at, tokens_encrypted }]`. |
| `enabled` | Boolean | Default `true` (indexed). |

> **Correction (was: `client_secret_encrypted: "vault:google-oauth-secret"`).**
> There is **no vault**. Secrets are AES-256-GCM ciphertext, encrypted with a
> master key from the `RAISIN_MASTER_KEY` environment variable. The shared
> encryptor lives in the `raisin-crypto` crate (`SecretBox`), reused by AI
> providers, embeddings, and integrations.

`connected_accounts[].tokens_encrypted` is base64 AES-256-GCM of
`{ access_token, refresh_token }`. **Refresh tokens never appear in plaintext** in
any node property, API response, or function input. `expires_at` and `subject`
(the account email) stay plaintext so the refresh job and admin UI operate
without decrypting.

### `raisin:VirtualMount`

Defined in `crates/raisin-core/global_nodetypes/raisin_virtual_mount.yaml`. Binds
an integration/account to a target workspace subtree.

| Property | Type | Notes |
|----------|------|-------|
| `title` | String (req) | Display name. |
| `integration_ref` | String (req) | Path (or id) of the backing `raisin:Integration` (indexed). |
| `account_ref` | String | `connected_accounts[].id`; defaults to the first account. |
| `target_workspace` | String (req) | Workspace to materialize into (indexed). |
| `mount_path` | String (req) | Path prefix inside the target workspace. |
| `remote_root` | String | Provider-side root (folder id, mailbox, …). |
| `adapter_function` | String | Optional override of the integration default. |
| `mapping_function` | String | Optional custom mapper. |
| `sync_config` | Object | `{ mode, interval_seconds, include_patterns, exclude_patterns, ephemeral, ttl_seconds, max_items_per_sync }`. |
| `write_config` | Object | `{ writeback, conflict }` — **reserved; write-through is deferred** (see below). |
| `state` | Object | Engine-managed: `{ last_sync_token, last_sync_at, last_error, consecutive_failures, status, last_fencing_token }`. Do not hand-edit. |
| `enabled` | Boolean | Default `true` (indexed). |

`sync_config.mode` is `poll` | `webhook` | `hybrid`. `webhook` mounts are skipped
by the periodic driver (they are driven by inbound webhooks instead). Ephemeral
mounts (`ephemeral: true` + `ttl_seconds`) auto-delete stale nodes each sync — the
mailbox/event-stream pattern.

---

## Adapter framework

An adapter is a `raisin:Function` whose entrypoint takes **exactly one argument**.

> **Correction (was: `handler(event, context)` reading `context.metadata.credential`).**
> QuickJS calls the entrypoint as `handler(input)` — one object. The credential
> is a **field of that object**, not on `context.metadata` (which is not surfaced
> to JS). `raisin.context` remains available as a read-only global but carries no
> `event` here — the adapter is invoked directly, not via a trigger.

```javascript
function handler(input) {
  const { operation, params, credential, mount } = input;
  switch (operation) {
    case "capabilities": return { /* Capabilities */ };
    case "list":         return listItems(credential, mount, params);
    case "get_changes":  return getChanges(credential, mount, params);
    // get, get_content, create, update, delete …
  }
}
```

The eight operations, the `ExternalItem` / `Change` / `Capabilities` shapes, the
`credential` shape (no `refresh_token`), and the error-`code` convention
(`auth_expired`, `rate_limited`, `conflict`, else transient) are specified in full
in the [adapter reference](../reference/virtual-node-adapters.md). Adapter authors
should work from that document.

### Mapping

Each external item becomes a node. By default the engine applies a **built-in Rust
mapping** — no function call on the hot path:

- `is_folder` → `raisin:Folder`.
- everything else → `raisin:Node`, with `title` and a `meta` object carrying
  mime/size/urls and provider passthrough.

> **Correction (was: default file mapping is `raisin:Asset`).** The *built-in Rust
> default* maps files to **`raisin:Node`**, not `raisin:Asset`: `raisin:Asset`
> requires a binary `file` Resource, which a link-only v1 virtual node does not
> have. A mount that wants `raisin:Asset` (or any custom type) supplies a
> `mapping_function`. The shipped **google-drive** package *does* ship such a
> mapper (folders → `raisin:Folder`, files → `raisin:Asset` with `web_url` /
> `download_url` links).

A custom `mapping_function` is a `raisin:Function` called once per item with
`{ external_item, mount }`, returning `{ node_type, name?, properties }` or `null`
to skip the item.

### Reserved virtual metadata

The materializer stamps these on every synced node (plain properties → SQL works):

| Property | Meaning |
|----------|---------|
| `__virtual` | Marks a mount-managed node. |
| `__mount_id` | Owning mount node id. |
| `__external_id` | Provider item id — the stable upsert-match key (survives renames). |
| `__etag` | Provider change token at last sync (drives skip-write). |
| `__synced_at` | ISO 8601 timestamp of the last sync write. |

> **Correction (was: `__cached_at` / `__cache_ttl` caching properties).** There is
> **no cache layer** and no `__cached_at`/`__cache_ttl`. Virtual nodes are *real*
> materialized nodes, not cache entries; freshness is a function of the sync
> interval, and ephemeral cleanup uses `__synced_at` + `ttl_seconds`.

---

## How sync runs

> **Correction (was: sync driven by trigger entries on the mount node + cron; the
> sync loop is a function calling `raisin.functions.execute`).** Sync is a
> dedicated **Rust job handler**, driven by its own periodic job — it does **not**
> piggyback on `raisin:Trigger` nodes, and it invokes adapters **directly** (not
> via `raisin.functions.execute`, which would block a worker up to 5 minutes per
> nested call). See the [internals doc](./virtual-nodes-internals.md) for the full
> rationale and pipeline.

At a glance:

```
60s scheduler tick (raisin-server main loop)
   └─▶ VirtualMountSyncCheck            (scan repos → due mounts)
          └─▶ VirtualMountSync{mount}   (one job per due mount)
                 ├─ decrypt account credential (strip refresh_token)
                 ├─ invoke adapter directly  (get_changes / list)
                 ├─ map each item  (built-in default, or mapping_function)
                 └─ materialize via the normal write path
                        └─▶ node_event triggers, fulltext, SQL, audit, replication
```

- **First sync** (or a provider with no delta API, or a manual `mode: "full"`)
  runs a **full reconcile**: recursive `list`, upsert everything, delete
  mount-owned nodes not seen this pass.
- **Subsequent syncs** run a **delta**: `get_changes(since_token)`, page, apply.
- **Etag skip-write:** an item whose `__etag` is unchanged is not re-written —
  avoids revision churn and trigger storms.
- **Deletes are mount-scoped:** only nodes with a matching `__mount_id` are
  removed; a user-created node under the mount path is never touched.

A separate periodic job, **`IntegrationTokenRefresh`** (every ~10 minutes),
renews OAuth access tokens before they expire, entirely in Rust — refresh tokens
never enter the function sandbox.

---

## OAuth (connecting an account)

> **Correction (was: OAuth routes in `crates/raisin-server/src/routes/oauth.rs`).**
> That module already exists and is the **inbound** OAuth 2.1 *authorization
> server* for MCP clients — untouched here. Outbound "connect Google" endpoints
> are a separate module, `crates/raisin-transport-http/src/routes/integrations.rs`
> + `handlers/integrations/`, namespaced `/api/integrations/…`.

| Endpoint | Purpose |
|----------|---------|
| `POST /api/integrations/{repo}/oauth/start` | Admin-only. Body `{ integration_path }` → `{ auth_url, state }`. |
| `GET /api/integrations/{repo}/oauth/callback` | Provider redirect. Authenticated by the single-use, TTL'd `state` (not a bearer token). Exchanges the code, encrypts tokens, appends a `connected_accounts` entry, redirects to the console. |
| `POST /api/integrations/{repo}/oauth/disconnect` | Admin-only. Best-effort provider revoke + remove the account. |
| `POST /api/integrations/{repo}/mounts/{mount_id}/sync` | Admin-only. Enqueue a "sync now" (`mode` = `delta` \| `full`). |

The token exchange happens **server-side**; the client secret and the tokens
never leave the server in cleartext.

---

## Patterns

The same infrastructure powers several intents — the only difference is
configuration, not code.

| Pattern | Node lifecycle | `sync_config` | Example |
|---------|----------------|---------------|---------|
| Storage mount | Persistent, reconciled | `mode: poll` | Google Drive folder → `/documents/shared/…` |
| Device state | Persistent, updated | `mode: poll`, short `interval_seconds` | Philips Hue lights → `/devices/lights/…` |
| Event stream | Ephemeral, processed then expired | `ephemeral: true`, `ttl_seconds` | Incoming mail → `/inbox/messages/…`, an agent works the inbox |
| Webhook push | Ephemeral / pushed | `mode: webhook` | External SaaS `POST`s `/api/webhooks/{repo}/{id}`; a trigger materializes an event node |

Because materialization is a normal write, a WebSocket client subscribed to the
mount subtree receives live events as external changes sync in — the "agents work
the inbox" story needs no new agent infrastructure.

---

## What is deferred

- **Write-through** (`write_config.writeback: "write_through"`) — the property
  exists and the Google Drive adapter implements `create`/`update`/`delete`, but
  the engine does **not** yet propagate local edits back to the provider. Default
  is off; v1 mounts are read/reconcile-only.
- **On-demand resolution** — no `VirtualPathResolver` intercepts read-path cache
  misses to fetch a node lazily. v1 is background-sync only. **Deferred by
  decision** (original design Phase 7): intercepting reads means read paths can
  invoke adapter functions, which brings unbounded read latency and its own
  scoped-auth design problem — out of scope for v1.
- **Content download into the binary store** — adapters can implement
  `get_content`, but the engine syncs metadata + links, not bytes.

Now landed (previously listed here): the **IMAP/email adapter**
(`builtin-packages/imap-adapter/`, on the native `raisin.imap.*` binding), the **`raisindb create adapter`
CLI scaffold**, the **`raisin.integrations.sync_now`** host binding, the
**"Test connection"** endpoint, and the **admin-console** Connectors/Mounts pages.

See [`docs/virtual-nodes-implementation-plan.md`](../virtual-nodes-implementation-plan.md)
for the full landed-vs-deferred breakdown.
