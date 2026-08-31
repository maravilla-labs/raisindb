# IMAP Mailbox Adapter

Mount an email inbox into a RaisinDB workspace path. The sync engine polls the
mailbox, maps each new message to a node, and keeps the subtree in sync — messages
become `raisin:Node` nodes (subject, from, to, date, snippet, message-id, unread
flag), mailboxes become `raisin:Folder` nodes.

This is the flagship **"agents work the inbox"** package: mount an inbox as an
**ephemeral** mount and every new message materializes as a short-lived node. A
`node_event` trigger fires an agent per message; once handled, the node expires on
its TTL so the inbox mount stays a rolling working set rather than an ever-growing
archive.

This package implements the frozen adapter contract in
`docs/reference/virtual-node-adapters.md`.

## Real IMAP over TLS (read this first)

This adapter speaks **real IMAP** (RFC 3501) over implicit TLS through the native
`raisin.imap.*` binding — Rust owns the protocol (TLS handshake, `LOGIN`, `SELECT`,
`UID SEARCH` / `UID FETCH`) and the function calls three high-level ops:

| Contract concept | How this adapter implements it |
|------------------|--------------------------------|
| Incremental cursor (`since_token`) | `UIDVALIDITY:UID` string |
| Delta feed (`get_changes`) | `raisin.imap.fetchSince(conn, sinceUid)` — messages with `UID > cursor` |
| Cursor reset / full resync | mailbox `UIDVALIDITY` changed → re-list from UID 0 |
| Message id (`external_id`) | IMAP `UID` |
| Folder listing (`list`) | `raisin.imap.listMailboxes(conn)` — mailboxes ONLY; see the full-resync warning below |
| Message body (`get` / `get_content`) | `raisin.imap.fetchMessage(conn, uid)` |

There is **no JMAP proxy** and no `raisin.http.fetch` in this package. The binding
opens a genuine TLS socket to your mail server. Egress is authorized by the adapter
function's `network_policy` (below), which the binding enforces on
`imaps://<host>:<port>` **before** any socket is opened.

