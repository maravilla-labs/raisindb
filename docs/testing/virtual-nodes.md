# Manual e2e — Virtual Nodes (Google Drive)

End-to-end smoke test for the virtual-nodes feature against a **real Google account**. This
exercises the full chain: connect account → create mount → files appear as nodes → edit in
Drive → delta sync updates the node → a user-defined trigger fires.

Steps flagged **[REAL CREDS]** require live Google OAuth credentials and a real Google Drive
account; the rest run against a local RaisinDB instance only. Automated coverage lives in the
integration/cluster tests (see the implementation plan §8); this script is the human
walkthrough that proves the real provider path.

---

## Prerequisites

- A local release server built with the relevant features (see repo `CLAUDE.md` build
  command) and pgwire enabled.
- `RAISIN_MASTER_KEY` set in the server environment (32-byte key). Integrations cannot store
  or decrypt account tokens without it. **[REAL CREDS]** — same key used for AI provider keys.
- The `google-drive-adapter` builtin package installed (ships on bootstrap; verify it appears
  in the admin console Packages page or via the packages API).
- **[REAL CREDS]** A Google Cloud OAuth client (client_id + client_secret) with the Drive
  scope enabled and the server's OAuth callback URL registered as an authorized redirect URI.
- **[REAL CREDS]** A Google account with a Drive folder you can edit, containing a few test
  files and at least one subfolder.
- Admin console running against the local server; you are operating in a repo (dev mode uses
  tenant `default`).

---

## 1. Create the integration

1. In the admin console, open **`/:repo/integrations`** and create a new integration:
   - provider_type: `google-drive`
   - title: e.g. `Google Drive (test)`
   - adapter_function: the `google-drive-adapter` path (prefilled by the package)
   - oauth_config: paste `client_id`, scopes, auth/token URLs, redirect URI. **[REAL CREDS]**
   - client secret: paste the Google `client_secret`. **[REAL CREDS]** It is stored only as
     `client_secret_encrypted` (AES-256-GCM); confirm the raw secret is **not** readable back
     in the node property / API response.
2. Confirm a `raisin:Integration` node now exists at `/integrations/{name}` in the
   `raisin:system` workspace.

**Expected:** integration node created; secret stored encrypted; `enabled = true`.

---

## 2. Connect a Google account (outbound OAuth)  **[REAL CREDS]**

1. From the integration page, click **Connect account**. The browser redirects to Google's
   consent screen.
2. Sign in and grant the Drive scope. Google redirects back to the server's OAuth callback.
3. Back in the console, confirm a new entry appears in the integration's
   `connected_accounts` with a `subject` (the account email) and an `expires_at`.

**Expected:** one connected account; `subject` and `expires_at` are plaintext; the tokens are
stored as `tokens_encrypted` only. **Verify `refresh_token` never appears** in any node
property, API response, or log line.

---

## 3. Create the mount

1. Open **`/:repo/mounts`** and create a `raisin:VirtualMount`:
   - integration_ref: the integration node path from step 1
   - account_ref: the `connected_accounts[].id` from step 2
   - target_workspace: a real content workspace (e.g. `default`)
   - mount_path: e.g. `/documents/shared`
   - remote_root: the Drive folder id to mount **[REAL CREDS]**
   - sync_config: `mode: "poll"`, `interval_seconds: 300` (lower, e.g. 60, to speed the test)
   - leave `mapping_function` unset to use the built-in default mapping
2. Save; confirm the mount node exists at `/mounts/{name}` with `enabled = true`.

**Expected:** mount node created; `state` initially empty (no `last_sync_at`).

---

## 4. Initial full sync — files appear as nodes

1. Trigger the first sync — either wait for the scheduler's `VirtualMountSyncCheck` tick to
   enqueue it, or use the manual "Sync now" action on the mount page.
2. Watch the mount's `state`: `status` becomes healthy, `last_sync_at` populated,
   `last_sync_token` set.
3. Browse `target_workspace` at `mount_path`. **[REAL CREDS]** The Drive folder's files and
   subfolders should now exist as nodes:
   - folders → `raisin:Folder`
   - files → `raisin:Asset` with `title` / `mimeType` / `size`
4. Inspect one file node's properties and confirm the reserved virtual metadata:
   `__virtual = true`, `__mount_id`, `__external_id`, `__etag`, `__synced_at`.
5. SQL check:

   ```sql
   SELECT path, properties->>'title'::String
   FROM 'default'
   WHERE properties->>'__mount_id'::String = '<mount node id>'
   ```

**Expected:** node count matches the Drive folder contents; hierarchy mirrors Drive; reserved
properties present; the query returns exactly the mounted items.

---

## 5. Edit in Drive → delta sync updates the node  **[REAL CREDS]**

1. In the Google Drive web UI, **rename** one test file and **edit the contents** of another.
   Optionally add a new file and delete an existing one.
2. Trigger the next sync (wait for the interval, or "Sync now").
3. Re-inspect the affected nodes in the console:
   - the renamed file's node keeps the **same node** (matched by `__external_id`) with an
     updated `title` and a bumped `__etag` / `__synced_at` — a rename must **not** create a
     duplicate node.
   - the edited file's `__etag` / `__synced_at` change; an unchanged file's node is **not**
     rewritten (etag skip-write — its revision count does not increase).
   - a new Drive file appears as a new node; a deleted Drive file's node is removed.
4. Confirm the deletion only removed the mount-owned node, and any non-virtual node you
   manually created under `mount_path` is untouched.

**Expected:** delta sync reflects rename/edit/add/delete correctly; no duplicates on rename;
unchanged files cause no revision churn; user-created nodes under the mount survive.

---

## 6. Trigger fires on a synced node

1. Before or after step 5, install a user-defined `raisin:Trigger` on `node_event` in
   `target_workspace`, scoped (path filter) to `mount_path`, that records something
   observable (log line, writes a marker node, sends a message).
2. Perform a Drive edit (step 5) and run the sync so the synced write emits a `node_event`.
3. Confirm the trigger's function ran as a result of the sync-materialized write.

**Expected:** the sync-materialized node write fires the user's `node_event` trigger. Note the
sync actor is `"virtual-mount-sync"`; if you use writeback later, its trigger filter must
exclude that actor to avoid loops.

---

## 7. Failure & re-auth spot checks (optional)

- **auth_expired:** revoke the app's access from the Google account security settings, then
  run a sync. **[REAL CREDS]** The mount `state.status` should become `auth_required` and the
  mount should be skipped by future checks until reconnected. Reconnect (repeat step 2) and
  confirm sync resumes.
- **degraded backoff:** point `remote_root` at an invalid folder id and sync repeatedly;
  after the failure threshold (default 5) `state.status` should be `degraded` and the effective
  interval should back off. Fix the id and confirm recovery resets the counters.

---

## Teardown

1. Delete the mount node — confirm its virtual nodes are cleaned up and non-virtual nodes
   under `mount_path` remain.
2. Disconnect the account and/or delete the integration node.
3. Revoke the OAuth app grant from the Google account if this was a throwaway client.
   **[REAL CREDS]**
