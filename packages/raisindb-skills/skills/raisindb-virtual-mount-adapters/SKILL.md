---
name: raisindb-virtual-mount-adapters
description: "Build custom connector adapters for RaisinDB virtual mounts: sync external systems (mail, calendars, drives, any API) into nodes and push local edits back. Covers the adapter operation contract (capabilities/list/get_changes/update/submit/subscribe), error taxonomy, cursors and has_more, bidirectional mappers, write modes (state_only/mirror/submit), echo prevention, and the field-tested traps. Use whenever the user wants a connector, adapter, integration sync, virtual mount, two-way sync with an external service, or mentions raisin:VirtualMount, raisin:Integration, get_changes, or 'sync X into RaisinDB'."
---

# RaisinDB Virtual-Mount Adapters

A **virtual mount** materializes an external system (mailbox, calendar, drive,
arbitrary API) as ordinary nodes under a mount path, kept in sync by the
engine — and optionally pushes local edits back. An **adapter** is a
`raisin:Function` (QuickJS or Starlark) that translates ONE normalized
operation per call into provider API calls. The engine owns scheduling,
leases, cursors, retries, batching, echo prevention, and conflict policy;
the adapter owns exactly one thing: **talking to the provider**.

**The canonical contract is `docs/reference/virtual-node-adapters.md`
(frozen). Read it before writing an adapter.** The reference implementation
is `builtin-packages/ms-graph-adapter/` (mail + calendar + drive); the
smallest complete one is `builtin-packages/google-drive-adapter/`.

## Start from the scaffold

```bash
npx raisindb create adapter <name>     # working package skeleton:
                                       # capabilities + list implemented,
                                       # connector config nodetypes, manifest
```

## Handler shape

One entrypoint, one argument, dispatch on `operation`:

```javascript
export default function handler(input) {
  var { operation, params, credential, mount } = input;
  switch (operation) {
    case "capabilities": return opCapabilities(mount);
    case "list":         return opList(credential, mount, params);
    case "get_changes":  return opGetChanges(credential, mount, params);
    // get, get_content, create, update, delete, submit,
    // subscribe, renew, unsubscribe, browse — all optional, see contract
    default: throw new Error("unsupported operation: " + operation);
  }
}
```

`credential` arrives **decrypted, in memory, per call** — access token only,
never a refresh token. `mount` is a read-only config snapshot.

## The five rules that come from production incidents

Every one of these was a real outage. Do not relearn them.

1. **Declare `has_more` from `get_changes`.** `{ items, next_token,
   has_more }` — `true` = "mid-enumeration cursor, call me again now",
   `false` = "caught up; the token is next run's resume point". Never rely
   on token identity: Microsoft Graph mints a FRESH delta token on every
   poll of an idle feed, and before `has_more` the engine's delta loop spun
   empty pages at request speed until the watchdog killed the run. Also:
   **never return `next_token: null` to mean "no changes"** — echo the
   cursor you were given; null means "no resumable cursor exists at all".

2. **Honor `params.folder_id` in `list`.** The engine recurses folders
   explicitly — each call names the folder it wants (null = mount root). An
   adapter that always lists its configured root passes every flat-hierarchy
   test and then silently never imports nested content, while the walk
   re-queues the same root folders forever.

3. **Classify errors; the taxonomy IS the retry policy.** Throw
   `Error` with `e.code` set (see the `coded()` helper in any builtin):
   - `rate_limited` — 429/503/504. The ONLY code that requeues (backoff).
   - `auth_expired` — 401/403. Mount goes to `auth_required`; user reconnects.
   - `config_error` — 400/404 on reads: the same request will fail the same
     way forever. Mount is badged misconfigured, 15-min standoff. Retrying
     a config error is how an adapter burns a core and gets a tenant
     throttled.
   - `cursor_invalid` — expired delta cursor (Graph: 410 *or* 400 with
     `syncStateNotFound`). Engine drops the cursor and full-reconciles in
     the same run.
   - `conflict` — writes only: optimistic-concurrency loss. Routed through
     the mount's conflict policy, never retried blindly.
   - anything else — transient for READS (retried); for WRITES the default
     inverts: unrecognized errors on the drain do NOT retry (a retried send
     is a duplicate email).

4. **Capability honesty.** Declare in `capabilities` ONLY what is
   implemented and correct: `can_write`/`can_update`/`mutable_fields` per
   resource, `supports_changes`, `supports_push`. A declared-but-broken
   capability resolves the mount into a mode that throws at drain time,
   after the engine claimed candidates. Corollary: if you cannot return
   CORRECT data, refuse loudly (`config_error`) — the MS Graph drive
   `get_content` refuses rather than round-tripping binary through a text
   decode, because corrupted bytes that read as "fetched" are worse than no
   bytes. (`raisin.http.fetch` decodes responses as TEXT — binary payloads
   must arrive base64 from the provider, like Graph's `contentBytes`, and
   be returned as `content_base64`, never `content`.)

