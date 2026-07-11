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

## Mail vs calendar

The mount's `sync_config.resource` selects the surface (default `"mail"`):

| `resource` | List | Delta (`get_changes`) | `remote_root` |
|------------|------|------------------------|---------------|
| `"mail"` (default) | `/me/mailFolders/{id}/messages` | `/me/mailFolders/{id}/messages/delta` | mail folder id, default `inbox` |
| `"calendar"` | `/me/calendars/{calId}/events` | `/me/calendars/{calId}/calendarView/delta` bounded by `sync_config.window` | calendar id, default `calendar` |

For calendars, `sync_config.window` bounds the `calendarView` delta:
`{ days_back: 7, days_ahead: 30 }` by default.

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

## Security notes

- Refresh tokens never enter the function sandbox. The adapter receives only a
  short-lived `access_token`; on `401`/`403` it throws `auth_expired` and the
  engine refreshes or pauses the mount. `429` throws `rate_limited`.
- Outbound HTTP is restricted by the adapter's `network_policy` to
  `graph.microsoft.com` and `login.microsoftonline.com`.
- The integration template ships `enabled: false` and contains **no** client
  secret.
