# Google Calendar Adapter (Experimental)

> **Experimental / Preview.** This connector is shipped as a preview feature.
> Validate it against your own Google account before relying on it in
> production. Behaviour and defaults may change between releases.

Mount a Google Calendar into a RaisinDB workspace path. The sync engine polls the
calendar, maps each event to a `raisin:Event` node, and keeps the subtree in
sync. Events are leaves — there is no folder hierarchy.

This package implements the frozen adapter contract in
`docs/reference/virtual-node-adapters.md` over the Google Calendar **v3** REST
API. It reads (`can_read`, `supports_changes`, `supports_push`), it **writes** a
full mirror (`can_create`, `can_update`, `can_delete`), it issues **one command**
(`can_submit` — an RSVP), and it offers **discovery** (`supports_browse`).

## What it ships

| Path | Workspace | Purpose |
|------|-----------|---------|
| `/adapters/google-calendar` | `functions` | Calendar v3 adapter function (`handler(input)`). |
| `/mappers/google-calendar-default` | `functions` | Default per-event mapping function (both directions). |
| `/mappers/google-calendar-outbox` | `functions` | Outbox mapper: `raisin:CalendarAction` -> RSVP command. |
| `/integrations/google-calendar` | `raisin:system` | Pre-configured `raisin:Integration` template, **disabled**. |

### Capabilities

`can_read`, `supports_changes`, `supports_push` and `supports_browse` are `true`,
and so are `can_create` / `can_update` / `can_delete` / `can_submit`. Folder
creation and search are not implemented.

`supports_browse` means the mount editor lists the account's calendars
(`calendarList.list`) instead of asking for a hand-typed calendar id. It needs no
scope beyond `calendar.readonly`.

`can_submit` is an **RSVP** against a `raisin:CalendarAction` node on a separate
`submit` mount (conventionally `/calendar/rsvp`), paired with the
**google-calendar-outbox** mapper. It is a command rather than a property on the
event because answering an invitation notifies the organizer: irreversible,
externally visible, and not something a bulk property edit may reach.

Google has no accept/decline endpoint — an RSVP is `events.patch` of the caller's
own attendee row — and `events.patch` documents that *"array fields, if
specified, overwrite the existing arrays"*. Sending only your own row would
therefore **delete every other guest from the meeting**. The adapter reads the
event, mutates the row marked `self` in place, and PATCHes the whole array back;
that is why an RSVP costs two round trips where the Graph one costs one. It sends
`sendUpdates=all` by default (override per command with
`raisin:CalendarAction.send_response: false`) because for Google, telling the
organizer *is* the RSVP — `sendUpdates=none` records the response and tells
nobody.

Two provider facts shape the mirror write path, and both are surprising enough to
state before you enable it:

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
  (defaults **7** days back, **90** days ahead), with `showDeleted=true` and
  **without** `singleEvents`. Google therefore returns the underlying records —
  single events, recurring **masters** carrying their RRULE, and modified or
  cancelled instances as their own records. Unmodified occurrences are not
  returned and are not wanted: they are projected from the master by the
  calendar expander. Configure the window under the mount's `sync_config`:

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
  - Subsequent calls pass `syncToken=since_token` and drop `timeMin`/`timeMax`/
    `orderBy` — the only parameters Google exempts. **Every other parameter must
    match the request that minted the token**, `showDeleted` included; a mismatch
    either silently drops deletions from the delta or is rejected with a 400.
    `next_token` is the response's `nextSyncToken`, and is **never null**: it
    falls back to the page token while paging, and otherwise echoes the caller's
    token so the cursor holds.
  - On HTTP **410 GONE** (and on the one-time **400** after a parameter change)
    the token is unusable; the adapter reports **`cursor_invalid`**, which makes
    the engine drop the stored token and full-reconcile **in the same run**. It
    is deliberately not a transient error — that had the engine retrying the same
    rejected token every tick forever.

A cancelled record is **not** uniformly a delete. One carrying a
`recurringEventId` is a cancelled **occurrence** of a series and is materialized
like any other exception, with `status: "cancelled"` — it is the only evidence
that the meeting does not happen, and the expander suppresses a projected
occurrence solely on the existence of an exception node at that slot. Only a
cancelled record **without** a `recurringEventId` is reported as `deleted`. The
default mapper materializes cancelled events rather than returning `null`, so
both providers agree.

## Google Cloud setup

1. In the [Google Cloud Console](https://console.cloud.google.com/) create (or
   select) a project and **enable the Google Calendar API**.
2. Configure the OAuth consent screen (internal or external) and add the scope
   below.
3. Create an **OAuth 2.0 Client ID** of type *Web application*.
4. Add your RaisinDB callback as an authorized redirect URI, e.g.
   `https://<your-host>/api/integrations/oauth/callback`.
5. Copy the **Client ID** and **Client secret**.

### OAuth scopes

The template requests **two**:

- `https://www.googleapis.com/auth/calendar.readonly` — read calendars and their
  events, and list them in the mount editor (`browse`).
- `https://www.googleapis.com/auth/calendar.events` — create, edit and delete
  events, **and** send an RSVP.

Grant only the first and you get a read-only mount: every write and every RSVP
returns 403, which the adapter reports as a `config_error` naming this scope
rather than as an expired token.

**A scope added after an account was connected does not take effect on its own.**
Google issues a widened scope on fresh consent only, never on refresh, so the
sequence is: make sure the scope is on the **live** `raisin:Integration` node
under `/integrations` (the `/connectors` template is package-owned and is
overwritten on update), enable it on the Google Cloud consent screen, and
**reconnect each account**.

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
  remote_root: "primary"     # calendar id (pick it from the browse list), or
                             # "primary" for the account's default
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

### An RSVP mount

RSVPs live in their own mount, beside the calendar one, so that answering an
invitation is never reachable by editing an event:

```yaml
node_type: raisin:VirtualMount
properties:
  title: Team Calendar RSVPs
  integration_ref: /integrations/google-calendar
  account_ref: "<connected_accounts[].id>"
  target_workspace: default
  mount_path: /calendars/team/rsvp
  remote_root: "primary"     # the SAME calendar the RSVP is answered from
  mapping_function: /mappers/google-calendar-outbox
  write_config:
    mode: submit
  enabled: true
```

It holds `raisin:CalendarAction` nodes (`action`, `target_external_id`,
optional `comment` and `send_response`). Nothing is sent until a node's `status`
is moved to `queued`; an ambiguous outcome parks at `unknown` and is never
auto-retried, because a retry means a second notification to the organizer.

## Security notes

- Refresh tokens never enter the function sandbox. The adapter receives only a
  short-lived `access_token`; on `401`/`403` it throws `auth_expired` and the
  engine refreshes or pauses the mount.
- Outbound HTTP is restricted by the adapter's `network_policy` to
  `www.googleapis.com` and `oauth2.googleapis.com`.
- The integration template ships `enabled: false` and contains **no** client
  secret.