> A JMAP-only provider (e.g. Fastmail's JMAP API) can still be integrated — but as a
> **separate** adapter package built on `raisin.http.fetch`. THIS package is real
> IMAP and connects to any RFC 3501 server (Gmail, Fastmail's IMAP endpoint,
> Outlook/Office 365, iCloud, Dovecot, self-hosted, …).

## What it ships

| Path | Workspace | Purpose |
|------|-----------|---------|
| `/adapters/imap` | `functions` | IMAP adapter function (`handler(input)`). |
| `/mappers/imap-default` | `functions` | Default per-message mapping function. |
| `/mappers/imap-outbox` | `functions` | Outbox mapper: `raisin:OutboundMail` → the message `submit` sends. |
| `/integrations/imap` | `raisin:system` | Pre-configured `raisin:Integration` template, **disabled**. Carries the `imap-mailbox` mount bundle (console: *Add bundle*), whose inbox is ephemeral with `reconcile_deletes: false` — see the warning below for why that flag is not optional. |

### Capabilities

Read, plus an optional **outbox** (`submit`). The adapter reports:

| flag | value |
|------|-------|
| `can_read` | `true` |
| `can_write` | `true` **iff** an email provider resolves for the mount (see *Sending*), else `false` |
| `can_submit` | same — declared only when a sender is genuinely resolvable |
| `can_create_folders` | `false` |
| `supports_changes` | `true` (UID-based delta via `fetchSince`) |
| `supports_webhooks` | `true` **iff** the mount sets `sync_config.pubsub_topic`, else `false` |
| `supports_push` | `true` **iff** the mount sets `sync_config.pubsub_topic`, else `false` |
| `supports_search` | `false` |
| `default_ttl` | `86400` (the ephemeral default — messages expire after a day) |
| `max_file_size` | `null` |
| `supports_idempotency_key` | `false` (SMTP has nowhere to put one) |

`supports_push` is **config-driven, not hard-coded**: it flips to `true` only for
a Gmail mount that has a `sync_config.pubsub_topic` (see *Gmail push* below). Every
other IMAP mount reports `supports_push: false`, so the engine keeps polling it —
the shared adapter never forces Gmail-specific behavior on a non-Gmail server.

`create` / `update` / `delete` throw "not supported" — there is no mirror surface,
so a `mirror` or `state_only` mount is refused, naming the missing flags. The one
write this connector has is `submit`.

A provider entry with no `from_address` also leaves it `false`: `EmailConfig::resolve`
ends with a `validate()` that requires one, so such an entry is selectable and then
refuses the send.

`can_submit` (and the `can_write` the engine demands alongside it) is declared
**only when `raisin.email.providers()` actually resolves a sender for this mount**
— not when a provider merely exists. Email switched off for the tenant, the named
entry disabled, or several enabled entries with no default all leave it `false`,
with the cause in `submit_unavailable_reason`. A capability declared with nothing
behind it makes the mount resolve as writable and then throw at drain time, after
the engine has already claimed the command.

## Sending: the outbox mount

**IMAP cannot send.** RFC 3501 has no submission verb, and `APPEND` files a copy
without delivering anything. So an outbox mount does not send over the mount's own
connection: it hands the message to the **tenant's configured email provider**
(`/config/email`, the console's *Email* page) through `raisin.email.send` — the
same SMTP/relay path every other server-side function uses.

Two consequences you must not discover from a recipient's inbox:

- **The mail is not sent from the mailbox this mount syncs.** `from`, `from_name`
  and `reply_to` come from the tenant's provider entry; a function chooses *which*
  configured sender to post through, never *who* it appears to be from. Configure
  an entry whose `from_address` is this mailbox and name it on the mount, or replies
  land somewhere the sender never looks.
- **No Sent copy is filed.** That would be an IMAP `APPEND`, which the native
  binding does not have (see *Not yet* below).

### Setting one up

1. **Grant the adapter function email.** Sending is deny-by-default per function.
   On `/adapters/imap`, set:

   ```yaml
   email_policy:
     enabled: true
     allowed_recipients: ["example.com"]   # globs on the recipient DOMAIN; "*" = anywhere
   ```

   It ships `enabled: false`: turning it on is a decision about this tenant's
   sending reputation. `enabled: true` with an empty list still denies everything.
   `secret_policy` already grants `email/*`, which is what lets the send resolve the
   provider's credential.

2. **Name the sender on the mount** (optional when the tenant has exactly one
   enabled provider, or a `default_provider`):

   ```yaml
   sync_config:
     email_provider: transactional   # a name from /config/email
   ```

3. **Create the mount** with the outbox mapper and `submit` mode:

   ```yaml
   mapping_function: /mappers/imap-outbox
   write_config:
     mode: submit
   ```

   Conventionally `/mail/outbox`, beside the read-only inbox mount.

Then create a `raisin:OutboundMail` node under it. It is born `draft`; moving
`status` to `queued` is what authorizes the send. The engine claims it durably
(`queued -> sending`, stamped with an `attempt_id`) before the provider is called,
so a crash leaves evidence instead of an unbounded question.

### What is mapped, and what is declined

Only `action: send`. `reply` / `reply_all` / `forward` are **refused**: threading them
needs `In-Reply-To` / `References` headers on the outgoing message, and
`raisin.email.send` accepts no headers — a fresh message with a `Re:` subject breaks
the recipient's thread silently. The refusal is the ADAPTER's, not the mapper's: the
mapper passes the action through, `submit` throws a `config_error` naming the missing
headers, and the command settles `failed` with **that** text. (It is claimed
`queued -> sending` first — the refusal costs one state transition and buys a reason
the author can act on.)

A command with no recipient, no subject or no body is declined by the mapper
(`to_external` returns `null`). The reason the author sees is then the ENGINE's
generic one — *"the mount's mapping function declined this command (to_external
returned null) — either it is not finished being authored, or it has already been
sent and must not be sent again"* — because a mapper has no channel for stating its
own reason. Giving it one is an open design item.

An HTML-only body gets a plain-text alternative generated from it, because the email
API requires a non-empty `text` and would otherwise refuse every message composed in
a rich editor. `importance` and attachments are not carried yet.

### How a failed send is treated

| The send failed with | Command lands at | Why |
|---|---|---|
| `[email:policy_denied]`, `[email:config]`, `[email:invalid_message]`, `[email:unsupported]`, `[email:auth_failed]` | `failed` | Refused before a socket, or by the provider before it looked at the message. Nothing left; a person edits and requeues. |
| `[secrets:policy_denied]` (and any other secret-resolution failure) | `failed` | The provider entry's `credential_ref` is outside this function's `secret_policy.allowed_names`. It refuses before the socket, and the capabilities probe cannot pre-empt it — `providers()` deliberately carries no `credential_ref`. |
| `[email:rate_limited]` | back to `queued` | A 429 is the provider refusing to *look* at the request — the only answer that proves nothing was sent. |
| `[email:transport]`, `[email:timeout]`, `[email:provider_error]` | `unknown`, never auto-retried | A relay timeout most often means the message *did* arrive and the acknowledgement was lost. Resending delivers a second copy to a real person. Check the provider for the recorded `attempt_id` before requeueing. |

### Not yet (both need Rust, not JS)

- **Filing a Sent copy** needs an IMAP `APPEND` binding; `raisin.imap` today is
  `fetchSince` / `listMailboxes` / `fetchMessage` only.
- **Flag writeback** (marking a message read, `UID STORE`) needs the same kind of
  new binding, which is what a `state_only` mount would require. Until then
  `unread` is imported and never pushed.

## Connection: host, port, TLS, mailbox

Connection settings come from the **mount's `sync_config`** (the adapter never reads
the integration node):

