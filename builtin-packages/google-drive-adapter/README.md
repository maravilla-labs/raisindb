# Google Drive Adapter

Mount a Google Drive folder into a RaisinDB workspace path. The sync engine polls
Drive, maps each item to a node, and keeps the subtree in sync — files become
`raisin:Asset` nodes (link-only in v1), folders become `raisin:Folder` nodes.

This package implements the frozen adapter contract in
`docs/reference/virtual-node-adapters.md` over the Google Drive **v3** REST API.

## What it ships

| Path | Workspace | Purpose |
|------|-----------|---------|
| `/adapters/google-drive` | `functions` | Drive v3 adapter function (`handler(input)`). |
| `/mappers/google-drive-default` | `functions` | Default per-item mapping function. |
| `/integrations/google-drive` | `raisin:system` | Pre-configured `raisin:Integration` template, **disabled**. |

### Capabilities

`can_read`, `can_write`, `can_create_folders`, and `supports_changes` (real Drive
changes API) are all `true`. Webhooks, search, and push are not implemented in v1.

### v1 scope — links only

Synced `raisin:Asset` nodes carry `web_url` (Drive `webViewLink`) and
`download_url` (Drive `webContentLink`) but **no inlined binary content**. The
adapter's `get_content` operation is fully implemented (binary via `alt=media`,
Google-native docs via `export`) for opt-in content sync, but the shipped mount
defaults keep content sync off. This avoids downloading large binaries into node
properties during ordinary sync.

## Google Cloud setup

1. In the [Google Cloud Console](https://console.cloud.google.com/) create (or
   select) a project and **enable the Google Drive API**.
2. Configure the OAuth consent screen (internal or external) and add the scopes
   below.
3. Create an **OAuth 2.0 Client ID** of type *Web application*.
4. Add your RaisinDB callback as an authorized redirect URI, e.g.
   `https://<your-host>/api/integrations/oauth/callback`.
5. Copy the **Client ID** and **Client secret**.

### OAuth scopes

The template requests least-privilege scopes:

- `https://www.googleapis.com/auth/drive.readonly` — read files and content.
- `https://www.googleapis.com/auth/drive.file` — create/update files this app owns.
- `https://www.googleapis.com/auth/drive.metadata.readonly` — read metadata.

To write through to files **not** created by this integration, widen the scope to
`https://www.googleapis.com/auth/drive` in the integration's `oauth_config`.

The template sets `access_type: offline` + `prompt: consent` so Google issues a
refresh token; the engine stores it encrypted and never passes it to the adapter.

## Connecting the integration

1. Open the admin console → **Integrations** → **Google Drive**.
2. Paste your **Client ID** and **Client secret**. The secret is encrypted into
   `client_secret_encrypted` (AES-256-GCM) — it is never stored in cleartext and
   never leaves the server.
3. Set the `redirect_uri` to match step 4 above, then **enable** the integration.
4. **Connect an account** — this runs the OAuth flow and stores the account under
   `connected_accounts` with encrypted tokens.

## Mounting a Drive folder

Create a `raisin:VirtualMount` (admin console → **Mounts**, or a node under
`raisin:system/mounts`) pointing at this integration:

```yaml
node_type: raisin:VirtualMount
properties:
  title: Shared Drive Docs
  integration_ref: /integrations/google-drive
  account_ref: "<connected_accounts[].id>"
  target_workspace: default
  mount_path: /documents/shared
  remote_root: "<Google folder id>"     # from the folder URL: .../folders/<id>
  # mapping_function: /mappers/google-drive-default   # optional; the engine uses
  #   its built-in Folder/Asset mapping if omitted
  sync_config:
    mode: poll
    interval_seconds: 300
    max_items_per_sync: 500
    ephemeral: false
  enabled: true
```

The engine runs a full reconcile on first sync, then incremental `get_changes`
deltas. Writes run under the `virtual-mount-sync` system actor.

## Security notes

- Refresh tokens never enter the function sandbox. The adapter receives only a
  short-lived `access_token`; on `401` it throws `auth_expired` and the engine
  refreshes or pauses the mount.
- Outbound HTTP is restricted by the adapter's `network_policy` to
  `www.googleapis.com` and `oauth2.googleapis.com`.
- The integration template ships `enabled: false` and contains **no** client
  secret.
