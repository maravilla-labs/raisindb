# Virtual Nodes — Engine Internals

Audience: developers hacking on the sync engine itself, and operators running it.
For the concept overview see [`virtual-nodes.md`](./virtual-nodes.md); for the
adapter API see [`../reference/virtual-node-adapters.md`](../reference/virtual-node-adapters.md).

All paths below are relative to the repo root.

---

## 1. The job pipeline

The engine is a Rust job handler, `VirtualMountSyncHandler`
(`crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/`), plus a companion
token-refresh handler (`.../handlers/integration_token_refresh.rs`). Both are
built in `crates/raisin-rocksdb/src/storage/jobs/init_system/integration_handlers.rs`
and dispatched in `.../jobs/handlers/mod.rs` (`JobType::VirtualMountSyncCheck |
VirtualMountSync` → the sync handler; `JobType::IntegrationTokenRefresh` → the
refresh handler). The three job types are defined in
`crates/raisin-storage/src/jobs/types/job_type.rs` (default timeouts: the two
checks 60s, `VirtualMountSync` 600s).

```
raisin-server main loop, 60s tick   (crates/raisin-server/src/main.rs ~700-778)
   │  register_job_with_id_idempotent, dedup key "vmount-check:{minute-bucket}"
   ▼
VirtualMountSyncCheck                (check.rs::run_check)
   │  enumerate tenants → repos → raisin:system ws → enabled raisin:VirtualMount
   │  keep those where last_sync_at + effective_interval <= now, mode != "webhook",
   │  status != "auth_required"; enqueue one sync each,
   │  dedup key "vmount-sync:{mount_id}"  (a mount never has two in-flight syncs)
   ▼
VirtualMountSync{ mount_id, mode }   (mod.rs::run_sync)
   │  1. load MountConfig + IntegrationConfig  (config.rs)
   │  2. resolve adapter path (mount override → integration default)
   │  3. build_credential: decrypt tokens_encrypted, STRIP refresh_token
   │  4. acquire per-mount lease lock (if locks configured) → fencing token
   │  5. ephemeral TTL cleanup (ephemeral.rs) up front
   │  6. full::run (first sync / mode=full / no token) OR delta::run
   ▼
adapter invocation                   (adapter.rs::FunctionAdapterInvoker)
   │  FunctionExecutorCallback, DIRECT — never a nested FunctionExecution job
   │  input = { operation, params, credential, mount }
   ▼
mapping                              (mod.rs::map_item)
   │  built-in default_mapping (config.rs) OR the mount's mapping_function
   ▼
materializer                         (materializer.rs::RocksDbMaterializer)
   │  transactional upsert/delete as "virtual-mount-sync" (system privileges)
   ▼
normal transactional write path
   └─▶ node_event triggers, fulltext, SQL indexes, audit, replication
```

`SyncCtx` (in `mod.rs`) is the per-run bundle threaded into `delta.rs` / `full.rs`;
it borrows the invoker + materializer and exposes `call(operation, params)`,
`renew_lease(token)`, plus the free functions `map_item`, `materialize_item`, and
`persist_mount_state`.

### Delta vs full

- **`delta.rs`** — loops `get_changes(since_token)`, materializing each `Change`
  (deletes routed to `materializer.delete`, others through the include/exclude
  glob filter then `materialize_item`). It persists `state.last_sync_token`
  **after** each page is fully materialized, so a crashed page re-runs safely
  (upserts are idempotent via etag skip-write). Stops when the cursor stabilizes
  (`next_token` unchanged/absent) or `max_items_per_sync` is hit.
- **`full.rs`** — an explicit-stack recursive `list` from `remote_root`, upserting
  everything (glob-filtered) and recording each `external_id` as *seen*. After the
  walk it deletes every mount-owned virtual node **not** seen this pass
  (`materializer.list_virtual`). Finally it makes a best-effort `get_changes(null)`
  call to capture a delta baseline token so the next sync can go incremental;
  providers without a changes API leave the token unset and stay on the full path.

