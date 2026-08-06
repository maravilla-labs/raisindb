# Virtual Node Adapters — Reference Contract

**Status:** frozen. This is the canonical, package-facing API surface for external
virtual-node adapters. Adapter packages, the sync engine, and the admin console all build
against it. If you are writing an adapter, you can implement everything here without reading
the implementation plan.

An **adapter** is a `raisin:Function` (JavaScript/QuickJS or Starlark) that translates a
single normalized operation into calls against one external system (Google Drive, IMAP,
SharePoint, a custom API, …). The RaisinDB **sync engine** invokes your adapter directly —
never through a trigger — decrypts the account credential immediately before the call, and
materializes the results into nodes under a mount path.

> **Experimental — the shipped Gmail / Microsoft 365 (Graph) / Google Calendar connectors
> are a preview feature.** The adapter *contract* on this page is frozen and stable; the
> three provider packages built against it (`builtin-packages/{imap-adapter,
> ms-graph-adapter, google-calendar-adapter}`) and the `raisin:Event` calendar mapping
> target (§6.1) ship as **Experimental / Preview**. Validate them against your own account
> before relying on them in production.

> **The write path (§10) is partly implemented: `state_only` is built and called.** For a
> mount configured `write_config.mode: "state_only"`, the engine calls your `update` —
> restricted to the fields both the mount and your `capabilities.mutable_fields` name. If you
> want a mount to push local edits, you must implement `update` and declare `can_write` /
> `can_update` / `mutable_fields`; a mount whose adapter does not is reported unwritable with
> the reason, in `state.writeback_supported` / `state.writeback_last_error`.
>
> The rest of §10 is designed, not yet implemented: `create`, `delete`, `submit` and
> `get_content` are never invoked today, and the `mirror` and `submit` modes are refused with
> a reason rather than run. The whole write path is additive — nothing on the read side
> changes, and every existing read-only adapter and mapper keeps working untouched.

---

## 1. Handler shape

The adapter entrypoint receives **exactly one argument**, an object with four keys. QuickJS
entrypoints take one argument (`handler(input)`); Starlark adapters receive the same object.

```javascript
function handler(input) {
  const { operation, params, credential, mount } = input;
  // ... dispatch on operation, return the operation's result ...
}
```

### 1.1 `input` object