5. **Stable `external_id` and honest `etag`.** `external_id` keys the node
   for its lifetime — a provider id that changes (Graph mail ids change on
   folder moves unless you send `Prefer: IdType="ImmutableId"`) turns a
   move into delete+create and loses local state. `etag` drives skip-writes:
   an etag that bumps on irrelevant provider state forces a full re-map
   every sync; one that misses real changes drops updates. Prefer the
   provider's real change marker; fall back to `lastModified`.

## Cursors and paging (read side)

- `list` pages via `next_cursor` (null = done). `get_changes` pages via
  `next_token` + `has_more` (see rule 1).
- The engine persists the cursor **after each fully-materialized page** and
  resumes from it across runs — your cursor must be durable and re-runnable
  (engine upserts are idempotent by etag).
- Respect `params.limit`; the engine enforces `max_items_per_sync` and a
  wall-clock budget per run regardless. A huge backlog imports across many
  clean runs — design for resumability, not for one giant call.
- Emit deletions from `get_changes` as `{ type: "deleted", item:
  { external_id } }`. Be conservative: if the provider's "removed" can also
  mean "left the query window" (Graph calendarView), do NOT emit a delete
  you cannot attribute — the full reconcile prunes what is truly gone.

## Mappers (optional, bidirectional)

A mount may name a mapper function that reshapes provider items into typed
nodes. **One function, both directions** — `to_node` (default operation),
`to_external` (write-back), `mapper_capabilities` (`{ to_external: true }`):

- A mapper that declares a field mutable MUST round-trip it exactly, or
  echo-prevention breaks and the field re-pushes forever.
- `to_external` receives `fields` — the engine sends **only the fields that
  actually diverged**. Emit exactly what `fields` names, nothing more: some
  provider fields have side effects on mere presence in an update (Graph
  re-sends meeting invites to every attendee whenever `attendees` appears
  in a PATCH).
- Child nodes (mail attachments → `raisin:Asset` subnodes) are supported —
  see §6.2 of the contract.
- Calendars: map to `raisin:Event` (§6.1) and sync **series masters with
  recurrence rules**, never expanded occurrences — the engine's local
  expander projects occurrences. Emit exceptions as their own items with
  `series_master_external_id` + `original_start`.

## The write path (push local edits out)

Three modes, resolved from `write_config` ∩ your declared capabilities:

- `state_only` — allow-listed field updates via `update` (flag a mail read,
  edit an event title).
- `mirror` — updates + creates + deletes, behind blast-radius rails.
- `submit` — an outbox: the node is a COMMAND (send this mail). At-most-once,
  **never retried on an ambiguous answer**. Implement `submit` as a single
  provider call so the duplicate window is one HTTP request.

Engine guarantees you build against: pushes carry only diverged fields;
`__pushed_state`/`__etag` stamp-backs prevent your own writes echoing back
as changes; `update` receives the stored `etag` as the concurrency base —
throw `conflict` when the provider says the object moved on, never
overwrite silently.

## Push notifications (webhooks)

Implement `subscribe`/`renew`/`unsubscribe` for event-driven sync. The
engine renews on a 30-minute sweep — set your renewal window comfortably
inside the provider's lifetime (Graph: ~3 days). Ship a notification secret
in `clientState` (or equivalent) — deliveries without it are rejected and
counted.

## Testing

- **JS unit tests next to the code** (`index.test.mjs`, `node --test`):
  mapper round-trips (both directions!), `get_changes` termination shapes
  (fresh-token idle feed, `has_more` transitions), and the http status →
  error-code mapping. These are pure functions; testing them is cheap and
  every skipped one has become a production incident.
- **Rust harness tests** (`crates/raisin-functions/.../tests_ms_graph_adapter.rs`
  pattern) load your real JS through QuickJS and pin the wire contract.
- **Never trust a flat-hierarchy or idle-feed test alone**: add one nested
  folder and one no-changes delta poll to every adapter test matrix.
- Live end-to-end: create the integration + mount in the admin console,
  `Sync now`, watch the run history and the delivery panel.

## Ship it

An adapter ships as a builtin or installable package:

```
my-adapter/
  manifest.yaml
  nodetypes/            # connection/connector config nodetypes (UI hints go in `meta`)
  content/
    _raisin__system/connectors/<name>/.node.yaml   # connector registration
    functions/adapters/<name>/                      # the adapter (+ modules)
    functions/mappers/<name>/                       # optional mapper(s)
```

Multi-file adapters work in every execution path (the module map is loaded
by the engine). Keep files under 300 lines; split modules like the builtins
do (http.js, changes.js, read.js, write.js, mount.js).