### Failure handling (`mod.rs::finalize`)

- `AdapterError::AuthExpired` → `state.status = "auth_required"`, mount is skipped
  by future checks until reconnect. Not retried.
- `AdapterError::RateLimited` / other → increment `consecutive_failures`, record
  `last_error`; at `DEGRADE_THRESHOLD` (5) the mount goes `degraded`. The effective
  interval backs off `interval * 2^min(failures, 5)`
  (`MountConfig::effective_interval_secs`). Success resets the counters.

`AdapterError::classify` (`adapter.rs`) maps a thrown adapter message onto these
variants by matching the reserved `code` substrings (`auth_expired`,
`rate_limited`, `conflict`, else `Transient`), because QuickJS surfaces a thrown
`Error` to Rust as a message string.

---

## 2. Why adapters are invoked directly, not via nested jobs

An adapter *is* a `raisin:Function`, so the obvious design would have the sync job
call `raisin.functions.call` / `raisin.functions.execute`. That was rejected:

- Those host bindings **block a job worker up to 5 minutes** per nested call
  (`MAX_WAIT_MS = 300_000`), because they enqueue a *separate* `FunctionExecution`
  job and wait for it. There is **no recursion-depth guard**.
- A mount syncing thousands of items — each a nested call — would exhaust the
  worker pool and stall every other job.

Instead, `raisin-rocksdb` cannot depend on `raisin-functions`, so the engine takes
a `FunctionExecutorCallback` by **dependency inversion** — the same
`Arc<dyn Fn(...) -> Pin<Box<dyn Future<...>>>` the `FunctionExecutionHandler`
already receives (defined at `.../jobs/handlers/function_execution.rs`). The
production `AdapterInvoker` (`FunctionAdapterInvoker`) calls that callback
**inline**: the adapter executes in the same async task, no extra job, no
worker-pool blocking. `raisin.functions.call` remains available for user-authored
orchestration but is off the sync hot path.

Adapters are invoked with a `None` auth context — see §6.

---

## 3. The etag skip-write rule

In `materializer.rs::upsert`, once the existing node is located, if the incoming
`virt.etag` is `Some` and equals the node's stored `__etag`, the upsert returns
`Ok(false)` **without writing**. This is load-bearing:

- No new MVCC revision is created for an unchanged item.
- No `node_event` fires — which is what keeps a re-sync from re-triggering every
  downstream workflow/agent (the "trigger storm" hazard, §5).

Adapters make this work by returning a `etag` that is **stable when nothing
changed**. The Google Drive adapter uses the file's monotonic `version` counter
for exactly this reason (`toExternalItem` in the adapter `index.js`).

---

## 4. `__external_id` rename matching

`materializer.rs::upsert` resolves the target node in two steps:

1. **Match by `__external_id` within the mount subtree** — scan the workspace for a
   node whose `__mount_id` == this mount **and** `__external_id` == the incoming
   item, under `mount_path`. If found, update it **in place, preserving its id and
   current path**. A provider-side rename/move therefore updates the existing node
   instead of creating a duplicate.
2. **Fallback to a path match** — if no `__external_id` match, look at the target
   path. A **foreign** node (different or absent `__mount_id`) occupying that path
   is **not** clobbered (logged + skipped); a mount-owned node there is updated;
   otherwise a fresh node id is minted.

`delete` and `list_virtual` are likewise scoped by `__mount_id` + `mount_path`, so
the engine never deletes user-created nodes that happen to live under the mount.

> **Implementation note / divergence.** Matching is currently done by
> `tx.scan_nodes(workspace)` + a linear `find`, **not** by a property-index lookup.
> The reference doc's suggestion to index `__mount_id` / `__external_id` remains
> good advice for *your own* SQL against virtual nodes, but the engine's
> upsert-match itself does a full workspace scan today. This is a known scaling
> ceiling for very large target workspaces — a candidate for an indexed lookup
> later.

