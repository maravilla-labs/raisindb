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
| `write_config` | Object | `{ mode, mutable_fields, conflict, writeback, command_node_types, create_node_types, delete_policy, move_policy }`. `mode` is `off` \| `state_only` \| `mirror` \| `submit`; a mode the adapter's capabilities cannot serve is refused with a reason rather than demoted. A blob that is present but does not parse fails the mount loudly rather than silently disabling writeback. See [The write path](#the-write-path). |
| `state` | Object | Engine-managed: `{ last_sync_token, last_sync_at, last_error, consecutive_failures, status, last_fencing_token }`. Do not hand-edit. |
| `enabled` | Boolean | Default `true` (indexed). |

**Mount bundles.** A connector whose resources need different write modes
(an outbox beside a read-only ledger beside a two-way catalogue) needs one mount
per resource, and each is a dozen adapter-specific values. The connector
template can carry that set as `raisin:Integration.mount_bundles`; the admin
console's *Add bundle* (Mounts page) asks for connection, workspace and root
folder, checks the workspace's `allowed_node_types` against what the bundle
materialises, and creates ordinary `raisin:VirtualMount` nodes from it
(`planBundle` in `packages/admin-console/src/api/integrations.ts`). Nothing
server-side reads the property. Stripe and Microsoft 365 are the two reference
examples.

Schema v5 added the two things a single-workspace, no-questions preset could not
express. An entry may name its own `target_workspace` (and `root_override`), so
the Microsoft 365 bundle puts mail in `workplace` and drive files in `assets`
beside every other asset; each destination is gated separately. And a bundle may
declare `prompts` — the values only the operator knows (which mailbox, which
SharePoint site), asked once and written onto every entry that lists the prompt
in `applies_to`, at a target from the closed set `sync_config.<key>` /
`remote_root` / `account_ref`.

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

Beyond the core operations there are two optional groups, each gated on a capability
flag and neither on the sync path: the **push lifecycle**
(`subscribe` / `renew` / `unsubscribe`, `supports_push`) and **`browse`**
(`supports_browse`), which lets the admin console list a provider's mail folders,
calendars, SharePoint sites and drives so an operator *picks* a mount's remote root
instead of pasting a provider id. An adapter that implements neither is fully
supported; the console simply keeps its free-text inputs.

The operations, the `ExternalItem` / `Change` / `Capabilities` / `BrowseItem` shapes, the
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

On the write path the mapper is **bidirectional**, dispatching on
`input.operation`: `to_node` (the call above, unchanged and still the default when
no operation is given) and `to_external` (`{ node, mount, fields? }` → a provider
payload). Both directions stay in one function node so they cannot drift. The
engine calls `to_external` today, for `state_only` mounts — see
[The write path](#the-write-path).

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

## The write path

> **Status.** One mode ships: `state_only`, which pushes a declared allow-list of
> fields (mail's `unread` is the worked example) through the adapter's `update`,
> as the first step of each sync run. `mirror` and `submit` are not built — a
> mount asking for `writeback: "write_through"` still records
> `state.writeback_supported: false`, naming what is missing, and the console
> still hides the controls. The rest of this section is the agreed design — read
> it before implementing adapter write operations so what you build matches what
> the engine will call. Full contract:
> [`docs/reference/virtual-node-adapters.md`](../reference/virtual-node-adapters.md) §10.
> Staging: [`docs/virtual-nodes-implementation-plan.md`](../virtual-nodes-implementation-plan.md).

### Three write modes, chosen per collection

The generalization that makes one engine serve calendars, mailboxes, files and
anything written later: **write mode is a property of the mount/collection, not
of the adapter.** The same IMAP adapter serves a `state_only` inbox mount and a
`submit` outbox mount.

| mode | the node is… | a local change means | example |
|------|--------------|----------------------|---------|
| `mirror` | the remote object | create/update/delete propagate | calendar event, Drive file |
| `state_only` | an immutable record with mutable state | only declared `mutable_fields` propagate | mail: body immutable, read/flags/folder are not |
| `submit` | a **command** | creating it and queueing it issues the command, once | send / reply / forward, RSVP |

`submit` is what makes immutable resources coherent. An email cannot be edited,
so its write path is a *sending* path — and the home for that is a separate mount
whose members are intents rather than mirrors:

```
/mail/inbox    mode: state_only   raisin:Mail
/mail/sent     read-only          raisin:Mail          <- canonical sent message
/mail/outbox   mode: submit       raisin:OutboundMail  <- commands
```

Reply and forward then need no special casing — the outbox node carries the action
and the provider's own message id. The existing `ephemeral` + `ttl_seconds`
machinery garbage-collects completed commands for free. The same shape serves any
future connector: a chat outbox, a refund queue, an order submission mount.

### The engine/adapter boundary

The write path is deliberately thin and domain-blind. The engine knows "call
`update` with these fields"; it does **not** know what a calendar is or that mail
bodies are immutable. That is the adapter package explaining itself through
`capabilities` (notably `mutable_fields`), its nodetypes, and its docs.

| Layer | Owns |
|-------|------|
| **Engine** (Rust, generic) | change detection, ordering, the mount lease, intent lifecycle, the already-pushed check, metadata stamp-back, safety rails, at-most-once semantics, error classification |
| **Adapter package** (JS, per provider) | the remote API calls, node↔provider translation, declared capabilities, the conflict resolver |
| **Convention** (per package) | which nodetypes, which collections, outbox layout, mount templates |

**Adapters never write nodes.** An adapter takes a request, hits the provider, and
returns a result; the engine performs every local write. Delegating that would
lose lease serialization, the stamp-back that prevents sync loops, the destructive
-operation rails, and the sandbox boundary — adapters run privileged with a system
auth context, so an adapter that could write nodes could write *any* node.

### Mapping becomes bidirectional — in one function

`mapping_function` gains a second direction, dispatched by `operation`:
`to_node` (`external_item → node`, exactly as today) and `to_external`
(`node → provider payload`).

Both live in the **same function node**, and that is load-bearing. The mapper is
separate from the adapter precisely so node shape can be customized without
forking the adapter — so if the reverse translation were hardcoded inside the
adapter, pointing a mount at a custom mapper would make it write the wrong fields
silently. One relationship expressed twice in two files is exactly the drift this
codebase pays for most often.

A mapper without `to_external` makes its mount read-only, recorded in
`state.writeback_supported` so the console can explain *why*. Writability is a
property of the **mount** — adapter and mapper together.

### The two mechanisms that keep it safe

- **Loop prevention is the etag stamp-back, not actor filtering.** After a push,
  the provider's new etag is stamped back under the sync actor, so the next delta
  returning that item hits the existing skip-write and writes nothing. For fields
  a provider's etag does not cover (IMAP `\Seen`), a `__pushed_state` companion
  records what was actually pushed.
- **`submit` is at-most-once and never auto-retried.** A retried send is a
  duplicate email. The command is durably marked `sending` *before* the call; an
  ambiguous failure parks at `unknown` for a human rather than retrying. Only
  `rate_limited` requeues, because it is the only error proving no side effect
  occurred.

Delete and move are per-collection policy (`detach` | `trash` | `purge` and
`push` | `detach` | `reject`) with adapter-declared defaults, and destructive
writes are bounded by proportional blast-radius rails so a mis-scoped bulk
statement cannot reach the provider.

---

## What is deferred

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
