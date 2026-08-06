# Google Calendar Adapter (Experimental)

> **Experimental / Preview.** This connector is shipped as a preview feature.
> Validate it against your own Google account before relying on it in
> production. Behaviour and defaults may change between releases.

Mount a Google Calendar into a RaisinDB workspace path. The sync engine polls the
calendar, maps each event to a `raisin:Event` node, and keeps the subtree in
sync. Events are leaves — there is no folder hierarchy.

This package implements the frozen adapter contract in
`docs/reference/virtual-node-adapters.md` over the Google Calendar **v3** REST
API. It reads (`can_read`, `supports_changes`, `supports_push`) and it **writes**
a full mirror (`can_create`, `can_update`, `can_delete`).

## What it ships

| Path | Workspace | Purpose |
|------|-----------|---------|
| `/adapters/google-calendar` | `functions` | Calendar v3 adapter function (`handler(input)`). |
| `/mappers/google-calendar-default` | `functions` | Default per-event mapping function. |
| `/integrations/google-calendar` | `raisin:system` | Pre-configured `raisin:Integration` template, **disabled**. |

### Capabilities

`can_read`, `supports_changes` and `supports_push` are `true`, and so are
`can_create` / `can_update` / `can_delete`. Folder creation and search are not
implemented, and neither is `submit`: an RSVP through Google is a PATCH of the
caller's own attendee row rather than a distinct action endpoint.

Two provider facts shape the write path, and both are surprising enough to state
before you enable it:

- **Google has no trash for events.** A delete is immediate and unrecoverable, so
  the adapter declares `supports_trash: false` and defaults `delete_policy` to
  `detach` — a local delete does **not** reach Google until an operator sets the
  mount's `delete_policy` to `purge`. A mount configured for `trash` is refused
  at resolution rather than silently promoted to a purge.
- **Google mails every attendee** when an event with attendees is created, moved
  or deleted. That is irreversible and externally visible, so every write sends
  `sendUpdates=none`. A mount that wants invitations sets
  `sync_config.send_updates` to `externalOnly` or `all` — deliberately opt-in,
  because a sync engine mirroring a node is not a person deciding to notify
  twelve people.

## Window + syncToken flow

The adapter has two complementary read paths:

- **Full / list** — `events.list` bounded by a rolling time window:
  `timeMin = now - window.days_back`, `timeMax = now + window.days_ahead`
  (defaults **7** days back, **90** days ahead), with `singleEvents=true` and
  `orderBy=startTime`. Recurring events are expanded into individual instances.
  Configure the window under the mount's `sync_config`:

  ```yaml
  sync_config:
    window:
      days_ahead: 90
      days_back: 7
  ```

- **Incremental (`get_changes`)** — uses Google's opaque **syncToken**:
  - First call (no `since_token`): the engine has already run a full reconcile,
    so the adapter pages a windowed list to the end purely to harvest a
    `nextSyncToken`, returns **no** changes, and hands that token back.
  - Subsequent calls pass `syncToken=since_token` (no `timeMin`/`timeMax`/
    `orderBy` — those invalidate a syncToken). `next_token` is the response's
    `nextSyncToken`, and is **never null**: it falls back to the page token
    while paging, and otherwise echoes the caller's token so the cursor holds.
  - On HTTP **410 GONE** the syncToken has expired; the adapter throws a
    **transient** error. There is no dedicated resync opcode — the engine drops
    the token and re-runs a full reconcile (`mode: "full"`), which is the
    accepted recovery path.

Cancelled events (`status: "cancelled"`) surface as `deleted` changes and the
default mapper returns `null` for them.

## Google Cloud setup

1. In the [Google Cloud Console](https://console.cloud.google.com/) create (or
   select) a project and **enable the Google Calendar API**.
2. Configure the OAuth consent screen (internal or external) and add the scope
   below.
3. Create an **OAuth 2.0 Client ID** of type *Web application*.
4. Add your RaisinDB callback as an authorized redirect URI, e.g.
   `https://<your-host>/api/integrations/oauth/callback`.
5. Copy the **Client ID** and **Client secret**.

### OAuth scope

The template requests a single least-privilege scope:

- `https://www.googleapis.com/auth/calendar.readonly` — read calendars and their
  events.

**Writing needs more, and the template deliberately does not ask for it.** A
mirror mount needs `https://www.googleapis.com/auth/calendar.events`; with only
the read scope every write returns 403, which the adapter reports as a
`config_error` naming this scope rather than as an expired token. Widening it is
three steps and none can be skipped: add the scope to the **live**
`raisin:Integration` node under `/integrations` (the `/connectors` template is
package-owned and is overwritten on update), enable it on the Google Cloud
consent screen, and **reconnect each account** — Google only issues a widened
scope on fresh consent, never on refresh.

The template sets `access_type: offline` + `prompt: consent` so Google issues a
refresh token; the engine stores it encrypted and never passes it to the adapter.

## Connecting the integration

1. Open the admin console → **Connectors** → **Google Calendar**.
2. Paste your **Client ID** and **Client secret**. The secret is encrypted into
   `client_secret_encrypted` (AES-256-GCM) — it is never stored in cleartext and
   never leaves the server.
3. Set the `redirect_uri` to match step 4 above, then **enable** the integration.
4. **Connect an account** — this runs the OAuth flow and stores the account under
   `connected_accounts` with encrypted tokens.

## Mounting a calendar

Create a `raisin:VirtualMount` (admin console → **Mounts**, or a node under
`raisin:system/mounts`) pointing at this integration:

```yaml
node_type: raisin:VirtualMount
properties:
  title: Team Calendar
  integration_ref: /integrations/google-calendar
  account_ref: "<connected_accounts[].id>"
  target_workspace: default
  mount_path: /calendars/team
  remote_root: "primary"     # calendar id, or "primary" for the account's default
  # mapping_function: /mappers/google-calendar-default   # optional; omit to use it
  sync_config:
    mode: poll
    interval_seconds: 300
    max_items_per_sync: 500
    window:
      days_ahead: 90
      days_back: 7
    ephemeral: false
  enabled: true
```

The engine runs a full reconcile on first sync, then incremental `get_changes`
deltas. Writes run under the `virtual-mount-sync` system actor.

## Security notes

- Refresh tokens never enter the function sandbox. The adapter receives only a
  short-lived `access_token`; on `401`/`403` it throws `auth_expired` and the
  engine refreshes or pauses the mount.
- Outbound HTTP is restricted by the adapter's `network_policy` to
  `www.googleapis.com` and `oauth2.googleapis.com`.
- The integration template ships `enabled: false` and contains **no** client
  secret.