---

## 5. Cluster safety: fencing tokens + lease locks

The 60s scheduler runs on **every** node, and job dispatch is per-node, so two
nodes can race the same mount. Two mechanisms defend against double-sync:

1. **Idempotent registration.** Every enqueue uses
   `register_job_with_id_idempotent` with a dedup key
   (`vmount-check:{minute}`, `vmount-sync:{mount_id}`,
   `token-refresh:{10-min-bucket}`). This collapses most duplicates: a mount never
   has two in-flight sync jobs registered.

2. **Per-mount lease lock with a fencing token** (`raisin-locks`,
   `crates/raisin-locks/src/lib.rs`). At the start of `run_sync`, when a
   `LockManager` is configured, the engine calls
   `try_acquire("{tenant}\0{repo}\0{branch}\0vmount:{mount_id}", owner, 600s)`:
   - `None` (held elsewhere) → **exit successfully as a no-op** (not a retry).
   - `Some(guard)` → the guard's monotonic **fencing token** is written into
     `state.last_fencing_token` alongside `last_sync_token`.
   - The state-persist path (`persist_mount_state`) reads the currently-stored
     token first and **skips its write if the stored token is newer** — a
     GC-stalled sync resuming after its lease expired cannot clobber a newer
     sync's cursor. This is the classic stale-fencing-token guard.
   - The lease is `renew`ed between pages during long full reconciles and
     `release`d on completion.

### Operator rule (this will bite you)

`LockManager` is **optional** — in `crates/raisin-server/src/main.rs` it is an
`Option<LockManagerHandle>`. When it is `None`:

- The engine falls back to **dedup-key-only** semantics (single-node correct).
- If replication is active, `run_sync` logs a warning that sync is **not**
  cluster-safe.

**A multi-node cluster MUST configure the Redis locks backend**
(`[locks] backend = "redis"`, built with `--features locks-redis`). The
`inprocess` backend is single-node only; with `inprocess` + multiple nodes, mounts
can **double-sync**. This is the same deployment rule that already governs
inventory/lease locks.

> The periodic **check** driver itself does not take a lock in the shipped code —
> it relies on the per-minute dedup bucket to collapse concurrent scans, and the
> per-mount `vmount-sync:{mount_id}` dedup + lease to make the enqueued syncs
> safe. (The original plan suggested an additional `vmount:check` lease; it was not
> needed given the dedup bucket.)

---

## 6. Security model

**Adapters run privileged.** `FunctionAdapterInvoker::invoke` passes a `None` auth
context to the executor, which resolves to a **system context with RLS bypassed**
(the same behavior trigger-invoked functions already have). Materialization writes
also run with system privileges, as actor `"virtual-mount-sync"`.

The identity is stamped in two places, and both matter. The transaction's raw
actor becomes `RevisionMeta.actor`; the transaction's **auth context** is what
the write path stamps into the node's `created_by` / `updated_by`, and it takes
precedence over the raw actor. The sync therefore uses
`AuthContext::system_as(SYNC_ACTOR)` — full system privileges, but an honest
identity — so a synced node, its audit-log rows and its emitted `node:*` events
all name `virtual-mount-sync` rather than `system`. Nodes written before this
was fixed are still attributed to `"system"`; there is no backfill.

So:

- An adapter package is **highly privileged code**. It is acceptable in v1 because
  adapters are **admin-installed** — treat installing an adapter like installing a
  server plugin.
- Downstream `node_event` triggers fired by sync writes also run privileged
  (existing behavior). Keep mount-scoped trigger filters specific.

