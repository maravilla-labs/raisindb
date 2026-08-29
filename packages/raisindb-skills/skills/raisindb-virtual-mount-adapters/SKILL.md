---
name: raisindb-virtual-mount-adapters
description: "Build custom connector adapters for RaisinDB virtual mounts: sync external systems (mail, calendars, drives, any API) into nodes and push local edits back. Covers the adapter operation contract (capabilities/list/get_changes/update/submit/subscribe), error taxonomy, cursors and has_more, bidirectional mappers, the receipt-etag and item-build-parity contracts, diverged-fields subsets, conflict policies (RemoteWins/LocalWins), write modes (state_only/mirror/submit), resolvers, echo prevention, the field-tested traps, and SHIPPING A MOUNT BUNDLE — the `mount_bundles` preset on your connector template that lets the admin console mint your connector's whole mount layout in one click, including per-entry target workspaces and the prompts that ask an operator for a mailbox, a site or a drive. Use whenever the user wants a connector, adapter, integration sync, virtual mount, two-way sync with an external service, or mentions raisin:VirtualMount, raisin:Integration, get_changes, or 'sync X into RaisinDB'."
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

## The seven rules that come from production incidents

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

6. **THE RECEIPT ETAG CONTRACT.** Every `update` (and `create`) must return
   a receipt `{ external_id, etag }` where `etag` is **the exact etag the
   next walk/delta will compute for the post-write state**. A `null` receipt
   etag means "keep the stale pre-write etag" — so the very next read sees a
   mismatch, rebuilds the node from the remote item, reseeds the baseline,
   and **silently reverts any local edit made while the run was in flight**
   (the Hue "can turn the light on but never off again" outage). What to DO:
   - If the provider echoes the updated resource on the write, derive the
     receipt etag from that echo **with the same formula the read path
     uses** (e.g. `@odata.etag || eTag || lastModifiedDateTime` — call the
     one shared function, never a re-implementation).
   - If the write response is bodiless or yields no etag by that formula,
     do a **read-after-write** (`get` the resource) and derive the etag from
     what came back. A write path that cannot answer with the post-write
     etag is not done.
   - Providers with no native etag at all: synthesize a **deterministic
     content hash** over the fields the item exposes — and because it is
     content-derived, the write path MUST read-after-write to hash the
     post-write state; hashing the request you sent is a guess, not a
     receipt.

7. **Item-build parity.** Every code path that builds an item — full walk,
   single `get`, `get_changes` feed, and the write-receipt path from rule
   6 — must produce **byte-identical metadata (and therefore the identical
   etag) for the same provider state**, including join-derived and enriched
   fields. One path filling a join field with `null` where another fills a
   value means two etags for one provider state, and every sync of that item
   is a spurious rebuild that clobbers pending edits (the Hue single-get vs
   full-walk divergence). What to DO: route all item construction through
   one shared `toExternalItem()`-style builder, and make single `get` fetch
   (or explicitly reproduce) every enrichment the walk performs — never let
   it return a "cheaper" shape of the same object.

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
- **`to_external` receives only the diverged subset.** The engine compares
  watched fields against the baseline (`__pushed_state`) and hands you
  `fields` containing **only the fields that actually diverged** — possibly
  a single one. Design `to_external` so that **every single-field subset of
  your mutable fields produces a valid, non-null payload**. Returning
  null/empty for any subset makes the push `Skipped`: the node stays
  diverged and is **re-nominated every drain, forever, with no request ever
  going out and no error surfaced** — a silent wedge. Test each mutable
  field alone in `fields`.
