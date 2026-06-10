# Shiftboard package flow - gap report

What the RaisinDB package system could NOT express when converting the
imperative `setup.mjs` bootstrap into the installable `package/` (deployed via
`raisindb package deploy ./package --install`, see `ci.sh`). Everything listed
under "out-of-band" had to stay as scripted operator steps. This feeds product
work on the package system + CLI.

## Out-of-band steps a package cannot do

1. **Repository creation.** A package installs *into* a repo; it cannot create
   one. CI must `POST /api/repositories` first (admin/system token required),
   then wait for builtin packages (messaging, ai-tools) to auto-install before
   the app package can be installed (its agent depends on the agent-handler).

2. **Identity users.** Demo login `planner@example.com` must be registered via
   `POST /auth/{repo}/register`. There is no `content/` equivalent for the
   identity store (and identity users appear to be instance-shared - the user
   "already existed" on a brand-new repo). Packages that need a demo/login user
   always need a post-install step.

3. **Tenant AI provider config / secrets.** The Groq API key lives at tenant
   level (`/api/tenants/{tenant}/ai/config`). Secrets clearly don't belong in a
   package, but a package also has no way to *declare* "requires provider groq
   with an API key" so install could fail fast with a clear message. `ci.sh`
   checks `has_api_key`/`enabled` manually (without printing the key).

4. **CORS / server-level config.** Allowed frontend origins are server/tenant
   config, not packageable. Irrelevant for the headless smoke test but a real
   step when pointing the SPA at a remote instance.

## Package-system expressiveness gaps

5. **No enforced dependency on other packages.** The agent node is
   `raisin:AIAgent` and references the builtin weather tool
   `/lib/raisin/ai/weather` - both provided by the builtin `ai-tools` package.
   The manifest has no (validated) `dependencies`/`requires` mechanism, so the
   validator emits `UNKNOWN_NODE_TYPE_REFERENCE` for `raisin:AIAgent` (warning
   only) and nothing checks at install time that ai-tools is present/recent
   enough.

6. **No parametrization.** `setup.mjs` honored `SHIFTBOARD_MODEL`; the package
   hardcodes `llama-3.3-70b-versatile` in the agent YAML. No install-time
   variables/templating (model, provider, titles).

7. **Function code lands as a file asset, not `code`.** The installer stores
   sibling `index.js` as a `raisin:Asset` with a binary `file` resource (no
   `code` property), while `setup.mjs` wrote a `code` property. The runtime
   handles both, but the two bootstrap paths produce structurally different
   asset nodes - worth unifying.

## CLI ergonomics (worked around in this change)

8. **Non-interactive auth did not exist** - added: `RAISINDB_SERVER` /
   `RAISINDB_TOKEN` / `RAISINDB_REPO` env overrides (env wins over `.raisinrc`)
   and `raisindb login --server --username --password [--tenant]` / `--token`
   for stored credentials.

9. **`deploy` only uploaded** - added `--install` so
   `raisindb package deploy ./package -r <repo> --install` is one shot.
   (Upload and install remain separate server endpoints.)

10. **No "wait until installed" verb.** Install returns quickly but content
    materializes asynchronously; `ci.sh` polls the packages endpoint plus
    representative node paths (`/lib/shiftboard/assign-shift`, a seed shift,
    the agent). A `raisindb package status --wait` would remove ~30 lines of
    polling from every CI script.

11. **Ink progress UI in CI logs.** The animated upload/validate components
    write cursor-control escape sequences to non-TTY output, making CI logs
    noisy. A `--plain`/non-TTY detection mode would help.

## Workflow variant (tutorial part 5) - additional findings

12. **`raisin:MessageFolder` rejected inbox tasks (engine).** The workflow
    engine creates human tasks at `{assignee}/inbox/task-*` in
    `raisin:access_control`, but a registered user's `inbox` folder is a
    `raisin:MessageFolder` whose builtin `allowed_children` only listed
    `raisin:Message`/`raisin:Conversation`. Task *creation* skips the
    parent-child check (the flow callbacks create with
    `validate_parent_allows_child: false`), but task *completion* updates the
    node through the validated path and failed with "not allowed as a child
    of 'raisin:MessageFolder'" - identity users could never answer a
    workflow task. Fixed upstream in
    `crates/raisin-core/global_nodetypes/raisin_message_folder.yaml`
    (allowed_children += `raisin:InboxTask`); `setup.mjs` additionally
    patches the node type per repo via
    `PUT /api/management/{repo}/{branch}/nodetypes/raisin:MessageFolder`
    so repos created by older builds work too. A package cannot express
    "patch this node type" - nodetype patching would be the package-system
    equivalent of `workspace_patches`.

13. **`raisin.flows.*` did not exist in the function runtime.** Functions
    could call other functions (`raisin.functions.call`) but not start a
    `raisin:Flow`. Added `raisin.flows.run(flowPath, input)` (fire and
    forget, returns `{ instance_id, job_id, status }`): callback type +
    builder field (`api/callbacks/`), `FunctionApi::flow_run` trait method,
    `flows_run` binding (`runtime/bindings/methods/flows.rs`), production
    callback wiring `raisin_flow_runtime::service::run_flow` through the
    unified job queue (`execution/callbacks/flows.rs`). Requires a server
    rebuild to be live.

14. **The QuickJS `raisin` API has TWO wrapper sources.** New bindings
    registered in `runtime/bindings/methods/` are auto-exposed by the
    *generated* wrapper (`wrappers/javascript.rs`), but the QuickJS
    environment actually evals the *hand-written static*
    `runtime/quickjs/api_wrapper.js` - a new category silently does not
    exist (`raisin.flows` was `undefined` even though the
    `__raisin_internal.flows_run` binding was registered). Both files must
    be updated; unifying them on the generated wrapper would remove the
    trap.

15. **`RaisinHttpClient` drops auth after admin login (SDK).** Admin
    authentication stores only the access token (`storage.setAccessToken`)
    and never sets the expiry, so `AuthManager.isAuthenticated()` stays
    false and `request()` silently sends NO Authorization header -
    `executeSql` etc. run as anonymous and get RLS-filtered empty results.
    `FlowClient`/`InboxApi` read the token directly and are unaffected.
    `workflow-test.mjs` works around it with a plain-fetch SQL helper (the
    event-ticketing example independently grew the same workaround).
