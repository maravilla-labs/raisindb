# Microsoft 365 Adapter (Preview)

> **Experimental / Preview.** This connector is shipped as a preview feature.
> Validate it against your own Microsoft 365 account before relying on it in
> production.

Mount an Outlook **mail folder** or a **calendar** into a RaisinDB workspace path.
The sync engine polls Microsoft Graph, maps each item to a node, and keeps the
subtree in sync — mail messages become message-ish `raisin:Node` nodes, calendar
events become `raisin:Event` nodes.

This package implements the frozen adapter contract in
`docs/reference/virtual-node-adapters.md` over the Microsoft **Graph v1.0** REST
API using the synchronous `raisin.http.fetch` binding.

## What it ships

| Path | Workspace | Purpose |
|------|-----------|---------|
| `/adapters/ms-graph` | `functions` | Graph v1.0 adapter function (`handler(input)`). |
| `/mappers/ms-graph-mail` | `functions` | Mail message → `raisin:Node` mapper. |
| `/mappers/ms-graph-calendar` | `functions` | Calendar event → `raisin:Event` mapper. |
| `/integrations/ms-graph` | `raisin:system` | Pre-configured `raisin:Integration` template, **disabled**. |

### Capabilities

`can_read` and `supports_changes` are `true`; the adapter is **read-only**
(`can_write`, `can_create_folders` are `false`). Webhooks, search, and push are
not implemented.

## Mail vs calendar vs files

The mount's `sync_config.resource` selects the surface (default `"mail"`).
`{principal}` and `{drive}` are resolved by the next section — by default they
are `/me` and `/me/drive`.

| `resource` | List | Delta (`get_changes`) | `remote_root` |
|------------|------|------------------------|---------------|
| `"mail"` (default) | `{principal}/mailFolders/{id}/messages` | `…/messages/delta` | mail folder id, default `inbox` |
| `"calendar"` | `{principal}/calendars/{calId}/events` | `…/calendarView/delta` bounded by `sync_config.window` | calendar id, default `calendar` |
| `"files"` | `{drive}/root/children` | `{drive}/root/delta` | drive item id, default the drive root |

For calendars, `sync_config.window` bounds the `calendarView` delta:
`{ days_back: 7, days_ahead: 30 }` by default.

Every item's `external_id` and `name` are the **Graph item id**, never the
filename or subject, so two items can never collide on a path and a rename
never moves a node.

**A provider-side MOVE is a two-step story, by design.** When a file is moved to
a different OneDrive folder, the next ordinary sync updates the node's
`parent_id` property — the truth is in the data immediately — but leaves the
node where it is in the tree: an upsert matched by `external_id` deliberately
preserves the existing path, which is what stops a rename from creating
duplicates. To re-shape the tree to the provider's current hierarchy, run a
**Remap** on the mount (`Remap` in the admin console, or `mode: "remap"`). That
runs a relocation pre-pass which moves each node to its newly-mapped path
**preserving the node id**, so revision history and anything added locally
survive — unlike a delete-and-recreate, which loses both. The same applies after
changing `path_template`, which is the case remap was built for.

## Shared mailboxes and SharePoint

By default a mount reads the connected account's own data. Two settings change
that. Both may be set on the **connection** (applies to all its mounts) or on an
individual **mount**, which wins.