- **Group fields: trigger on ANY member, emit VALUES for ALL.** Fields that
  only make sense together on the wire (colour x/y pairs, start/end/timezone
  triples) must be pushed as the full group when *any* member appears in
  `fields`, reading the missing members' current **values** from the node —
  never gate the group on ALL members being in `fields`. The Hue mapper
  required both `color_x` AND `color_y` diverged; a one-coordinate edit
  emitted nothing and wedged forever. (Membership in `fields` is the
  trigger; the node's values are the payload.)
- **Aliases must clear every gate.** If a field has two spellings
  (`unread`/`is_read`), each alias must appear in the adapter's
  `mutable_fields`, the mapper's write allow-list, AND `to_node`'s output
  (so the baseline carries both and either edit diverges) — one missed gate
  means one spelling silently never writes back, while the other works
  (the ms-graph `is_read` outage). Pick a deterministic winner when both
  diverge at once.
- Outside the diverged-subset rules above, emit exactly what a push needs
  and nothing more: some provider fields have side effects on mere presence
  in an update (Graph re-sends meeting invites to every attendee whenever
  `attendees` appears in a PATCH).
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

Engine guarantees you build against: a sync run drains local edits FIRST,
then reads; there is no dirty flag — divergence is a value comparison
against the engine-owned `__pushed_state` baseline; pushes carry only
diverged fields; `__pushed_state`/`__etag` stamp-backs prevent your own
writes echoing back as changes; `update` receives the stored `etag` as the
concurrency base — throw `conflict` when the provider says the object moved
on, never overwrite silently. And return the post-write etag in the receipt
(rule 6) — the stamp-back is only as good as the etag you hand it.

### Read-path conflict semantics (know what the engine does to your nodes)

What happens when an incoming item's etag differs from the stored one
depends on the mount's conflict policy — and the adapter's receipt-etag
discipline decides how often that branch is even reached:

- **RemoteWins (default).** The node is **rebuilt wholesale** from mapper
  output and `__pushed_state` is **reseeded from the incoming item**. Any
  pending local edit is silently reverted AND de-nominated — it will never
  push. There is no partial merge.
- **LocalWins (read side).** Pending diverged watched fields keep their
  LOCAL values (a local delete stays deleted); the reseeded baseline keeps
  the OLD entries for exactly those fields; everything else — non-diverged
  watched fields, unwatched properties, the etag — follows the incoming
  item, so the follow-up drain pushes cleanly against the fresh etag.
  Incoming deletes still win. It protects **pending** edits only: an
  already-pushed edit replayed by a lagging delta is protected by nothing
  but your receipt etag matching.
- **LocalWins (write side, on a provider 409)** re-sends with etag null.

Therefore: **the receipt etag (rule 6) plus item-build parity (rule 7) are
the first line of defence** — with them, the read following your own push
computes the same etag, skips the item, and neither policy branch fires.
LocalWins is the safety net for edits made *while a run is in flight*, not
a licence to return sloppy receipts. Assume RemoteWins when writing the
adapter: a correct adapter never needs the net.

### Resolvers vs mirror updates

Two distinct write-back participants — pick by what the answer is:

- **Mirror/state_only `update`** is for edits that are pure provider state:
  the diverged fields map 1:1 onto a provider PATCH and the receipt closes
  the loop. Most fields belong here.
- A **resolver** (e.g. `resolvers/ms-graph-mail`) participates when the
  engine must adjudicate a conflict or a non-1:1 write — it renders a
  verdict from local + remote + baseline rather than blindly mapping
  fields. A resolver must apply the **same alias and polarity rules as the
  mapper** (ms-graph-mail: `is_read` mirrors `unread` with inverted
  polarity, checked after `unread` so verdicts match the mapper's payload
  precedence) — a resolver whose field rules drift from the mapper's
  produces verdicts the push then contradicts. If you ship both, test them
  against the same fixture set.

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
- **Receipt/walk etag parity tests**: assert
  `receipt.etag === toExternalItem(sameProviderState).etag` for every write
  path — normal echo, fallback-marker body, and the bodiless read-after-
  write path. Asserting mere etag *presence* passes with a stale etag and
  ships the clobber bug.
- **Single-field subset tests**: call `to_external` with each mutable field
  ALONE in `fields` (and each member of every group field alone) and assert
  a non-null payload every time — the forever-renominated wedge is
  invisible to round-trip tests that always pass full field sets.
- **Rust harness tests** (`crates/raisin-functions/.../tests_ms_graph_adapter.rs`
  pattern) load your real JS through QuickJS and pin the wire contract.
- **Never trust a flat-hierarchy or idle-feed test alone**: add one nested
  folder and one no-changes delta poll to every adapter test matrix.
- Live end-to-end: create the integration + mount in the admin console,
  `Sync now`, watch the run history and the delivery panel.

## Ship a mount bundle with your connector

**If your connector needs more than one mount, author a bundle.** This is the
difference between a connector an operator can install and one they can only
follow a README about.

A mount carries exactly one `write_config`, one delta cursor and one backfill.
So a connector whose resources need different write modes — an outbox of
commands beside a read-only ledger beside a two-way catalogue — is unusable
with fewer than N mounts, each of which is ten values only YOU know: the mapper
path, `sync_config.resource`, the mode, the `command_node_types`. Operators
rebuilt that set by hand, per tenant, from prose, and could not reproduce it.

Put it on the connector template as `raisin:Integration.mount_bundles` and the
admin console's **Mounts → Add bundle** mints the lot, asking only for what is
genuinely the operator's: connection, workspace, root folder.

```yaml
# content/_raisin__system/connectors/<name>/.node.yaml
properties:
  mount_bundles:
    - id: acme-workplace
      title: Acme
      default_workspace: workplace     # a suggestion; the operator confirms
      default_root: /acme
      prompts:                         # v5 — the operator's half, see below
        - key: site_id
          title: Site
          type: remote                 # remote | select | text
          browse: site                 # your adapter's `browse` kind
          required: true
          required_when: { scope: site }
          applies_to: [files]          # entry keys this answer is written onto
          target: sync_config.site_id  # sync_config.<key> | remote_root | account_ref
      mounts:
        - key: inbox
          title: Inbox
          subpath: mail/inbox
          default: true                # pre-selected in the picker
          remote_root: inbox           # bake a well-known id when you have one
          node_types: [raisin:Mail]    # checked against the workspace gate
          mapping_function: /mappers/acme-mail
          sync_config: { resource: mail, mode: hybrid, interval_seconds: 300 }
          write_config:
            writeback: 'off'           # QUOTE IT — see below
            mode: state_only
            mutable_fields: [unread]
        - key: files
          title: Files
          subpath: drives/acme
          target_workspace: assets     # v5 — this entry lands elsewhere
          root_override: /             #      with a root of its own
          node_types: [raisin:Asset]
          mapping_function: /mappers/acme-files
          sync_config: { resource: files, mode: hybrid, interval_seconds: 300 }
          write_config: { writeback: 'off', mode: 'off' }
```

Rules worth knowing before you author one:

- **Nothing server-side reads this.** It is instantiated client-side into
  ordinary `raisin:VirtualMount` nodes by `planBundle`
  (`packages/admin-console/src/api/integrations.ts`), and once created they owe
  the bundle nothing — they are edited like any other mount. So a bundle is a
  starting point, not a binding.
- **`node_types` is load-bearing.** The console checks it against the target
  workspace's `allowed_node_types` and refuses to create while a type is
  missing. Without that check the mount is created, rejects 100% of items,
  reports `outcome: "ok"`, and flips `backfill_complete` — permanently empty.
  Ship the matching `workspace_patches` in your manifest, `raisin:Folder`
  included, at the root level too.
- **`target_workspace` / `root_override` (v5)** let one bundle span workspaces.
  Use it when a resource belongs somewhere the rest does not — files as
  `raisin:Asset` in the asset library, say. Each destination is gated separately.
- **`prompts` (v5)** are the values only the operator knows, which you cannot
  bake: a mailbox, a SharePoint site, a drive. `applies_to` names the entries
  the answer is written onto; `required_when` hides a prompt until another
  answer matches; `target` is a CLOSED set (`sync_config.<key>`, `remote_root`,
  `account_ref`) and `planBundle` throws on anything else rather than silently
  dropping it. `type: remote` renders a picker over your adapter's `browse`
  operation — which is the reason to implement `browse` at all.
- **Quote `'off'`.** serde_yaml is YAML 1.1, where a bare `off` is boolean
  `false`, and `writeback`/`mode` are Strings. This has bitten twice.
- **Declare the mode your capabilities can actually serve.** A `submit` entry
  needs `command_node_types` covering its own `node_types` or it drains nothing;
  a `state_only` entry needs non-empty `mutable_fields`; `mirror` needs
  `can_create`/`can_update`/`can_delete`. A mode the adapter cannot serve is
  refused at drain time, after the engine has claimed the work.
- **Test the preset like code.** The bundle is data that ships to every tenant
  who clicks the button. Assert that every `resource` is one your adapter
  accepts, every mapper path exists, every `node_types` entry is in the
  workspace patch, every prompt `applies_to` names a real entry, and no
  interval is below your provider's rate-limit floor. `maravilla-connect`'s
  `tests/package.rs` is a worked example for both its bundles.

Two reference bundles ship today: Stripe (one workspace, seven resources, three
write modes) and Microsoft 365 (two workspaces, prompts, mail as four mounts
sharing one root).

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
    functions/resolvers/<name>/                     # optional resolver(s)
```

Multi-file adapters work in every execution path (the module map is loaded
by the engine). Keep files under 300 lines; split modules like the builtins
do (http.js, changes.js, read.js, write.js, mount.js).