**Refresh tokens never enter the sandbox.** `build_credential`
(`adapter.rs`) hands the adapter `{ access_token, account_id, provider_type }` plus an
optional `username` (the connected account's verified `subject`, for username/secret
adapters like IMAP) — and, structurally, it **cannot** include `refresh_token`. Refresh tokens
are decrypted **only** in the Rust token-refresh handler
(`integration_token_refresh.rs`), used to POST the refresh grant, re-encrypted, and
written back. They never appear in a function input, an API response, or a log
line. The same holds for the OAuth **client secret**: it is decrypted only in the
callback/refresh Rust paths and is never echoed.

**Encryption.** All secrets at rest use `raisin-crypto`'s `SecretBox`
(AES-256-GCM, wire format `[nonce(12)][ciphertext+tag]`, base64 for JSON blobs).
The master key is loaded via `master_key_with_embedding_fallback()`
(`RAISIN_MASTER_KEY`, falling back to the legacy `EMBEDDING_MASTER_KEY` so existing
embedding deployments keep decrypting). See §7 for the operational caveat.

**`network_policy` pins adapters to provider hosts.** Adapter function nodes ship
a `network_policy` restricting outbound HTTP to the provider's hosts — the Google
Drive adapter's `.node.yaml` allows only `www.googleapis.com/**` and
`oauth2.googleapis.com/**`. This bounds the blast radius of privileged adapter
code. New adapters should do the same.

---

## 7. Operational notes (read before deploying)

1. **`RAISIN_MASTER_KEY` is load-bearing and has no rotation story.** It already
   gates AI-provider keys; with integrations it gates *all* external-system access
   (client secrets + OAuth tokens). Lose it and every stored secret becomes
   undecryptable; there is currently **no key-rotation mechanism** — rotating it
   invalidates all existing ciphertext. Provision it from a secrets manager and
   back it up out-of-band. (The `EMBEDDING_MASTER_KEY` fallback exists only for
   backward compatibility; prefer `RAISIN_MASTER_KEY`.)

2. **Multi-node → Redis locks are mandatory.** As in §5: without
   `[locks] backend = "redis"`, a cluster can double-sync a mount. `inprocess` is
   single-node only.

3. **First sync of a large folder is a trigger storm.** A full reconcile of a
   10k-item Drive folder emits ~10k node events (one materialize per item), each
   firing whatever `node_event` triggers match. Etag skip-write suppresses
   *repeat* storms on later syncs, but the **initial** sync is unavoidably noisy.
   Mitigations: make mount-scoped trigger filters specific, and stage large mounts
   deliberately. `max_items_per_sync` caps items per run but the aggregate is still
   emitted across runs.

4. **Webhook lookup is a linear scan.** Webhook-mode sync relies on
   `/api/webhooks/{repo}/{id}`, whose `webhook_id` is resolved by a linear scan
   over all `raisin:Trigger` nodes (`handlers/webhooks/lookup.rs`). Fine at current
   scale; a hot path if thousands of mounts use webhook mode — flagged for an index
   later.

5. **The OAuth `state` store is in-process.** `handlers/integrations/state_store.rs`
   holds `state` values in an in-memory, TTL'd (600s), single-use map. A multi-node
   deployment behind a load balancer must pin an OAuth start/callback pair to the
   same node. The flow is a one-shot admin action, so this is an acceptable v1
   constraint.

6. **Manual "sync now" hard-codes branch `main`.** The
   `/mounts/{mount_id}/sync` handler enqueues with `branch = "main"`, whereas the
   periodic check uses the repo's configured default branch. On a repo whose
   default branch is not `main`, prefer the periodic path or set the mount up on
   `main`. (Minor divergence; noted for anyone extending the endpoint.)

---

## 8. Capability resolution and caching

The sync loop **never invokes the adapter's `capabilities` operation** — grep the
engine and you will find zero call sites. The flags the engine actually needs at
sync time (`supports_changes`, the ephemeral TTL) are read from the mount's loaded
config (`config.rs`), not re-fetched per run. This keeps the hot path free of an
extra adapter round-trip.