| `sync_config` field | meaning | default |
|---------------------|---------|---------|
| `host` | IMAP server hostname (e.g. `imap.gmail.com`) | — (required) |
| `port` | IMAP port (implicit TLS) | `993` |
| `tls` | implicit TLS — only `true` is supported today | `true` |
| `mailbox` | IMAP folder to sync (falls back to `remote_root`, then `INBOX`) | `INBOX` |

Because the binding enforces the network policy on `imaps://<host>:<port>`, you must
add your mail host to the adapter function's `network_policy.allowed_urls` in
`content/functions/adapters/imap/.node.yaml`. The template ships Gmail, Fastmail,
Outlook, and iCloud; add yours as `imaps://<host>:993`. Note that
`network_policy.http_enabled: true` is the **master switch for all native egress**
(HTTP *and* IMAP) — leave it `true`.

## Authentication: app password vs XOAUTH2

Credentials are supplied as a **connected account** and reach the adapter as
`input.credential`: a `username` (usually the full email address) plus a secret. The
adapter reads the secret from `credential.password`, `credential.app_password`, or
`credential.access_token` (first non-empty wins). The secret is never logged.

- **App password (default).** Most providers require an app-specific password for
  IMAP when 2FA is on (Gmail → Google Account → Security → App passwords; Fastmail →
  Settings → Privacy & Security → App passwords). Create one scoped to mail, connect
  an account in the admin console, and paste `username` + the app password. Set
  `api_config.auth_mode: app_password`.
- **XOAUTH2 (OAuth).** For providers that issue OAuth access tokens for IMAP (e.g.
  Gmail), complete `oauth_config` and connect via the OAuth flow; the engine stores
  the refresh token encrypted and passes the adapter only a current `access_token`,
  used as the IMAP login secret. Set `api_config.auth_mode: oauth`. (The binding
  logs in with LOGIN/plain today; XOAUTH2 SASL is a follow-up if a provider needs it.)

Refresh tokens never enter the function sandbox. On a rejected LOGIN the binding
throws `[imap:auth_expired]`, which the adapter re-throws as `code: "auth_expired"`
so the engine pauses the mount for re-auth.

## Connecting the integration

1. Open the admin console → **Connectors** → **IMAP Mailbox**.
2. Choose the auth mode and connect an account (username + app password, or OAuth).
3. Add your IMAP host to the adapter's `network_policy.allowed_urls`
   (`imaps://<host>:993`) if it is not one of the shipped defaults.
4. **Enable** the connector.

## Mounting an inbox (the ephemeral pattern)