| Key | Type | Notes |
|-----|------|-------|
| `operation` | String | One of the operations in §2. |
| `params` | Object | Operation-specific arguments (see each operation's table). |
| `credential` | Object \| null | Decrypted account credential. `null` for adapters/operations that need none (e.g. a public API). |
| `mount` | Object | Read-only snapshot of the mount config (§1.3). |

### 1.2 `credential` object

Decrypted by the sync engine **just before invocation** and passed in memory only.

```javascript
credential = {
  access_token: "ya29....",       // current, valid access token
  account_id: "acct_01H...",       // connected_accounts[].id it came from
  provider_type: "google-drive",   // integration provider_type
  username: "alice@example.com",   // OPTIONAL — the connected account's verified
                                   //   subject (email); present only when the
                                   //   account has one. See below.
  // NO refresh_token field — see the boxed rule below.
}
```

`username` is the connected account's **verified subject** — the account email captured
at OAuth-callback time (`connected_accounts[].subject`). It is forwarded only when the
account actually has a subject, so treat it as optional. Adapters that authenticate with a
username/secret pair (e.g. IMAP `LOGIN` or SASL `XOAUTH2`, where the access token is the
secret) key on it; pure bearer-token REST adapters can ignore it.
`build_credential` in `crates/raisin-rocksdb/src/jobs/handlers/virtual_mount_sync/adapter.rs`
constructs this object.

> **`refresh_token` is NEVER passed to the adapter.** Refresh tokens are stored only as
> AES-256-GCM ciphertext inside the integration node and are used exclusively by the
> engine's token-refresh job. They never enter the function sandbox, never appear in any
> function input, node property, or API response. Do not attempt to refresh tokens yourself;
> if `access_token` is rejected by the provider, throw `auth_expired` (§4) and the engine
> handles the refresh/reconnect lifecycle.

### 1.3 `mount` object

A read-only snapshot of the relevant `raisin:VirtualMount` config. Mutating it has no
effect — persist state by returning results, not by editing this object.

```javascript
mount = {
  mount_id: "01H...",              // owning mount node id
  remote_root: "0AL...folderId",   // provider-side root (folder id, mailbox, calendar, …)
  mount_path: "/documents/shared", // path prefix inside the target workspace
  sync_config: { /* FULL authored object — see below */ },
  api_config:  { /* the integration's api_config, verbatim — see below */ }
}
```

**The adapter receives the FULL `sync_config` and the integration's `api_config`.**
`sync_config` is forwarded **verbatim** — every key the mount node was authored with,
not a fixed whitelist. Besides the engine-interpreted keys (`mode`, `interval_seconds`,
`include_patterns` / `exclude_patterns`, `ephemeral`, `ttl_seconds`,
`max_items_per_sync`) it carries any **provider-specific** keys the adapter defines, e.g.
`host` / `port` / `tls` / `mailbox` / `auth` (IMAP), `resource` and `window` (Graph /
Calendar). `api_config` is the owning `raisin:Integration`'s `api_config` object,
forwarded verbatim as **connection defaults**; a mount typically merges the two with
`sync_config` winning per key. Both are built by `build_mount_snapshot`
(`.../virtual_mount_sync/adapter.rs`) from `MountConfig.sync_config_raw` and
`IntegrationConfig.api_config` (`.../virtual_mount_sync/config.rs`).

Two conventions the shipped connectors establish, available for any adapter:

- **`sync_config.window` — `{ days_ahead, days_back }`.** A time window (in days,
  relative to *now*) that bounds a listing/delta. The Google Calendar adapter defaults to
  `{ days_ahead: 90, days_back: 7 }`; the Microsoft Graph adapter to
  `{ days_ahead: 30, days_back: 7 }`. Used to build `timeMin`/`timeMax` (Calendar) or the
  `calendarView` start/end (Graph).
- **`sync_config.resource` — surface selector.** For multi-surface providers, names which
  surface to sync. The Microsoft Graph adapter reads `"mail"` (default) →
  `/me/mailFolders/{id}/messages`, or `"calendar"` → `/me/calendars/{id}/events`.

> **Preview — merge `api_config` under `sync_config`.** The IMAP / Gmail adapter reads
> `host` / `port` / `tls` / `auth` / `username` from `api_config` (integration template
> defaults) merged under the mount's `sync_config` (per-mount override wins). Note the
> mailbox is named `default_mailbox` in `api_config` and `mailbox` in `sync_config`. If
> your adapter accepts connection settings from both, document which wins.

### 1.4 `raisin.context`

The usual read-only `raisin.context` global is available (`tenant`, `repo`, `branch`,
`workspace`, `execution_id`). Because the adapter is invoked **directly, not via a trigger**,
`context.event` is absent — do not depend on it.

### 1.5 Host APIs available to adapters

An adapter runs as an ordinary server-side function, so the full `raisin.*` host API is
available. The ones an adapter actually reaches for a remote system:

| Host API | Use | Notes |
|----------|-----|-------|
| `raisin.http.fetch(url, opts)` | Talk to any HTTP/HTTPS provider (REST, JMAP, GraphQL). | The workhorse — most adapters need only this. Gated by the function's `network_policy` (`allowed_urls` globs). |
| `raisin.imap.{fetchSince,listMailboxes,fetchMessage}` | Talk to a real IMAP server over TLS (a native protocol binding, not HTTP). | See §1.6. Gated by the same `network_policy` — allow `imaps://host:port`. |
| `raisin.integrations.sync_now(mount_id, mode?)` | Trigger a sync of a mount from function code (e.g. a webhook-refresh trigger). | `mode` is `"delta"` (default) or `"full"`. Enqueues a `VirtualMountSync` job. |
| `raisin.crypto.verifyJwt(token, opts)` | Verify an RS256/ES256-signed JWT against a JWKS — the generic OIDC/JWT primitive for **signed** push (e.g. a Gmail Pub/Sub OIDC-authenticated push, an Apple/Firebase token). | `opts = { jwks_url, issuer?, audience?, algorithms? }`. Returns `{ valid, claims?, error? }` — never throws on a bad token (that is `{ valid: false, error }`). The `jwks_url` host must match the function's `network_policy`; a hard error is reserved for policy denial / unreachable JWKS. |

All egress — HTTP *and* IMAP — is bounded by the function node's `network_policy`; a host
that matches no `allowed_urls` pattern is refused before any request or socket opens. `raisin.imap`
and any future native protocol binding follow the same pattern as `raisin.http.fetch`; see
[Adding a Native Host Capability](../guides/adding-a-native-host-capability.md).

### 1.6 `raisin.imap.*` — native IMAP protocol

For adapters that must speak **real IMAP** (not JMAP-over-HTTP), Rust owns the protocol (TLS +
`LOGIN` / SASL `XOAUTH2` + `UID FETCH`) and the adapter calls high-level operations. `conn` is
`{ host, port, tls, auth, username, password }`:

- `tls` (default `true`) selects implicit TLS on 993; `false` uses a plaintext socket
  (only for trusted/loopback hosts) and authorizes against an `imap://…` policy URL.
- `auth` (default `"password"`) selects the mechanism: `"password"` sends `LOGIN`; `"xoauth2"`
  performs a native SASL `AUTHENTICATE XOAUTH2` handshake with `password` as the OAuth2 bearer
  access token.
- `password` is an app password or an OAuth2 access token and is **never logged**.

The connection's `host:port` must be authorized by the function's `network_policy`
(e.g. `allowed_urls: ["imaps://imap.example.org:993"]`) or the call is refused before any
socket opens.

| Method | Signature | Returns |
|--------|-----------|---------|
| `fetchSince` | `raisin.imap.fetchSince(conn, sinceUid, opts?)` | `{ messages: [{ uid, from, to, subject, date, snippet, flags, message_id }], highestUid, uidvalidity }` |
| `listMailboxes` | `raisin.imap.listMailboxes(conn)` | `[{ name, path, flags }]` |
| `fetchMessage` | `raisin.imap.fetchMessage(conn, uid, opts?)` | `{ headers, from, to, subject, date, text, html?, snippet, flags, message_id }` |

`opts` for `fetchSince` is `{ mailbox?: string ("INBOX"), limit?: number (200, capped) }`; for
`fetchMessage` it is `{ mailbox?: string }`. Only messages with `uid > sinceUid` are returned.
`highestUid` is the new cursor — it is **unchanged when nothing is new** (map it to the delta
`next_token`, never `null`, per §2.8). A changed `uidvalidity` means the mailbox reset and the
adapter must force a full resync. The same three calls exist in Starlark
(`raisin.imap.fetch_since` / `list_mailboxes` / `fetch_message`).

---

## 2. Operations

Dispatch on `input.operation`. An adapter should implement every operation it advertises as
supported in `capabilities`; unsupported operations may throw a non-reserved error code
(treated as transient) or, preferably, be gated by the corresponding capability flag so the
engine never calls them.

| operation | params | returns |
|-----------|--------|---------|
| `capabilities` | `{}` | `Capabilities` (§3.3) |
| `list` | `{ folder_id?, cursor?, limit? }` | `{ items: ExternalItem[], next_cursor: string \| null }` |
| `get` | `{ item_id?, path? }` | `ExternalItem \| null` |
| `get_content` | `{ item_id }` | `{ content, mime_type }` |
| `create` _(write)_ | `{ parent_id, name, is_folder, payload?, content?, mime_type? }` | `ExternalItem` |
| `update` _(write)_ | `{ item_id, payload?, name?, content?, mime_type?, fields?, etag? }` | `ExternalItem` |
| `delete` _(write)_ | `{ item_id, mode? }` | `{ deleted: true }` |
| `submit` _(write, optional)_ | `{ payload, external_id?, idempotency_key }` | `{ external_id, etag?, provider_id? }` |
| `get_changes` | `{ since_token: string \| null, folder_id? }` | `{ items: Change[], next_token: string }` |
| `subscribe` _(push, optional)_ | `{ notification_url }` | `{ subscription_id, secret?, expires_at?, resource? }` |
| `renew` _(push, optional)_ | `{ subscription_id, notification_url }` | `{ subscription_id, expires_at? }` |
| `unsubscribe` _(push, optional)_ | `{ subscription_id }` | `{ ok: true }` |
| `browse` _(discovery, optional)_ | `{ kind?, parent_id?, query?, cursor?, limit? }` | `{ items: BrowseItem[], next_cursor: string \| null }` |

`subscribe` / `renew` / `unsubscribe` are the **push lifecycle** (§2.9), called only when the
adapter advertises `supports_push: true` and the mount runs in `webhook` or `hybrid` mode. All
three are optional.

The four operations marked _(write)_ are the **write path** (§10). They are called only for a
mount that declares a write mode, and only for the specific ops that mode needs. Read-only
adapters omit them and report the matching capability flags as `false`.

### 2.1 `capabilities`

Static self-description. Must be cheap and side-effect-free (ideally no network call).
Returns a `Capabilities` object (§3.3).

**Who calls it, and what it drives.** The background sync loop does **not** call
`capabilities` on the hot path — it reads the sync-relevant flags (`supports_changes`,
`ephemeral` TTL) from the mount's already-loaded config. `capabilities` is invoked by the
**"Test connection"** endpoint (`POST /api/integrations/{repo}/test`,
`crates/raisin-transport-http/src/handlers/integrations/test_connection/`). That handler
invokes `capabilities`, then a bounded `list` probe, and on success **caches the returned
object onto the `raisin:Integration` node** in two properties:

- `capabilities` — the full report (§3.3),
- `capabilities_checked_at` — ISO 8601 timestamp of the probe.

The admin console reads the cached `capabilities` to **drive connector form visibility** — it
hides write/writeback controls when `can_write` is false, hides the ephemeral-TTL field unless
`default_ttl` is set, and so on. So `capabilities` is a UI/diagnostic contract: report it
**honestly**, because a lie here shows the wrong form, not a sync bug. Keep it network-free so
the probe stays fast.

### 2.2 `list`

Enumerate immediate children (one level) of `folder_id`. If `folder_id` is omitted, list the
children of `mount.remote_root`.

| param | type | meaning |
|-------|------|---------|
| `folder_id` | String? | Parent to list; defaults to `mount.remote_root`. |
| `cursor` | String? | Opaque pagination cursor from a previous `next_cursor`. |
| `limit` | Number? | Suggested page size. The engine also enforces `sync_config.max_items_per_sync`. |

Returns `{ items: ExternalItem[], next_cursor: string | null }`. Return `next_cursor: null`
when there are no more pages. The full-reconcile path (§5) calls `list` recursively.

### 2.3 `get`

Fetch a single item by `item_id` (preferred) or by `path` (relative to `remote_root`).
Returns the `ExternalItem`, or `null` if it does not exist / was deleted.

### 2.4 `get_content`

Download the byte content of one item. Returns `{ content, mime_type }`. `content` is the
file body (base64 string for binary, or a UTF-8 string for text — the adapter and mapping
decide, but be consistent per `mime_type`). Only called when content sync / write-through is
enabled; v1 mounts commonly link via `web_url`/`download_url` instead and never call this.

### 2.5 `create`

Create a file or folder under `parent_id`.

| param | type | meaning |
|-------|------|---------|
| `parent_id` | String | Provider parent id. |
| `name` | String | New item name. |
| `is_folder` | Boolean | Folder vs file. |
| `content` | String? | Body for files (ignored when `is_folder`). |
| `mime_type` | String? | Content type for files. |

Returns the created `ExternalItem` (with its new `external_id` and `etag`).

### 2.6 `update`

Update an existing item's name and/or content.

| param | type | meaning |
|-------|------|---------|
| `item_id` | String | Provider item id. |
| `payload` | Object? | Provider-shaped body from the mapper's `to_external` (§6.0). This is what a `state_only` write sends. |
| `fields` | String[]? | The allow-list `payload` was built for. Present on `state_only` writes; apply ONLY these. |
| `name` | String? | New name (rename). |
| `content` | String? | New body. |
| `mime_type` | String? | New content type. |
| `etag` | String? | Expected current etag for optimistic concurrency. |

Returns the updated `ExternalItem`. **If `etag` is supplied and does not match the provider's
current etag, throw an error with `code: "conflict"`** (§4) — do not blindly overwrite.

### 2.7 `delete`

Delete the item identified by `item_id`. Returns `{ deleted: true }`. Deleting an
already-absent item should succeed idempotently (return `{ deleted: true }`).

### 2.8 `get_changes`

Incremental delta since a provider change token. This is the fast path; implement it whenever
the provider has a real delta/changes API and advertise `supports_changes: true`.

| param | type | meaning |
|-------|------|---------|
| `since_token` | String \| null | Cursor from a previous `next_token`; `null` on the first delta call. |
| `folder_id` | String? | Scope of the change feed; defaults to `mount.remote_root`. |

Returns `{ items: Change[], next_token: string }`. The engine pages until `next_token`
stabilizes or `max_items_per_sync` is reached, then persists `next_token` as the mount's
`last_sync_token`. `next_token` must be a durable, resumable cursor: re-running from a
persisted token must be safe (the engine's upserts are idempotent). If the provider has **no**
delta API, set `supports_changes: false` and the engine falls back to a full-listing diff via
`list`; you need not implement `get_changes` in that case.

> **CRITICAL — never return `next_token: null` to mean "no changes."** When there is nothing
> new, return the **unchanged cursor** you were given (`since_token`), not `null`. A `null`
> next_token reads to the engine as "no more pages": it stops paging **without advancing** and
> **retains the previous cursor** — deliberately, so a spurious `null` cannot wipe the stored
> cursor and silently force a full resync on the next run. The engine's defensive retention
> lives in `delta.rs` (it only overwrites `last_sync_token` when the adapter actually returns a
> `Some(token)`), but you must still return the real cursor so incremental sync keeps working.
> Return `null` **only** if your provider genuinely has no resumable cursor — and then you
> should be on the full-listing path (`supports_changes: false`) instead.

### 2.9 `subscribe` / `renew` / `unsubscribe` — push lifecycle _(Experimental / Preview)_

Optional. Implement these to get **event-driven** sync ("on new email", "on calendar change")
instead of interval polling. They are called only when your `capabilities` reports
`supports_push: true` **and** the mount's `sync_config.mode` is `"webhook"` or `"hybrid"`.

**The key reframe: a push notification is only an _invalidation signal_.** The engine never
inspects the provider's notification payload. A provider ping just means *"re-run this mount's
normal delta sync"* — it maps the ping to a mount and enqueues an ordinary delta sync, and your
existing `get_changes` does the actual work. This is what keeps push fully generic: Microsoft
Graph subscriptions, Google Calendar channels, and Gmail Pub/Sub all collapse to the same three
operations, and every provider quirk (validation handshakes, header names, payload shapes) lives
in **your adapter and the notifications endpoint**, never in the engine.

The engine owns the subscription lifecycle and stores its result in the mount's engine-managed
`state` (never author these yourself): `push_subscription_id`, `push_secret`, `push_expires_at`,
`push_notification_url`, `push_status` (`"active" | "failed" | "unsupported"`), and
`push_last_error`.

**`subscribe`** — register a push subscription with the provider for the given
`notification_url` (a per-mount unguessable URL the engine generates and hands you). Return how
to identify and renew it.

| param | type | meaning |
|-------|------|---------|
| `notification_url` | String | Where the provider should POST notifications. Register this verbatim with the provider (Graph `notificationUrl`, Google channel `address`, Pub/Sub push endpoint, …). |

Returns:

| field | type | meaning |
|-------|------|---------|
| `subscription_id` | String | Provider subscription / channel id. **Required.** |
| `secret` | String? | Optional shared secret the provider will echo (Graph `clientState`, Google channel token). Stored so the notifications endpoint can verify pings; the URL token is the primary auth. |
| `expires_at` | String? | ISO-8601 expiry. Omit only if the subscription never expires; otherwise set it so the engine renews before it lapses. |
| `resource` | String? | Opaque provider resource id, stored for your own use on `renew`. The engine never interprets it. |

**`renew`** — the engine calls this for any active subscription expiring within one day. Extend
(or recreate) the subscription and return its new id + expiry.

| param | type | meaning |
|-------|------|---------|
| `subscription_id` | String | The id you returned from `subscribe`. |
| `notification_url` | String | The same per-mount URL (in case you must recreate the subscription). |

Returns `{ subscription_id, expires_at? }` (the id may rotate).

**`unsubscribe`** — best-effort teardown when the mount is disabled. Delete the provider
subscription. Return `{ ok: true }`. Errors are logged and ignored (the provider will expire it
anyway).

**How pings reach your sync.** The engine hands the provider a URL of the shape
`{base}/api/integrations/{repo}/notifications/{mount_token}`, where `mount_token` is a stable,
unguessable per-mount token (`{mount_id}.{nanoid}`) and `{base}` comes from the `RAISINDB_BASE_URL`
server config (push cannot be wired without it). The public notifications endpoint validates the
token, runs any provider validation handshake, and enqueues a normal delta sync for that mount. You
do **not** implement that endpoint per provider — it is **one generic endpoint**, keyed on request
*shape*, never on provider identity.

#### Notifications endpoint contract

`GET|POST /api/integrations/{repo}/notifications/{mount_token}` — public (providers cannot send a
bearer), guarded solely by the unguessable `mount_token` plus the per-subscription `secret`. It
handles, in order:

1. **Validation echo.** If the request carries a challenge parameter — `validationToken` /
   `validation_token` (Microsoft Graph), `challenge` / `hub.challenge` (pub/sub-style hubs), in the
   query string **or** JSON body — it is echoed back verbatim as `text/plain; charset=utf-8` with
   `200`, regardless of method. Graph sends this at subscription-creation time, before any secret is
   in play, and requires a prompt verbatim echo. A bare `GET` with no challenge (Google channel
   `sync` message / a health probe) also returns `200`.
2. **Token → mount.** The `{mount_token}` path segment is matched (constant-time) against each
   mount's stored `state.push_notification_url` last segment (the URL the provider was literally told
   to call), falling back to a `push_token`/`push_mount_token` field, then the mount id. No match →
   `404` (the token is never echoed back).
3. **Secret verification.** A provider-supplied secret — a `clientState` **body** field (Graph), an
   `X-Goog-Channel-Token` **header** or a `token` body field (Google), or a `token` **query** param
   (generic / pub-sub) — is compared in constant time against the mount's stored `state.push_secret`
   (the `secret` you returned from `subscribe`). If a secret is stored and none matches → `401`. If no
   secret is stored, the unguessable token is the only guard (allowed).
4. **Invalidation.** On success it enqueues one `VirtualMountSync { mode: "delta" }`
   (deduped `vmount-sync:{mount_id}`) and acks `200` **immediately** — it never blocks on the sync.
   The provider payload is discarded; your `get_changes` does the real work.

So an adapter's only job for push is the three operations plus returning a `secret` in one of the
carriers above; it never parses a notification body and never runs any per-provider endpoint code. For
providers that authenticate the push itself with a **signed JWT** (e.g. Gmail Pub/Sub OIDC push), use
`raisin.crypto.verifyJwt` (§1.5) — but note the shipped Gmail path uses the simpler shared-secret
`token` carrier, and the endpoint's JWT check is opt-in glue, not a hardcoded provider branch.

**Bootstrapping & lifecycle, at a glance:**

- A `webhook`-mode mount is not polled. To create the initial subscription, the periodic check
  enqueues **one** sync run for a webhook mount that has no live subscription; that run's engine
  `subscribe` step registers push, then the mount falls silent and is driven by pings.
- A `hybrid`-mode mount keeps polling on its interval **and** maintains a push subscription — a
  safety net when a provider ping is missed.
- If `supports_push` is `false` on a `webhook` mount, the engine marks it `push_status:
  "unsupported"` and stops trying (it will neither poll nor push — reconfigure the mount).
- Renewal runs on a periodic engine job; you only implement the three operations.

### 2.10 `browse` — remote discovery for the mount editor _(optional)_

Optional, and **never called during sync**. `browse` exists so a human configuring a mount can
*pick* a remote container instead of pasting a provider id into a text box. The admin console
calls it through `POST /api/integrations/{repo}/browse` (admin-gated, synchronous, bounded);
the sync engine never calls it at all.

Advertise it with `supports_browse: true` (§3.3). Adapters that do not implement it keep
working unchanged — the console simply keeps its free-text input.

| param | type | meaning |
|-------|------|---------|
| `kind` | String? | What class of thing to list. Adapter-defined; see below. Absent means the adapter's most useful default. |
| `parent_id` | String? | List the children of this container. Absent means the top level for `kind`. |
| `query` | String? | Free-text filter, when the provider supports search for that kind. |
| `cursor` | String? | Opaque page cursor from a previous `next_cursor`. |
| `limit` | Number? | Page size hint. The endpoint caps this regardless of what you honour. |

Returns `{ items: BrowseItem[], next_cursor: string | null }` (§3.4).

**`kind` is deliberately adapter-defined, not an enum.** Providers do not agree on what is
selectable — Graph has mail folders, calendars, sites and drives; IMAP has only folders. A
closed enum would have to be widened in the contract, the engine, the HTTP layer and every
console control each time a connector added a container type, which is exactly the
mirrored-code drift this codebase keeps getting bitten by. Validate the SHAPE, pass the slug
through verbatim, and let the console label it from `kind` + `name`.

The ms-graph adapter implements: `folder` (mail folders, hierarchical), `calendar`, `site`,
`drive` (document libraries), `driveItem` (folders within a drive, hierarchical) and `mailbox`
(directory users).

**`browse` results are a convenience, not an authorization statement.** A provider may happily
list a container the credential cannot actually read — Graph's directory listing is the
standing example, since it enumerates users regardless of which mailboxes the account has been
granted. Never treat a browsable item as a usable one; the console keeps manual entry available
and `Test connection` remains the thing that proves access.

---

## 3. Data types

All timestamps are ISO 8601 strings. All ids are provider-native strings.

### 3.1 `ExternalItem`

The normalized representation of one external object.

```javascript
{
  external_id:  string,          // stable provider id — the upsert key. REQUIRED.
  name:         string,          // display name. REQUIRED.
  mime_type:    string | null,
  size_bytes:   number | null,
  is_folder:    boolean,         // REQUIRED — drives the default mapping (§6).
  parent_id:    string | null,   // provider parent id (null at root)
  created_at:   string,          // ISO 8601
  modified_at:  string,          // ISO 8601
  etag:         string | null,   // change-detection token; enables etag skip-write
  web_url:      string | null,   // human-openable link
  download_url: string | null,   // direct content link
  metadata:     object           // provider-specific passthrough; preserved on the node
}
```

`external_id` must be **stable across renames and moves** — the engine matches on it to
update the existing node instead of creating a duplicate. `etag` should change whenever the
item's content or metadata changes; a stable `etag` lets the engine skip re-writing an
unchanged node (avoiding revision churn and spurious trigger storms). `metadata` is carried
onto the materialized node's properties untouched, so put provider extras there.

### 3.2 `Change`

One entry in a `get_changes` feed.

```javascript
{
  type: "created" | "updated" | "deleted",
  item: ExternalItem,            // for "deleted", at least external_id must be populated
  relative_path: string          // path of the item relative to remote_root / mount root
}
```

For `"deleted"`, the engine only needs `item.external_id` to locate and remove the node; the
rest of `item` may be a best-effort tombstone.

### 3.3 `Capabilities`

Returned by the `capabilities` operation. Cached on the `raisin:Integration` node by the
"Test connection" endpoint (§2.1) and read by the admin UI to decide which connector controls
to show. Report every field honestly.

```javascript
{
  can_read:            boolean,
  can_write:           boolean,
  can_create_folders:  boolean,
  supports_changes:    boolean,   // true = real delta API (get_changes); false = full-listing diff
  supports_webhooks:   boolean,
  supports_search:     boolean,
  supports_push:       boolean,   // event-driven / push providers — gates the §2.9 push lifecycle
  supports_browse:     boolean,   // implements §2.10 browse — gates the mount editor's pickers
  default_ttl:         number | null,  // suggested TTL (seconds) for ephemeral nodes
  max_file_size:       number | null,  // bytes; engine skips larger items

  // --- write path (§10). All optional; every one defaults to false / empty,
  //     so an adapter that omits them is correctly treated as read-only.
  can_create:              boolean,
  can_update:              boolean,
  can_delete:              boolean,
  can_submit:              boolean,
  mutable_fields:          string[],       // the state_only allow-list — which node
                                           // properties this provider accepts as writes
  default_delete_policy:   "detach" | "trash" | "purge" | null,
  default_move_policy:     "push" | "detach" | "reject" | null,
  supports_trash:          boolean,        // delete can soft-delete rather than purge
  supports_idempotency_key: boolean        // submit can forward a provider idempotency key
}
```

`supports_changes` is the most load-bearing flag: `false` forces the engine onto the
full-listing reconcile path for every sync.

**`mutable_fields` is how a provider explains what "writable" means for it.** The engine has
no domain knowledge — it does not know that a mail body is immutable while its read flag is
not. An adapter that lists `["unread", "categories", "folder"]` is telling the engine exactly
which property edits it will accept; an edit to anything else is rejected with a clear error
instead of being silently dropped. Declare the narrowest honest set.

`default_delete_policy` / `default_move_policy` are the adapter's **recommended** defaults for
its domain — mail typically wants `trash` (users expect "delete" to mean the provider's
trash), files and calendars typically want `detach` (never destroy remote data from a local
delete). A mount may override either; `purge` is never a default.

`supports_browse` affects only the admin UI: false (or absent) keeps the mount editor's
free-text id inputs, true adds pickers backed by §2.10.

### 3.4 `BrowseItem`

Returned by `browse` (§2.10). Purely a UI affordance — none of it is persisted.

```javascript
{
  id:           string,    // provider id; what the operator's choice writes into the mount
  name:         string,    // human label
  kind:         string,    // echoes the requested kind (or the actual kind, for mixed listings)
  has_children: boolean,   // show an expander — the picker will browse with parent_id = id
  hint:         string | null   // optional secondary line (address, path, item count)
}
```

`id` must be the value the mount actually needs (a Graph folder id, a composite site id), not
a display path — the picker writes it through verbatim.

---

## 4. Error convention

Signal failures by **throwing an `Error` with a `code` property**. The engine dispatches on
`code`:

| `code` | Meaning | What the engine does |
|--------|---------|----------------------|
| `"auth_expired"` | The `access_token` is rejected / the account needs re-auth. | Marks the account as needing re-authentication, sets the mount's `state.status = "auth_required"`, and **pauses the mount** (skips it in future sync checks) until the user reconnects. Not retried. |
| `"rate_limited"` | The provider is throttling. | Backs off using the standard job retry mechanism (exponential). The mount is retried later; no state corruption. |
| `"conflict"` | Write-through only: an `update`/`create` lost an optimistic-concurrency check (etag mismatch). | Surfaced to the caller of the write. Not a sync-loop failure. With `conflict: "remote_wins"`, the local edit is dropped and a warning event is emitted. |
| anything else | Transient failure. | Standard job retry: increments `state.consecutive_failures`, records `state.last_error`, and re-enqueues with backoff. After the configured threshold (default 5) the mount goes `state.status = "degraded"` and the interval backs off. Success resets the counters. |

Example:

```javascript
if (resp.status === 401) {
  const e = new Error("access token rejected");
  e.code = "auth_expired";
  throw e;
}
```

Never swallow an auth failure and return empty results — an empty `list`/`get_changes` reads
as "everything was deleted" and the reconcile will remove the mount's nodes. Throw the
correct `code` instead.

---

## 5. Sync model (what the engine does with your results)

You do not manage nodes, cursors, or transactions — the engine does. For context:

- **Full reconcile** runs on first sync, when `supports_changes: false`, or on an explicit
  `mode: "full"` manual sync. The engine recursively `list`s, upserts every item, then
  deletes any mount-owned node it did not see this pass.
- **Delta sync** runs otherwise: the engine calls `get_changes(since_token)`, pages, maps and
  materializes each `Change`, then persists `next_token`.
- **Upsert matching** is by `external_id` within the mount subtree (falling back to path), so
  a provider-side rename updates the existing node instead of duplicating it.
- **Etag skip-write:** if the existing node's `__etag` equals the incoming item's `etag`, the
  engine writes nothing. Keep your `etag` stable-when-unchanged to benefit.
- **Deletes are scoped:** the engine only deletes nodes it owns (`__mount_id` matches). It
  never deletes user-created nodes that happen to sit under the mount path.
- **`include_patterns` / `exclude_patterns`** (globs from `sync_config`) are applied by the
  engine before mapping.
- **Ephemeral mounts** (`sync_config.ephemeral: true`) auto-delete nodes older than
  `ttl_seconds` (see `default_ttl` in your `Capabilities`).

The sync engine runs writes under the actor `"virtual-mount-sync"` with a system auth
context. Downstream `node_event` triggers therefore also run privileged — adapter code is
trusted, admin-installed code.

---

## 6. Mapping function contract (optional)

By default, the engine maps each `ExternalItem` to a node with a **built-in Rust mapping** —
no function call, zero overhead on the minimal sync path:

- `is_folder === true` → node type **`raisin:Folder`**.
- everything else → node type **`raisin:Node`**, with `title` and a `meta` object carrying
  `mime_type`, `size`, `web_url` / `download_url`, and any provider `metadata` passthrough.

> The default file type is **`raisin:Node`**, not `raisin:Asset`: `raisin:Asset` requires a
> binary `file` Resource, which a link-only virtual node does not have. To materialize
> `raisin:Asset` (or any custom type), supply a `mapping_function`. The shipped
> **`google-drive-adapter`** does exactly this — its default mapper emits `raisin:Folder` for
> folders and `raisin:Asset` (with `web_url` / `download_url` links, no inlined content) for
> files.

### 6.0 The mapper is bidirectional — and both directions live in ONE function

A mapper dispatches on `input.operation`, exactly like an adapter:

```javascript
function handler(input) {
  switch (input.operation) {
    case "to_node":     // { external_item, mount }
      // -> { node_type, name?, properties } | null   (null = skip this item)
      return toNode(input.external_item, input.mount);

    case "to_external": // { node, mount, fields? }
      // -> { payload, external_id? } | null          (null = not writable)
      return toExternal(input.node, input.mount, input.fields);

    case "mapper_capabilities": // { mount }
      // -> { to_external: true }
      return { to_external: true };
  }
}
```

**Why both directions must be in the same function node, and not inside the adapter.** The
mapper is deliberately *separate* from the adapter so you can customize node shape without
forking the adapter. If the reverse translation were hardcoded in the adapter's
`update`/`create`, then pointing a mount at a custom mapper would leave the adapter writing
the wrong fields — silently, with no error anywhere. That is one relationship expressed twice,
in two files, free to drift. Keeping `to_node` and `to_external` side by side in one file
gives them one author and one place to stay consistent.

Rules:

- **Backward compatible.** Input with no `operation`, or `operation: "to_node"`, behaves
  exactly as before. Existing mappers keep working untouched.
- **`fields` is the allow-list** for `state_only` mounts (§10). When present, emit *only*
  those keys — the engine is asking for a patch, not a whole object.
- **Return `null` from `to_external`** to say "this node is not writable". The write parks
  with a stated reason rather than pushing a guess.
- **A mapper without `to_external` makes its mount read-only.** This is probed once per run
  and recorded in `state.writeback_supported` / `state.writeback_last_error`, so the console
  can explain *why* a write control is unavailable. Writability is a property of the **mount**
  — adapter and mapper together — so a write-capable adapter paired with a read-only custom
  mapper is honestly reported as not writable.
- **The probe is its own operation: `mapper_capabilities`.** It takes `{ mount }` and returns
  `{ to_external: true }`. A mapper that implements `to_external` MUST answer it — anything
  else (a `null`, a missing key, a throw) is read as "no reverse mapping". A mapper written
  before the write path existed answers `null` for free, because it reads only
  `input.external_item` and falls straight through its `if (!item …) return null` guard, so no
  shipped mapper needed changing and none can be accidentally write-enabled.
  The probe is deliberately *not* a `to_external` call with a null node: that would oblige
  every `to_external` ever written to tolerate a null node forever, and a strict one would be
  misreported as read-only for throwing on a call it was never meant to receive.
  It is asked only of a mount that actually requested writeback — probing a read-only mount
  would spend a QuickJS invocation per run to compute a value that is discarded.
- **Keep `to_external` pure and I/O-free**, like `to_node`. It runs inside the write drain,
  under the mount lease.
- **The built-in Rust default mapping has no reverse.** A mount with no `mapping_function` is
  read-only by construction — the default mapping is lossy, so inverting it would be guessing.

### 6.1 `raisin:Event` — the standard calendar mapping target

> **Experimental / Preview.** `raisin:Event` and the calendar connectors that emit it
> (Google Calendar, Microsoft 365) are a preview feature — validate against your own
> account before relying on it in production.

Calendar connectors map each external event to a **`raisin:Event`** node
(`crates/raisin-core/global_nodetypes/raisin_event.yaml`) rather than the generic
`raisin:Node`. It is a `strict` type, so it declares both the typed calendar fields and
the reserved `__`-virtual properties (a strict type must declare the reserved props or
synced writes are rejected). Fields:

| Property | Type | Meaning |
|----------|------|---------|
| `title` | String (required) | Event title / summary. |
| `start` | String | Start time, ISO 8601. Indexed. |
| `end` | String | End time, ISO 8601. Indexed. |
| `all_day` | Boolean | Whole-day event (default `false`). |
| `location` | String | Event location. |
| `attendees` | Array | Attendee list. |
| `organizer` | String | Organizer (email or display name). |
| `recurrence` | String | Recurrence rule (RRULE); an array of rules is joined with newlines. |
| `status` | String | `confirmed` / `tentative` / `cancelled` / … |
| `url` | String | Link to the event in the source calendar. |
| `calendar_id` | String | Identifier of the source calendar. |

Plus the reserved `__virtual` / `__mount_id` / `__external_id` / `__etag` / `__synced_at`
properties (§7), which the engine stamps and a mapping function must not set.

The pattern the shipped connectors follow: the **adapter** carries raw provider fields
through `ExternalItem.metadata` (leaving `is_folder: false`, so the default mapping would
otherwise pick `raisin:Node`), and a **mapping function** hoists that metadata into the
typed `raisin:Event` properties. See
`builtin-packages/google-calendar-adapter/content/functions/mappers/google-calendar-default/index.js`
and `builtin-packages/ms-graph-adapter/content/functions/mappers/ms-graph-calendar/index.js`.
A cancelled event maps to `null` (skipped) rather than a `cancelled`-status node.

To customize, set a mount's `mapping_function` to a `raisin:Function` path. It is called
**once per external item**:

```javascript
// input
{
  external_item: ExternalItem,
  mount: { mount_id, mount_path, sync_config }
}

// return — the node to materialize
{
  node_type: "raisin:Asset",       // any allowed node type
  name: "optional override",       // omit to let the engine derive it
  properties: { title: "...", /* ... */ }
}

// return null → SKIP this item (a filtering hook)
```

The mapping function is a normalization/filtering hook only. It must be pure and fast: it
runs once per item, in the sync hot loop. It must **not** call `raisin.functions.call` (see
§8). Returning `null` drops the item from materialization entirely.

The engine always writes the reserved virtual properties (§7) on top of whatever the mapping
returns — you cannot suppress them, and you should not set them yourself.

---

## 7. Reserved virtual metadata properties

The materializer stamps these on **every** synced node. They are plain node properties, so
ordinary SQL works against them.

| Property | Type | Meaning |
|----------|------|---------|
| `__virtual` | Boolean | Marks a mount-managed node. |
| `__mount_id` | String | Owning mount node id. |
| `__external_id` | String | Provider item id — the stable upsert-match key. |
| `__etag` | String | Provider change token at last sync (drives skip-write). |
| `__synced_at` | String | ISO 8601 timestamp of the last sync write. |

Because they are plain properties, query them with the standard JSON-property conventions,
e.g.:

```sql
SELECT * FROM 'workspace' WHERE properties->>'__mount_id'::String = $1
SELECT * FROM 'workspace' WHERE properties->>'__external_id'::String = $2
```

**Indexing:** the reserved `__`-properties are written at runtime by the materializer, so
they are not part of any node type's declared schema — the built-in `raisin:Folder` /
`raisin:Node` mappings do **not** ship an `index: [Property]` declaration for them. Two
consequences:

- The engine's own **upsert-match currently does a full workspace scan** (not a property-index
  lookup) to find the node with a given `__external_id` / `__mount_id`. This is a known
  scaling ceiling for very large target workspaces.