| Setting | Effect |
|---|---|
| `principal` | Whose mailbox / calendar / OneDrive. Blank ⇒ `/me`; an address ⇒ `/users/{upn}`. This is how a **shared mailbox** is mounted. |
| `drive_scope` | `files` only: `me` (own OneDrive), `user` (the `principal`'s OneDrive), `site` (a SharePoint library, with `site_id` and optional `drive_id`). Inferred when unset — a `site_id` implies `site`, a `principal` implies `user`. |

```yaml
# A shared mailbox
sync_config:
  resource: mail
  principal: sales@contoso.com

# A SharePoint document library (default library of the site)
sync_config:
  resource: files
  drive_scope: site
  site_id: contoso.sharepoint.com,<siteGuid>,<webGuid>
```

SharePoint items are ordinary Graph `driveItem`s, so they sync through the same
`/mappers/ms-graph-files` mapper as OneDrive — there is no separate resource or
mapper for SharePoint.

**Permissions are not granted here.** Reading another principal's data needs the
matching delegated scope on the app registration *and* the access itself in
Exchange (Full Access on the shared mailbox) or SharePoint. A mount naming a
mailbox the account cannot open is indistinguishable from a working one until
the first sync — run **Test connection** before enabling it.

`external_id`, node `name`, and `relative_path` are always the **Graph item id**
(never the subject/title), so distinct items never collide on a path. The
human-readable subject is written to the `title` property by the mappers.

## Microsoft Entra (Azure AD) setup

1. In the [Microsoft Entra admin center](https://entra.microsoft.com/) register a
   new application.
2. Add a **Web** redirect URI matching your deployment's callback, e.g.
   `https://<your-host>/api/integrations/oauth/callback`.
3. Under **API permissions**, add the delegated Microsoft Graph scopes below and
   grant consent.
4. Under **Certificates & secrets**, create a **client secret**.
5. Copy the **Application (client) ID** and the **client secret value**.

### OAuth scopes

The template requests least-privilege delegated scopes:

- `offline_access` — issue a refresh token for unattended sync.
- `Mail.Read` — read Outlook mail.
- `Calendars.Read` — read calendars and events.
- `Files.Read` — read the connected account's OneDrive.

Needed only for the cross-principal mounts described above:

- `Mail.Read.Shared` — shared mailboxes.
- `Calendars.Read.Shared` — shared calendars.
- `Files.Read.All` — another user's OneDrive.
- `Sites.Read.All` — SharePoint document libraries.
- `User.ReadBasic.All` — the directory list behind the mailbox / user pickers.
- `Place.Read.All` — the room-mailbox picker.

These are **delegated**, not application, permissions: the connected account
still reaches only what a human has been granted, so adding them does not widen
access on its own.

#### What delegated permissions cannot do

Worth knowing before mounting shared resources, because these are Microsoft's
limits and not something this adapter can work around:

- **A shared mailbox cannot be positively identified.** `mailboxSettings/
  userPurpose` is the only authoritative signal in v1.0, and reading it for
  anyone but the signed-in user needs the APPLICATION permission
  `MailboxSettings.Read`. The picker therefore labels an unlicensed,
  sign-in-blocked directory object as a *likely* shared mailbox and leaves
  "Test connection" as the real proof of access.
- **Shared and delegated resources cannot receive webhooks.** Microsoft states
  that `Mail.Read.Shared` / `Calendars.Read.Shared` do not support change
  notification subscriptions; that needs application `Mail.Read` /
  `Calendars.Read`. A shared mount therefore polls its delta feed instead of
  being pushed to.
- **A shared PRIMARY calendar is not in `/me/calendars`.** Only accepted shares
  of *secondary* calendars are copied into the recipient's mailbox, so the
  picker offers "browse another user's calendars" as its own step.
- **Only the primary calendar has a delta feed.** v1.0 documents
  `calendarView/delta` at the mailbox level only, so a mount pointed at a
  secondary or shared calendar reports `supports_changes: false` and
  full-reconciles on every run.

Adding a scope to an already-configured connector requires **reconnecting each
account** — Microsoft issues consent only on a fresh authorization. The package
manifest deliberately pins the integration node to `mode: skip`, so a package
update never rewrites a live connector's scopes, credentials or accounts; make
scope changes in the admin console.

The endpoints use the `common` authority (work/school **and** personal accounts);
swap `common` for a tenant id in the integration's `oauth_config` to restrict to a
single tenant.

## Connecting the integration

1. Open the admin console → **Connectors** → **Microsoft 365 (Preview)**.
2. Paste your **client ID** and **client secret**. The secret is encrypted into
   `client_secret_encrypted` (AES-256-GCM) — it is never stored in cleartext and
   never leaves the server.
3. Set the `redirect_uri` to match the app registration, then **enable** the
   integration.
4. **Connect an account** — this runs the OAuth flow and stores the account under
   `connected_accounts` with encrypted tokens.

## Mounting

Create a `raisin:VirtualMount` (admin console → **Mounts**) pointing at this
integration.

Mail:

```yaml
node_type: raisin:VirtualMount
properties:
  title: Outlook Inbox
  integration_ref: /integrations/ms-graph
  account_ref: "<connected_accounts[].id>"
  target_workspace: default
  mount_path: /mail/inbox
  remote_root: inbox            # mail folder id, or a well-known name (inbox, ...)
  sync_config:
    resource: mail
    mode: poll
    interval_seconds: 300
  enabled: true
```

Calendar:

```yaml
node_type: raisin:VirtualMount
properties:
  title: My Calendar
  integration_ref: /integrations/ms-graph
  account_ref: "<connected_accounts[].id>"
  target_workspace: default
  mount_path: /calendar
  remote_root: calendar         # calendar id (set a real id for non-default calendars)
  sync_config:
    resource: calendar
    window:
      days_back: 7
      days_ahead: 30
    mode: poll
    interval_seconds: 300
  enabled: true
```

## Delta / token flow

`get_changes` drives incremental sync. On the first call (`since_token` null) the
adapter builds the initial `.../messages/delta` (or `.../calendarView/delta`) URL.
Graph returns a page plus either `@odata.nextLink` (more pages) or
`@odata.deltaLink` (end of feed). The adapter returns that link as `next_token`;
the engine persists it and passes it back verbatim on the next call. `next_token`
is **never null** — when nothing is new the `deltaLink` round-trips unchanged.

Each page also carries **`has_more`**: `true` for a `nextLink` (a
mid-enumeration cursor — keep paging now), `false` for a `deltaLink` (caught up
— the token is the *next run's* resume point). The engine cannot infer this
from the token itself, because Graph mints a **fresh delta token on every poll
of an idle feed**: before `has_more` existed the delta loop had only "the token
stopped changing" to stop on, so against an idle calendar it span empty pages at
request speed — committing the fresh cursor each round — until the job watchdog
killed the run every ten minutes.

## Immutable ids (mail and calendar)

Requests against Outlook resources carry `Prefer: IdType="ImmutableId"`, and the
mount keys each node on the id Graph returns.

Graph's **default** message id is derived from the item's location, so moving a
message between folders **changes its id**. To a virtual mount — which keys a
node on `external_id` for the node's whole lifetime — that reads as a *delete of
the old id plus a create of a new one*: the node is destroyed and rebuilt,
taking its attachment subnodes, its history and any local annotation with it,
and any pending writeback against the old id 404s. Filing an email is the single
most ordinary thing a person does to a mailbox, so this is on by default.

**Migration.** Immutable ids are a *different id space*. A mount that already
synced with default ids will not recognise its own items: the next full
reconcile imports every message afresh under its immutable id and prunes the old
nodes. That is a one-time re-import — node ids change, per-node history
restarts, attachments are re-fetched — and on a mount with writeback enabled the
re-imported nodes are unseeded, so the first drain pushes their current values
back once (bounded by `max_items_per_sync` per run, and idempotent). **Nothing
is lost at the provider.**

Two ways to manage it:

- **Take the re-import** (recommended, and cheapest while a mount is small).
- **Defer**: set `immutable_ids: false` on the mount (or the connection) to keep
  the old id space. New mounts should always run with it on.

If a *folder-scoped* mail mount reports `misconfigured` right after the switch,
re-pick its folder in the mount editor: `remote_root` holds a folder id captured
in the old id space.

## Security notes

- Refresh tokens never enter the function sandbox. The adapter receives only a
  short-lived `access_token`; on `401`/`403` it throws `auth_expired` and the
  engine refreshes or pauses the mount. `429` throws `rate_limited`.
- Outbound HTTP is restricted by the adapter's `network_policy` to
  `graph.microsoft.com` and `login.microsoftonline.com`.
- The integration template ships `enabled: false` and contains **no** client
  secret.