Create a `raisin:VirtualMount` (admin console → **Mounts**, or a node under
`raisin:system/mounts`) pointing at this connector. The ephemeral settings are what
make it a rolling working set:

```yaml
node_type: raisin:VirtualMount
properties:
  title: Support Inbox
  integration_ref: /integrations/imap
  account_ref: "<connected_accounts[].id>"
  target_workspace: default
  target_branch: main            # branch the synced message nodes are written to
  mount_path: /inbox
  remote_root: INBOX             # mailbox name (also settable via sync_config.mailbox)
  # mapping_function: /mappers/imap-default   # optional; the engine's built-in
  #   Folder/Node mapping is used if omitted
  sync_config:
    mode: poll
    interval_seconds: 60
    max_items_per_sync: 200
    ephemeral: true              # auto-delete synced nodes past their TTL
    ttl_seconds: 86400           # 1 day — matches the adapter's default_ttl
    host: imap.gmail.com         # IMAP server host
    port: 993                    # implicit TLS
    tls: true
    mailbox: INBOX               # IMAP folder to sync
  enabled: true
```

`sync_config.host` / `port` / `tls` / `mailbox` are read by the adapter (it never
sees the integration node), so set them on the mount. The engine runs a full
reconcile on first sync (folders via `list`), then incremental `get_changes` deltas
keyed on the `UIDVALIDITY:UID` cursor. When the mailbox's `UIDVALIDITY` changes, the
adapter forces a full re-list from UID 0. Writes run under the `virtual-mount-sync`
system actor.

### ⚠️ Forcing a FULL resync prunes every message node — unless the mount says so

`list` on this adapter enumerates **mailboxes only** — messages arrive through
`get_changes`, never through the walk. The engine's full reconcile does not know
that: it treats everything the walk did not yield as stale and stages it for
deletion. `seen` holds mailbox ids, so it is non-empty and the engine's
empty-reconcile guard does not fire either. A forced full run (the console's
*Full resync* button, or a remap) therefore deletes **every message node under
the mount**, and they do not come back: the walk is followed by a delta baseline,
which asks `get_changes` for a cursor and discards its items, so the next
incremental pass resumes at the highest UID already seen and returns only NEW
mail.

**The fix, and it is now shipped as a default:**

```yaml
sync_config:
  reconcile_deletes: false   # this mount's `list` is not authoritative for its own content
```