`capabilities` is instead resolved by the **"Test connection"** endpoint,
`POST /api/integrations/{repo}/test`
(`crates/raisin-transport-http/src/handlers/integrations/test_connection/`). That
handler:

1. Resolves the adapter node and (optionally) an account credential — with
   `refresh_token` stripped by `support::build_credential`, same invariant as the
   sync engine.
2. Invokes `capabilities`, then a **bounded `list` probe** (`PROBE_LIMIT = 10`
   items, whole call under a `PROBE_TIMEOUT = 30s` ceiling), and returns a
   structured diagnostic (`ok`, `latency_ms`, `auth`, `capabilities`, `probe`,
   `error`) — a failed connection is still HTTP `200`; it is a diagnostic result,
   not a server error. The probe `sample` carries item **names only**, never URLs
   (which can embed tokens).
3. On success, **caches the report onto the `raisin:Integration` node**:
   `support::cache_capabilities` writes `properties.capabilities` (the full object)
   and `properties.capabilities_checked_at` (ISO 8601). Both properties are
   declared on `raisin_integration.yaml`.

The admin console reads the cached `capabilities` to **drive connector form
visibility** (hide write controls when `can_write` is false, hide the TTL field
unless `default_ttl` is set, etc.). So the report is a UI/diagnostic contract, not
a sync input — its honesty is what the operator sees, and "Test connection" is the
first thing to reach for when debugging a new adapter.

---

## 9. The branch model (config branch vs `target_branch`)

Virtual-node sync spans **two branches**, and conflating them is the easiest way to
get a phantom double-sync:

- **Config branch = the repo's default branch.** `check.rs::check_repo` resolves
  `storage.get_repository(...).config.default_branch` (falling back to `main`) and
  scans the `raisin:system` workspace **on that branch only** for
  `raisin:VirtualMount` nodes. It never scans any other branch.
- **`target_branch` = where nodes materialize.** Each mount carries a
  `target_branch` property (`config.rs:141`, declared on
  `raisin_virtual_mount.yaml`). `mod.rs::run_sync` validates that branch exists,
  then materializes virtual nodes into it (`MountScope.branch = target_branch`).
  Adapter invocations and mount-state writes stay on the config branch; only the
  materialized content lands on `target_branch`. A missing `target_branch` marks
  the mount misconfigured and skips it (no crash).

**Why forked branches are inert.** Because the check driver scans *only* the config
(default) branch, forking a branch that happens to contain a copied mount config
node — cursor, fencing token, and all — does **not** spawn a second sync. The
scanner simply never looks at the fork. This is deliberate: it means branch
operations (fork/merge/publish) can copy the `raisin:system` workspace freely
without accidentally arming a duplicate sync engine on the copy. To actually sync
into a different branch, set a mount's `target_branch` — do not fork the config
node onto that branch expecting it to run.

---

## 10. The delta cursor retention rule

`delta.rs` guards the mount's `last_sync_token` against being clobbered by a
falsy adapter cursor. On each page it inspects the adapter's returned `next_token`:

- `Some(token)` → advance: `state.last_sync_token = Some(token)`, persisted after
  the page is fully materialized (so a crashed page re-runs safely; upserts are
  idempotent via etag skip-write).
- `None` → **stop paging but retain the existing cursor.** The engine does *not*
  overwrite `last_sync_token` with `None`. Overwriting it would clear the cursor and
  silently force a full resync on the next run.