- For **your** SQL against virtual nodes (`properties->>'__mount_id'::String = …`), declaring
  `index: [Property]` on `__mount_id` / `__external_id` in a custom mapped node type will make
  those queries index-backed rather than full scans. It does not change the engine's internal
  matching.

Do not set the `__`-prefixed properties from a mapping function; the engine owns them and
overwrites them.

---

## 8. Performance guidance (mandatory reading for adapter authors)

**Do not call `raisin.functions.call` in hot loops.** A nested function call blocks a job
worker for up to **5 minutes per nesting level**, and there is **no recursion-depth guard**.
An adapter or mapping function that calls other functions per item can exhaust the worker
pool and stall all sync jobs. Keep adapter and mapping logic self-contained: do the provider
I/O and normalization inline, and let the engine handle materialization. If you genuinely need
cross-function orchestration, do it outside the per-item path (e.g. once per sync run), never
per `ExternalItem`.

Additional guidance:

- Respect `limit` and page efficiently; the engine caps work with
  `sync_config.max_items_per_sync`, but tight pages reduce latency and memory.
- Keep `capabilities` free of network calls where possible — it is polled.
- Make `next_token` (delta) and `next_cursor` (list) durable and resumable; the engine may
  re-run a page after a crash and relies on idempotency.
- Keep `etag` stable when nothing changed, so the engine's skip-write suppresses needless
  revisions and trigger storms.