The engine honours the key (default `true`); a mount that sets it false is never
pruned by the walk. It was inert for this connector until it had a mount that set
it — i.e. it protected only an operator who had read this section. The connector
template now ships a **mount bundle** (`raisin:Integration.mount_bundles`, the
admin console's *Add bundle*) whose inbox and outbox entries both set it, so a
mount created the ordinary way is safe by default rather than by documentation.

Ephemeral + `ttl_seconds` is still the layout to use: with no CONDSTORE and no
EXPUNGE feed in the binding, a message deleted on the server is invisible to this
adapter, and the TTL is the only thing that retires it. **On a non-ephemeral IMAP
mount with `reconcile_deletes` left on, the message nodes are lost permanently.**

Making `list` enumerate messages is **not** the fix: it costs a full mailbox
fetch every run and re-opens the full-vs-delta `relative_path` divergence.

## Subtrees: `sync_config.folder_scope`

Default `folder` — one mailbox, exactly as before. `tree` syncs the mount's
mailbox **and every mailbox beneath it**, materialising the folder hierarchy and
filing each message under its own folder.

```yaml
sync_config:
  mailbox: INBOX
  folder_scope: tree
  ephemeral: true
  ttl_seconds: 86400
  reconcile_deletes: false
```

What changes, and none of it is optional reading before switching a live mount:

- **The message id space changes.** IMAP UID and UIDVALIDITY are per mailbox, so
  the bare UID collapses `INBOX` uid 5 and `Archive` uid 5 onto one node. Tree
  mode's `external_id` is `<mailboxPath>|<uidvalidity>.<uid>`. Switching an
  existing mount from `folder` to `tree` therefore **re-imports it once**, with
  new node ids and per-node history restarting — the same one-time cost the
  ms-graph immutable-ids migration documents. The cursor carries the scope
  family (`rsn-imaptree-1:`), so the flip re-baselines rather than resuming a
  cursor that means something else.
- **The path leaf becomes `<uidvalidity>.<uid>`, not the subject.** Folder mode
  files a message at its bare subject; two messages sharing a subject collide
  there, and the walk would place them somewhere else again. Tree mode files at
  `<mailbox chain>/<uidvalidity>.<uid>`, which is what lets the walk's folder
  path and the delta's message path agree byte for byte. The leaf is **the tail
  of the `external_id`**, deliberately and by construction — one function emits
  both. If it were the bare UID they would disagree the moment a mailbox's
  **UIDVALIDITY changes** (restored from backup, server renumbering): the id
  carries the uidvalidity and so becomes a new id, while the mailbox
  re-enumerates from UID 0 and the paths repeat — a *new* node aimed at a path
  the *old* node still occupies. `add_node` refuses that with a path conflict,
  which the materializer treats as item-level: the message is **skipped and the
  run still reports `ok`**, for every message, on every run, so a restored
  mailbox silently stops importing. Nothing clears the old nodes either: a tree
  mount runs `reconcile_deletes: false` and the walk never enumerates messages,
  so only `ttl_seconds` retires them. With the uidvalidity in the leaf, a reset
  instead costs one re-import of that mailbox at fresh paths, and the stale
  nodes age out.
  The separator is `.` and not `:` because the leaf is a path segment as well as
  an id: name sanitisation keeps `[a-z0-9-_.]` and drops the rest, so `100:5`
  and `10:05` would both reduce to `1005`. It is less readable than a subject,
  deliberately; `path_template` cannot restore the old layout, because the
  engine sanitises every placeholder value and would flatten a chain into one
  segment.
- **It costs one IMAP login per mailbox per poll.** Every binding call opens its
  own TCP + TLS + LOGIN and logs out at the end. The adapter advances a bounded
  slice of mailboxes per `get_changes` call (5 today) from a rotation index
  persisted **inside the cursor**, so no mailbox starves — but the steady-state
  cost is real. For Gmail specifically, Google documents **15** simultaneous
  clients per account ([Add Gmail to another email
  client](https://support.google.com/mail/answer/7126229) — past it you get
  `Too many simultaneous connections`) and an IMAP download ceiling of **2500
  MB/day** ([Gmail bandwidth
  limits](https://knowledge.workspace.google.com/admin/gmail/gmail-bandwidth-limits),
  which also states the limits may change without notice). A mount spanning
  more than **50** mailboxes is refused with
  `config_error`: mount a subtree instead. (A `raisin.imap.fetchSinceMulti`
  binding — one session, N SELECTs — would raise the slice to 25; the adapter
  uses it automatically if it appears.)
- **Gmail's `[Gmail]/All Mail`, Trash and Spam are skipped**, together with
  everything under them, keyed on their RFC 6154 SPECIAL-USE attributes
  (`\All`, `\Trash`, `\Junk`) and never on their names, which Gmail localises.
  All Mail re-lists every message in the account; syncing it would import the
  whole mailbox a second time under a second path. A server that advertises no
  SPECIAL-USE attributes gets none of this — use `exclude_patterns`, which works
  because the mailbox chain *is* the relative path in tree mode.
- **A poll delivers at most `max_items_per_sync` per mailbox, and the rest are
  stepped over for good.** See *Known limit: the truncation cliff* below — it is
  not fixable inside this package, and the mitigation is a config value.
- **The delta baseline seeds a slice at a time.** The engine asks for a baseline
  exactly once and never pages it, and a baseline that throws leaves the mount
  re-walking the provider on every run. So the baseline probes one message in
  each of the first few mailboxes, marks the cursor unfinished, and the ordinary
  polls seed the remainder — emitting nothing for a mailbox until it has a
  watermark, which is the same "from now on" the rest of the adapter has.
- **Deletes still need `reconcile_deletes: false`.** The walk enumerates
  mailboxes in tree mode too, so it is authoritative for folders and never for
  messages.
- **The delta re-asserts the folder tree, and it has to.** A tree mount is
  `ephemeral: true` with a 24h TTL, and the engine's TTL sweep deletes *every*
  mount-owned node whose `__synced_at` has aged out — there is no exemption for
  folders. The walk that created them runs exactly once (after
  `backfill_complete` the engine only calls `get_changes`), so without this the
  whole hierarchy was deleted a day after the backfill; the next message then
  re-created its parent through the store's auto-parent path as a stub with no
  `__mount_id`, and from that point every walk that tried to stage the real
  folder was skipped forever as *"foreign node occupies target path"* — the
  mount could never own its own folders again, and a mailbox deleted upstream
  could never be pruned. So `get_changes` emits the folder set at the start of
  each rotation round, before the messages, using the same chain and the same
  etag spelling the walk uses. On an ephemeral mount that etag carries a
  freshness bucket of `ttl_seconds / 3`: an unchanged etag is *skipped without
  re-stamping* `__synced_at`, so a folder that never changed would still have
  been swept. Cost: each folder node is rewritten three times a day, and never
  on a mount that is not ephemeral. It is also what gives a mailbox created
  *after* the backfill a real folder node instead of the same stub.
- **The hierarchy delimiter is read per mailbox**, not guessed. The binding does
  not yet forward the delimiter it already reads from each `LIST` response
  (`MailboxInfo` carries only `name` / `path` / `flags`), so the adapter derives
  it from the path — exactly, and per mailbox, as RFC 2342 requires. It prefers
  `MailboxInfo.delimiter` the moment the binding starts sending it.

## Known limit: the truncation cliff (lives in the Rust binding, not this package)

**A mailbox that gains more than `max_items_per_sync` messages between two
visits loses the OLDEST of them permanently.** Not "until the next poll" —
permanently, until someone forces a full re-import.

Where it lives, exactly: `crates/raisin-functions/src/runtime/imap/client.rs`,
`fetch_since_inner`. It runs `UID <cursor+1>:*`, sorts the hits ascending, and
then — when there are more than `limit` of them — does
`uids.split_off(uids.len() - limit)`, i.e. **keeps the newest `limit` and
discards the rest**. The watermark it returns is the highest UID it *fetched*.
So the cursor jumps *over* the discarded UIDs, and the next call asks for
`UID > <that highest>`. The binding has no oldest-first mode and no UID-range
fetch, so there is no second call that could ask for them again.

Trigger condition, precisely — all of these at once:

1. a single mailbox receives **more than `max_items_per_sync`** new messages
   (default **200**; `sync_config.max_items_per_sync`),
2. within the interval between two `get_changes` visits to *that mailbox*, and
3. the mount already has a cursor for it (a first sync from UID 0 has the same
   cliff against the mailbox's whole history, which is why the tree baseline
   seeds a watermark and deliberately imports nothing older).

Tree mode does not cause this and cannot fix it — it is identical in folder
mode — but it does **widen the window**: a tree mount visits each mailbox once
per rotation round, so with N mailboxes and a slice of 5 the interval between
two visits to one mailbox is roughly `ceil(N / 5)` polls, not one.

What the adapter does about it: nothing it can. `has_more` gets the *other*
mailboxes seen sooner; it does not recover a stepped-over message. The
per-mailbox page is deliberately the **whole** item budget rather than a
fraction of it (dividing it across the slice lowered the cliff fivefold for no
gain).

Mitigations, in order of preference:

- **Raise `max_items_per_sync`** above the burst you expect between visits
  (this is the only real lever). The adapter clamps it to **1000**, so a burst
  larger than that cannot be covered this way at all; and on a tree mount keep
  the mailbox count low so rounds are short.
- **Poll more often** — but not below the platform baseline; tightening poll
  intervals is how accounts get rate-limited.
- **Mount a subtree** rather than a whole account, so each mailbox is visited
  every round or two.

The actual fix is in the binding: either `fetch_since` keeps the **oldest**
`limit` UIDs and reports *that* slice's highest as the watermark (so the rest
are re-offered on the next call), or it grows a UID-range fetch. Both are Rust
changes; neither can be emulated from this package.

## Worked example: fire an agent per new message

Because synced messages are ordinary nodes, a standard `node_event` trigger reacts
to each new one. Register a trigger scoped to the mount path that dispatches an
agent (pseudocode for a `raisin:Function` trigger body):

```javascript
// Trigger: on node.created under /inbox where node_type == raisin:Node
function handler(input) {
  const msg = input.event.node;             // the freshly synced message node
  const p = msg.properties;

  // Skip anything already handled or read.
  if (p.unread !== true) return;

  // Hand the message to an agent to triage/answer. Keep it to a single
  // dispatch — do not fan out per-item work in the sync hot loop.
  raisin.agents.dispatch("support-triage", {
    subject: p.title,
    from: p.from,
    snippet: p.snippet,
    message_id: p.message_id,
    node_path: msg.path,
  });
}
```

Wire it as a trigger on the target workspace filtered to `node.created` +
`mount_path` prefix (see the events/triggers docs for the exact registration
surface). The flow is: **new mail → IMAP delta → materialized node → `node_event`
→ agent**. When the agent finishes and the TTL lapses, the ephemeral node is
reaped, keeping `/inbox` a live queue rather than an archive.

> The `raisin.agents.dispatch` / trigger-registration calls above are illustrative
> of the intended wiring; use your deployment's actual agent-dispatch and
> trigger-registration APIs. The load-bearing contract this package guarantees is
> that each new message arrives as a `raisin:Node` under `mount_path` with the
> properties shown, carrying the reserved `__virtual` / `__mount_id` /
> `__external_id` metadata the engine stamps.

## Gmail push via Pub/Sub (Experimental / Preview)

By default this adapter **polls**. A Gmail mount can instead be **pushed**: Gmail
notifies RaisinDB the instant new mail lands, and the engine re-syncs on the spot
instead of waiting for the next poll. Gmail has **no direct webhook**, so push
rides Google Cloud Pub/Sub.

**Push is only an invalidation signal.** The Pub/Sub message body (a `historyId`)
is **ignored**. A ping means nothing more than *"re-run this mount's normal delta"* —
the engine then does the exact same IMAP `get_changes` (UID delta) it does when
polling. So push and poll share one code path; push just removes the wait. Nothing
in the mail path changes: messages still arrive over real IMAP.

### The chain

```
new mail → Gmail users.watch → Pub/Sub topic → Pub/Sub push subscription
        → HTTPS POST to the mount's notifications endpoint
        → engine sync_now(mount, "delta") → IMAP get_changes (UID delta) → nodes
```

### Who owns which hop

| Hop | Owner | What happens |
|-----|-------|--------------|
| Create the Pub/Sub **topic** | **Operator** (one-time, in GCP) | The adapter cannot create it — `users.watch` only *targets* an existing topic. |
| Grant Gmail permission to publish | **Operator** | Give `gmail-api-push@system.gserviceaccount.com` the **Pub/Sub Publisher** role on the topic. |
| Create the Pub/Sub **push subscription** | **Operator** | Point its push endpoint at the mount's notifications URL (below). |
| `users.watch` / `users.stop` | **Adapter** (`subscribe`/`renew`/`unsubscribe`) | Arms/renews/stops the mailbox against the topic. |
| Renewal scheduling, endpoint auth, `sync_now` | **Engine** | Calls `renew` before the ~7-day expiry; verifies the inbound POST; enqueues the delta sync. |

### Operator setup (one-time)

1. **Create a topic:** `gcloud pubsub topics create <your-topic>` in the GCP
   project whose OAuth client you use for Gmail.
2. **Let Gmail publish to it:**
   `gcloud pubsub topics add-iam-policy-binding <your-topic>
     --member=serviceAccount:gmail-api-push@system.gserviceaccount.com
     --role=roles/pubsub.publisher`
3. **Find the mount's notifications endpoint.** The engine assigns each push-enabled
   mount an unguessable URL of the shape
   `https://<your-raisindb-host>/api/integrations/{repo}/notifications/{push_mount_token}`.
   Read it off the mount's state (`push_notification_url`) after enabling push.
4. **Create a push subscription** targeting that URL:
   `gcloud pubsub subscriptions create <sub> --topic=<your-topic>
     --push-endpoint="<notifications_url>"`
   Optionally set `--push-auth-service-account=<sa>` so Pub/Sub attaches an OIDC JWT
   (see *Verifying the push* below).
5. **Configure the mount** (see below) with `pubsub_topic` (required) and, optionally,
   `pubsub_verify_token`. Enable it. The engine calls the adapter's `subscribe`
   (`users.watch`) and the flow is live.

### Mount config

Set these on the **mount's `sync_config`** (the topic is per-deployment, so it lives
on the mount, not the shipped integration template):

| `sync_config` field | meaning |
|---------------------|---------|
| `pubsub_topic` | **Required for push.** `projects/<project>/topics/<topic>` — the topic `users.watch` targets. Absent ⇒ `supports_push:false` ⇒ poll-only. |
| `pubsub_verify_token` | Optional shared secret echoed back as the subscription `secret`; the engine can require inbound pushes to carry it. |
| `mode` | Use `hybrid` — push **plus** a slow poll as a safety net (recommended). `webhook` disables polling entirely (push-only). |

```yaml
sync_config:
  mode: hybrid              # push + a slow poll backstop
  interval_seconds: 300     # backstop cadence while push is live
  host: imap.gmail.com
  port: 993
  tls: true
  mailbox: INBOX
  pubsub_topic: projects/my-proj/topics/raisin-gmail
  pubsub_verify_token: "s3cret-optional"
```

Push requires an **OAuth (XOAUTH2) account** — the `subscribe`/`renew`/`unsubscribe`
ops call the Gmail REST API with the same access token used for XOAUTH2 IMAP (the
`https://mail.google.com/` scope already grants the Gmail API). An **app-password**
mount has no bearer token, so push is unavailable there and the ops throw
`auth_expired`; use polling instead.

### What the adapter does

- **`subscribe`** → `POST https://gmail.googleapis.com/gmail/v1/users/me/watch`
  with `{ topicName: <pubsub_topic>, labelIds: ["INBOX"] }`. Returns
  `subscription_id: "gmail-watch:<email>"`, `secret: <pubsub_verify_token>`, and
  `expires_at` (Gmail's `expiration`, ~7 days out, converted to ISO-8601). If
  `pubsub_topic` is missing it **throws a clear config error** — it never silently
  no-ops.
- **`renew`** → re-runs `users.watch` for a fresh `expires_at`. Gmail's watch lapses
  in ~7 days and Google recommends re-calling it about daily; the engine's renewal
  job drives this before expiry.
- **`unsubscribe`** → `POST .../users/me/stop` → `{ ok: true }`.

The `historyId` Gmail hands to Pub/Sub is deliberately unused — RaisinDB re-deltas
over IMAP rather than trusting the notification payload.

### Verifying the push

The inbound POST from Pub/Sub is authenticated two ways; use either or both:

- **Endpoint token + verify token (default).** The notifications URL embeds a
  per-mount unguessable `push_mount_token`, and the optional `pubsub_verify_token`
  is stored as the subscription secret — an attacker cannot forge either.
- **OIDC JWT (optional, GCP-native).** If the push subscription is created with a
  push-auth service account, Pub/Sub attaches a Google-signed OIDC JWT in the
  `Authorization` header. Operators can have RaisinDB verify it with the generic
  `raisin.crypto.verifyJwt` binding (issuer `https://accounts.google.com`, audience =
  your endpoint). This is **not** hard-wired into the adapter — no GCP-specific code
  ships in the connector; it stays a generic, opt-in verification step.

## Security notes

- The connection secret (`credential.password` / `app_password` / `access_token`) is
  passed to the binding and **never logged**; the `ImapConn` redacts it from every
  debug/trace render. Refresh tokens never enter the function sandbox — on `LOGIN`
  rejection the adapter throws `auth_expired` and the engine refreshes or pauses the
  mount. On provider throttling it throws `rate_limited` and the engine backs off.
- Outbound IMAP is restricted by the adapter's `network_policy.allowed_urls`
  (`imaps://<host>:993`); the binding refuses to open a socket to any host that does
  not match, before connecting. Widen it deliberately for your provider.
- The integration template ships `enabled: false` and contains **no** credentials.