This is the engine-side half of the adapter contract's **"never return
`next_token: null` to mean no changes"** rule
([reference §2.8](../reference/virtual-node-adapters.md#28-get_changes)). A
well-behaved adapter returns the *unchanged* cursor when there is nothing new; the
retention logic in `delta.rs` is defense-in-depth for adapters that get it wrong.
Loop termination does not depend on a `null`: paging also stops when the cursor
**stabilizes** (`next == token`) or `max_items_per_sync` is reached.

---

## 11. Native protocol bindings

An adapter is an ordinary server-side function, so its reach to the outside world is
whatever the `raisin.*` host API exposes. **HTTP covers most connectors**:
`raisin.http.fetch` is the only network egress the sandbox grants, and REST / JMAP /
GraphQL providers all ride on it — those adapters are pure JS/Starlark, no Rust.

When a provider speaks a *protocol* HTTP cannot carry (a raw TCP/TLS protocol), we add
that capability **natively in Rust** and expose it as a new `raisin.<ns>.*` host API.
**IMAP is the first native binding.** The IMAP adapter (`builtin-packages/imap-adapter/`)
originally spoke JMAP-over-HTTP only because no raw IMAP socket existed in the sandbox;
`raisin.imap.{fetchSince,listMailboxes,fetchMessage}`
(`crates/raisin-functions/src/runtime/imap/`) closes that gap — Rust owns the IMAP
protocol (TLS + `LOGIN` + `UID FETCH`) and the adapter calls high-level ops.

The pattern is deliberately **extensible**: a single Rust implementation reaches both
function runtimes (QuickJS and Starlark), and every native binding is gated by the same
`network_policy` egress rule as `raisin.http.fetch` — the IMAP binding requires the
connection's `imaps://host:port` to match the function's `allowed_urls` before it opens a
socket (`api/raisindb/imap.rs::authorize_imap`), and credentials are never logged. Adding
a new protocol is a documented, repeatable procedure — see
[Adding a Native Host Capability](../guides/adding-a-native-host-capability.md).

---

## 12. Provider config passthrough (api_config + full sync_config)

> The Gmail / Microsoft 365 (Graph) / Google Calendar connectors that rely on this are an
> **experimental / preview** feature. The passthrough mechanism itself is not experimental.

An adapter needs the mount's provider-specific configuration to reach an external system —
IMAP host/port/tls, a Graph `resource` selector, a calendar time `window`, and so on. Two
config objects reach every adapter in the `mount` snapshot, both **forwarded verbatim**:

- **the full `sync_config`** — every key the `raisin:VirtualMount` was authored with, not
  a fixed subset;
- **`api_config`** — the owning `raisin:Integration`'s `api_config` object, carrying
  connection *defaults*.

Both are assembled by `build_mount_snapshot`
(`crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/adapter.rs`) from
`MountConfig.sync_config_raw` and `IntegrationConfig.api_config`
(`.../virtual_mount_sync/config.rs`). `sync_config_raw` is the raw authored object, kept
separately from the typed `SyncConfig` (which captures only the fields the engine itself
acts on).

**This used to be the blocker.** The earlier `build_mount_snapshot` forwarded only a
hard-coded ~7-key whitelist of `sync_config` and did **not** forward `api_config` at all,
so an adapter could never receive host/port/tls/mailbox/username/auth or any
provider-specific key. Verbatim passthrough of both objects is what makes provider
adapters possible. The parallel `credential.username` fix (from the connected account's
verified `subject`, see §6) is what lets username/secret adapters authenticate.

How the shipped connectors consume it:

- **Gmail / IMAP** (`builtin-packages/imap-adapter`) merges `api_config` (template
  defaults: `host` / `port` / `tls` / `default_mailbox` / `auth`) **under** the mount's
  `sync_config` (per-mount override wins per key), and reads the login identity from
  `credential.username`. Gmail sets `auth: xoauth2`, so the OAuth access token is used as
  the IMAP SASL bearer secret.
- **Microsoft 365 / Graph** (`builtin-packages/ms-graph-adapter`) reads
  `sync_config.resource` (`"mail"` default vs `"calendar"`) to pick the Graph surface and
  `sync_config.window` `{ days_ahead, days_back }` to bound the `calendarView` delta.
- **Google Calendar** (`builtin-packages/google-calendar-adapter`) reads
  `sync_config.window` for the `timeMin`/`timeMax` bounds and `remote_root` for the
  calendar id (default `"primary"`).

---

## 13. Push notifications (Experimental / Preview)

> Push is an **experimental / preview** feature. The lifecycle below is fully generic:
> any user-authored connector gets push by implementing the same three optional adapter
> ops — the shipped Graph / Calendar / Gmail connectors are just examples.

**The whole model in one line: a push notification is only an _invalidation signal_.** The
engine never inspects a provider's notification payload. A ping means *"re-run this mount's
normal delta sync"* — the notifications endpoint maps the ping to a mount and enqueues an
ordinary `VirtualMountSync { mode: "delta" }`; the mount's existing `get_changes` does the
work. This is what collapses Microsoft Graph subscriptions, Google Calendar channels, and
Gmail Pub/Sub onto one code path. Every provider quirk lives in **adapter JS** and in the
**shape-matching** notifications endpoint, never in the engine.

### The generic lifecycle

Three optional adapter ops (`subscribe` / `renew` / `unsubscribe`, reference §2.9) and one
generic subscription driver (`virtual_mount_sync/subscription.rs`) do all the work:

1. **Bootstrap.** `check.rs::is_due` returns `false` for a `webhook`-mode mount **except**
   when it has no live subscription — then it enqueues exactly **one** sync run. That run's
   `subscription::ensure` step (called from `mod.rs::run_sync`) checks the adapter's
   `Capabilities.supports_push`; if `true` and there is no active non-expired subscription, it
   builds the per-mount URL and calls the adapter's `subscribe`. `hybrid` mounts keep polling on
   their interval **and** maintain a subscription (a safety net for missed pings). `poll` mounts
   never subscribe.
2. **The per-mount URL.** `subscription::ensure_mount_token` mints a stable, unguessable
   `push_mount_token` of the form `{mount_id}.{nanoid(32)}` and persists it in the mount's
   engine-managed `state`. `notification_url(base, repo, token)` assembles
   `{base}/api/integrations/{repo}/notifications/{mount_token}`, where `{base}` is
   `RAISINDB_BASE_URL` (read via `configured_base_url()` — the engine can't depend on the HTTP
   crate, so it reads the same env var transport does). **No base URL → push cannot be wired**:
   the mount is marked `push_status = "failed"` with an explanatory `push_last_error`.
3. **State.** `subscribe`'s result is stored in `MountState` (`config.rs`): `push_subscription_id`,
   `push_secret`, `push_expires_at`, `push_notification_url`, `push_mount_token`, `push_status`
   (`"active" | "failed" | "unsupported"`), `push_last_error`. `has_active_push(now)` = status
   `active` + a subscription id + a future (or absent) expiry — this is what makes `ensure`
   idempotent and gates re-bootstrap.
4. **Pings.** A provider POST hits the public endpoint
   (`handlers/integrations/notifications.rs`). It runs the validation-echo / token-match /
   secret-verify contract (reference §2.9 "Notifications endpoint contract"; all shape-driven,
   zero provider identity) and, on success, enqueues the deduped delta sync and acks `200`
   immediately — never blocking on the sync.
5. **Renewal.** `main.rs` registers a `VirtualMountSubscriptionRenew` job on a ~30-min bucket
   (dedup `vmount-sub-renew:{bucket}`). Its handler renews any active subscription expiring within
   `RENEW_WINDOW_SECS` (1 day) via the adapter's `renew` — provider subscriptions are short-lived
   (Graph ~3d, Google ~7d). A failed renew marks the mount `failed` so the periodic check
   re-bootstraps a fresh subscription through `ensure`.
6. **Teardown.** Disabling a push mount calls `subscription::teardown` → adapter `unsubscribe`
   (best-effort; errors are logged and ignored — the provider expires it anyway). The stable
   `push_mount_token` is retained so a later re-enable reuses the same URL.
7. **Unsupported.** A `webhook` mount whose adapter reports `supports_push: false` is marked
   `push_status = "unsupported"` once (with a warning) and never re-bootstrapped — it would
   neither poll nor push, a misconfiguration the admin must fix (switch to `hybrid`/`poll`).

Subscription/renew/unsubscribe adapter calls use a throwaway `SyncCtx` (`subscription::adapter_ctx`)
that takes **no lease** (`lock_manager: None`) — they are short and idempotent, unlike a sync run.

### How the shipped connectors map onto it

- **Microsoft Graph** (`ms-graph-adapter`) — `subscribe` POSTs `/subscriptions` with the mount's
  `notificationUrl` and a random `clientState` returned as `secret`; the resource path comes from the
  mount's `resource` (`/me/mailFolders/{id}/messages`, `/me/events`, or `/me/drive/root` for
  OneDrive files). `renew` PATCHes a fresh `expirationDateTime`; `unsubscribe` DELETEs. Graph's
  `validationToken` handshake is echoed by the RaisinDB endpoint, **not** the adapter; `clientState`
  is the endpoint's secret check.
- **Google Calendar** (`google-calendar-adapter`) — `subscribe` opens an `events.watch` channel with
  `address = notificationUrl` and a channel `token` reused as `secret`. Two Google quirks are handled
  in adapter JS: `channels.stop` needs both the channel id **and** the opaque `resourceId`, and every
  `watch` mints a new channel — so the adapter packs `{channelId}\t{resourceId}\t{secret}` into
  `subscription_id` and **reuses the same secret across renews**, keeping the engine-stored
  `push_secret` valid for the channel's whole life (the engine only stores `secret` on `subscribe`,
  not on `renew`). Google's channel `sync` message is the bare-`GET` case the endpoint acks with `200`.