- Restrict outbound requests to your provider's hosts; adapter function nodes ship with a
  `network_policy` scoped to provider hosts.

---

## 9. Connector setup metadata & the setup-urls endpoint

Wiring an external system almost always needs steps performed **on the provider side** —
create an OAuth client, register a redirect URI, grant scopes, and (for some push providers)
paste a per-mount notification endpoint into the provider's subscription. A connector
**declares its own external-side guide**, and the server hands the admin UI the exact URLs a
user must paste back. Nothing here is provider-specific in the engine or the endpoint —
provider specifics live entirely in the connector template and adapter.

### 9.1 Declared setup metadata on `raisin:Integration`

Two optional properties on the `raisin:Integration` node
(`crates/raisin-core/global_nodetypes/raisin_integration.yaml`, `strict: true`) let a
connector ship its own instructions:

| Property | Type | Meaning |
|----------|------|---------|
| `setup_instructions` | String (markdown) | The external-side steps: create the OAuth client, register the Redirect URI shown in the UI, grant scopes/permissions, paste the mount Notification URL, etc. Surfaced **read-only** in the admin console. |
| `docs_url` | String (optional) | A link to the provider's own setup documentation. |

A connector is a `.rap` package under `builtin-packages/` whose integration **template**
lives at `content/_raisin__system/integrations/<name>/.node.yaml`. Because `setup_instructions`
and `docs_url` are just template properties, **user-authored connectors ship their own
external-setup guide** by filling these in — the admin console renders whatever the template
declares. Write `setup_instructions` in Markdown; keep it to the provider-side steps (the
RaisinDB-side URLs are injected by the endpoint below, so you don't hardcode a base URL).

### 9.2 The `setup-urls` endpoint

The server computes the two URLs a user pastes on the provider side and returns them to the
admin UI (admin-guarded, Experimental). Both URL shapes are frozen — they are literally what
`oauth_callback` is routed on and what the public notifications endpoint parses.

| Route | Returns |
|-------|---------|
| `GET /api/integrations/{repo}/setup-urls` | `oauth_redirect_uri` only (`notification_url: null`) — shown on the **connector page** before any mount exists. |
| `GET /api/integrations/{repo}/mounts/{mount_id}/setup-urls` | `oauth_redirect_uri` **plus** the per-mount `notification_url` — shown on the **mount**. |

Response body (`SetupUrlsResponse`,
`crates/raisin-transport-http/src/handlers/integrations/setup_info.rs`):

| Field | Type | Meaning |
|-------|------|---------|
| `oauth_redirect_uri` | String | `{base}/api/integrations/{repo}/oauth/callback` — register this as the OAuth client's "redirect URI" / "reply URL". Independent of any mount. |
| `notification_url` | String \| null | `{base}/api/integrations/{repo}/notifications/{mount_token}` — the per-mount push endpoint. `null` on the integration-level route. Register verbatim with the provider (Graph auto-registers it; **Gmail Pub/Sub requires the operator to paste it** into the push subscription). |
| `mount_token` | String \| null | The stable `push_mount_token` embedded in `notification_url` (`"{mount_id}.{nanoid(32)}"`), or `null` when no mount is in scope. |
| `base_url_configured` | Boolean | `false` when `RAISINDB_BASE_URL` is unset: the URLs carry a literal `{base}` placeholder the user must replace with the server's public origin. The endpoint returns the paths rather than failing, so the UI can flag the missing host. |

### 9.3 The notification token is minted lazily and shared with the engine

The per-mount `notification_url` embeds a `push_mount_token` stored on the
`raisin:VirtualMount` node's `state.push_mount_token`. The mount-level `setup-urls` route
**lazily mints and persists** this token on first call (and backfills
`state.push_notification_url` once a real base URL is configured), in the engine's exact
`"{mount_id}.{nanoid(32)}"` format. This is the **same** token the sync engine's
`ensure_mount_token` produces
(`crates/raisin-rocksdb/.../virtual_mount_sync/subscription.rs`): whichever side runs first
mints it, the other reads the existing value, and it stays **stable across renews**. The
public notifications endpoint resolves an incoming request by matching its last path segment
against the stored `push_notification_url`, then `push_mount_token`, then the mount id — so
the URL the UI shows and the URL the provider calls always agree.

---

## 10. The write path

> **Status: `state_only` ships; the rest is designed, not yet implemented.** The engine's
> write drain calls `update`, and only `update` — for a mount with
> `write_config.mode: "state_only"`, restricted to the fields both the mount and your
> `capabilities.mutable_fields` name. `create`, `delete`, `submit` and `get_content` are
> still never invoked, and a mount requesting `writeback: "write_through"` (a full mirror)
> still records `state.writeback_supported: false` with the missing operations named.
> Adapters may implement the rest now; they will not be exercised until those stages land.
> Design and staging: `docs/virtual-nodes-implementation-plan.md`.

### 10.0 What a `state_only` write looks like end to end

1. The mount is configured `mode: "state_only"`, `mutable_fields: ["unread"]`; your adapter
   declares `can_write`, `can_update` and `mutable_fields: ["unread"]`; your mapper answers
   the `mapper_capabilities` probe with `{ to_external: true }`. Any of those missing and the
   mount is reported unwritable **with the reason**, rather than silently doing nothing.
2. A user edits `unread` on a synced node.
3. On the next sync run — under the mount lease, **before** the read phases — the engine
   finds the node whose `unread` no longer matches the engine-owned `__pushed_state`, re-reads
   it, and asks your mapper for `to_external` with `fields: ["unread"]`.
4. It calls your `update` with `{ item_id, payload, fields, etag }`, where `etag` is the
   node's stored `__etag` (your optimistic-concurrency base).
5. It stamps your returned `etag` and the pushed values back onto the node as the sync actor.
   That stamp is what makes the delta echoing your own write a no-op.

You are never called twice for one edit, and never called for an edit that already landed.

### 10.1 The boundary: what the engine owns, what you own

The engine's write path is deliberately **thin and domain-blind**. It knows "call `update`
with these fields". It does not know what a calendar is, that a mail body is immutable, or
what an outbox means. All of that is your package explaining itself, through `capabilities`,
your nodetypes, and your docs.

| Layer | Owns |
|-------|------|
| **Engine** (Rust, generic) | change detection, ordering, the mount lease, intent lifecycle, the "already pushed?" check, metadata stamp-back, safety rails, at-most-once semantics, error classification |
| **Adapter package** (your JS) | the remote API calls, node↔provider translation, the declared capabilities, the optional conflict resolver |
| **Convention** (your docs + nodetypes) | which node types, which collections, outbox layout, mount templates |

**Adapters never write nodes.** Your adapter is a function the engine calls: take a request,
hit the provider, return a result. The engine performs every local write. This is not
stylistic — delegating writes would lose lease serialization (a concurrent sync clobbers your
write), the metadata stamp-back that prevents infinite sync loops, the destructive-operation
rails, and the sandbox boundary (adapters run privileged with a system auth context, so an
adapter that could write nodes could write *any* node in the workspace).

> **Adapter decides what the remote becomes and performs the remote call.
> Engine decides what the node becomes and performs the local write.**

### 10.2 Write modes

A write mode is a property of the **mount**, not of the adapter. The same IMAP adapter serves
a `state_only` inbox mount and a `submit` outbox mount. Your adapter declares which operations
it can perform; the mount decides which of them apply where.

| mode | the node is… | a local change means | typical use |
|------|--------------|----------------------|-------------|
| `mirror` | the remote object itself | create / update / delete propagate | calendar events, files |
| `state_only` | an immutable record with mutable state | only `mutable_fields` propagate; other edits are rejected | mail (body immutable, read/flags/folder are not) |
| `submit` | a **command** | creating it and moving it to `queued` issues the command once | send / reply / forward, RSVP |

`submit` is what makes immutable resources writable in a coherent way. An email cannot be
"edited" — so its write path is a *sending* path, and the natural home for that is a separate
mount whose members are intents rather than mirrors:

```
/mail/inbox    mode: state_only   raisin:Mail
/mail/sent     read-only          raisin:Mail          <- canonical sent message
/mail/outbox   mode: submit       raisin:OutboundMail  <- commands
```

Reply and forward then need no special casing: the outbox node carries the action and the
provider's own message id. The same shape generalizes to any connector — a chat outbox, a
refund queue, an order submission mount are all `submit` collections.

### 10.3 `submit` is at-most-once — never retried

`submit` issues a side effect the provider cannot take back. A retried send is a duplicate
email; a retried charge is a duplicate charge. So the engine treats `submit` differently from
every other operation:

- The command node is durably moved to `sending` **before** the call, so a crash mid-flight is
  a *bounded* ambiguity rather than an unbounded one.
- On success → `sent`, with `external_id` / `etag` stamped back.
- `rate_limited` → requeued. **This is the only error that requeues**, because it is the only
  one that proves no side effect occurred.
- `auth_expired`, `config_error`, `conflict` → `failed`. Definitive pre-effect rejections.
- **Anything else — including a timeout — parks at `unknown` and is never retried
  automatically.** Only a human moves it back to `queued`.

This inverts the usual default: for reads, an unrecognized error is transient and retried; for
`submit`, an unrecognized error is *ambiguous* and must not be. Throw precise codes (§4).

If your provider accepts an idempotency key, declare `supports_idempotency_key: true` and
forward the engine's `idempotency_key` — that is what lets an ambiguous case be safely
resolved rather than parked.

### 10.4 Delete and move

Neither is a fixed behaviour; both are policy the mount resolves from your declared defaults.

| `delete_policy` | effect |
|-----------------|--------|
| `detach` | the node is removed locally, the remote is untouched. **A later full reconcile re-imports it** — there is no suppression list. Say so in your docs. |
| `trash` | the remote is soft-deleted (provider trash / deleted items). Call `delete` with `mode: "trash"`. |
| `purge` | the remote is hard-deleted. Never a default. |

`move_policy` is `push` | `detach` | `reject`. **There is no `move` operation** — a move is
modelled as an `update` carrying the new parent/folder field, which keeps the operation
surface small and means a provider that reparents through its normal update call needs no
extra code.

### 10.5 Optimistic concurrency and conflict

Writes carry the node's last-known provider etag. If it no longer matches, throw
`code: "conflict"` (§4) rather than overwriting — that is the signal the engine's conflict
policy acts on (`remote_wins` by default, or `local_wins`, or park for a human).

A package may also ship a **conflict resolver function**, referenced by the mount as
`resolver_function`. It is a plain `raisin:Function`, invoked exactly like a mapper, and it
receives both sides:

```javascript
// input:  { local, remote, base_etag, field_diff, mount }
// return: { resolution: "local_wins" | "remote_wins" | "merged" | "park", node?, fields? }
```

This is where domain knowledge belongs — only a calendar package knows that two edits touching
different fields of the same event can be merged, while two edits to the same start time
cannot. A throw parks the write; it is never silently dropped.

### 10.6 What the engine guarantees you

So you can keep your adapter simple:

- **You are never called concurrently for the same mount.** Writes drain under the same lease
  as sync, ahead of the read phase.
- **You are not called for a write that already landed.** The engine compares stored provider
  metadata before every call and skips no-ops.
- **You are not called with a change you made.** Metadata stamped after your write suppresses
  the echo when the next delta returns the item you just changed.
- **You are not called for a runaway delete.** Proportional blast-radius rails stop a
  mis-scoped bulk statement before it reaches the provider, park the pending writes, and
  surface the block for an operator — without stopping reads.