- **Gmail (Pub/Sub)** (`imap-adapter`) — push is offered **only** when the mount sets
  `sync_config.pubsub_topic`; a plain IMAP mount reports `supports_push: false` and keeps polling.
  `subscribe` arms `users.watch` against the operator's Pub/Sub topic (reusing the XOAUTH2 OAuth token
  as the REST bearer); `unsubscribe` calls `users.stop`. The adapter owns only the `watch`/`stop` hop
  — it can never create the topic or the Pub/Sub push subscription, so the operator must create those
  and point the push subscription at the mount's notifications URL (see the website guide). The
  Pub/Sub message body (`historyId`) is ignored — the ping is a pure invalidation. The adapter exposes
  `sync_config.pubsub_verify_token` as the shared secret the endpoint checks (via the `token` carrier);
  an operator who instead configures an OIDC-authenticated push subscription can verify the signed JWT
  with `raisin.crypto.verifyJwt`.

### `raisin.crypto.verifyJwt` — signed-push verification

`crypto.verifyJwt(token, opts)` (`crates/raisin-functions/src/api/raisindb/crypto.rs`, shared
descriptor in `runtime/bindings/methods/crypto.rs`) is the generic OIDC/JWT-verify primitive for
**signed** push (Gmail Pub/Sub OIDC push, and any future provider that signs its callbacks). Given
`opts = { jwks_url, issuer?, audience?, algorithms? }` it authorizes `jwks_url` against the function's
`network_policy` **before opening any socket** (same gate as `raisin.imap` / `raisin.http`), fetches
the JWKS (process-wide cache, 300s TTL, 10s fetch timeout), then delegates to a pure offline verifier
(`runtime::crypto::verify_with_jwks`). It returns `{ valid, claims?, error? }` and **never throws on a
bad token** — an invalid token is `{ valid: false, error }`; a hard `Err` is reserved for policy denial
and an unreachable/invalid JWKS. Tokens and claims are never logged. It reaches both runtimes (QuickJS
`raisin.crypto.verifyJwt`, Starlark `raisin.crypto.verify_jwt`) from the one impl.
