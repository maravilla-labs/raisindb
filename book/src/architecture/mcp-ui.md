# Native MCP-UI Support

MCP-UI lets a tool call return not just JSON but an HTML "mini-app" that the MCP host (an AI client such as a desktop assistant, ChatGPT, or Goose) renders inline. The tool runs server-side as the real signed-in user, returns structured `data` plus a reference to a UI resource, the host fetches the widget once (and caches it), and delivers the `data` to it. The widget itself carries no cookies and no auth -- it is a dumb renderer of whatever the tool handed it.

This chapter documents how that capability maps onto RaisinDB's existing content model, and -- more importantly for a maintainer -- **the decision points and why each went the way it did.** The short version: RaisinDB already had almost everything MCP-UI needs, so the design reuses the content model, the asset tree, RLS, and the MCP engine rather than bolting on a parallel system. The one genuinely new server-side primitive (a static-content endpoint) turns out to be useful on its own.

Read [MCP Servers as Content](./mcp.md) first; this chapter assumes the server descriptor, the tool/function binding, the `structuredContent` channel, and the `resources/read` limitation described there.

## The mapping onto RaisinDB's content model

Nothing here introduces a new storage concept. Every moving part is an existing one:

| MCP-UI concept | RaisinDB primitive it reuses |
|----------------|------------------------------|
| The widget (HTML/CSS/JS bundle) | Ordinary `raisin:Asset` nodes in a `raisin:Folder` tree -- uploaded through the existing multipart/chunked upload pipeline |
| Serving the widget's files to a browser | A new static-content HTTP endpoint over `NodeService` (RLS-enforced) |
| Which widget a tool renders | A `ui: { mode, entry }` field on the tool -- a **path reference**, not a new node type |
| The widget's initial data | The tool's existing `structuredContent` (see [MCP Servers as Content](./mcp.md)) |
| A widget button re-invoking the server | An ordinary `tools/call` on the same MCP session -- no new wire protocol |
| Serving widget bytes over the MCP channel | A new `blob` field on `ResourceContents` + an `AssetReader` service seam |
| Whether a subtree is servable at all (deny-by-default) | The *presence* of a `raisin:StaticSiteFolder` ancestor — a package-shippable content node |
| Per-subtree embedding/CORS policy | An optional `serving_config` on that same `raisin:StaticSiteFolder` subtype |

The rest of the chapter is the reasoning behind the non-obvious cells in that table.

## Decision: widgets are files, not a `raisin:McpUiWidget` node type

An earlier draft proposed a dedicated `raisin:McpUiWidget` node type. **It was dropped.** A widget is just files, and RaisinDB already has a first-class file model: `raisin:Asset` (`crates/raisin-core/global_nodetypes/raisin_asset.yaml`) carries a `file: Resource` plus `file_type`/`file_size`/`content_hash`, and `raisin:Folder` (`raisin_folder.yaml`) nests arbitrarily.

Two facts make "just use the asset tree" cheap rather than aspirational:

- **Deep folder/asset trees already work anywhere the workspace allows both types.** Type-level `allowed_children` exists on `NodeType` but is not enforced in `raisin-core`'s validation path -- every core type declares it as `[]`. What actually governs "can an Asset live under a Folder here" is the *workspace's* `allowed_node_types` / `allowed_root_node_types`. So a widget bundle is a normal upload into any workspace that permits folders and assets.
- **Path lookup is a materialized full-path string, not a recursive parent-walk.** `NodeRepository::get_by_path` (`raisin-rocksdb/.../lookup.rs`) keys on the *entire* path as one opaque string in a dedicated path index, prefix-scanned for the newest revision. `/site/widgets/order/img/logo.png` costs no more than a top-level lookup. This is why serving a relative reference (`./style.css`) from inside a widget is free -- it is just another path lookup, no segment-by-segment resolution.

So a widget becomes an ordinary `raisin:Folder` (or a lone `raisin:Asset` for a single-file widget), and the only thing a tool declares is a *path reference* to it. Dropping the node type also means no new package, no new workspace allowlist entry, and no schema migration.

## Decision: the static endpoint MUST route through NodeService (fail-closed RLS)

The new route is `GET /resources/{repo}/{branch}/{ws}/{*path}` -- a top-level route, deliberately *not* under `/api`. It resolves `{ws}/{path}` via `get_by_path`; if the node is a `raisin:Asset` it streams the `file` bytes with the stored/guessed MIME type; if it is a `raisin:Folder` (or the path is empty/trailing-slash) it serves the folder's index document (default `index.html`) instead. That index fallback is the *only* genuinely new serving logic -- everything else is existing byte-serving reachable via a cleaner, path-shaped URL.

> **Naming caution.** The HTTP `/resources/...` route serves raw file bytes to a browser and is unrelated to the MCP JSON-RPC `resources/read` method (see [MCP Servers as Content](./mcp.md)), which returns `ResourceContents` over the MCP channel. They share the word "resources" but are different transports, different callers, and different code: the browser hits `/resources/...` directly for a `mode: uri-list` widget's files, whereas the MCP engine serves widget bytes for `mode: html` through `resources/read`. Do not conflate them.

The critical constraint is **which of two existing byte-serving code paths it is built on.** They have opposite trust models, and only one is correct here:

- **The RLS-enforced path (copy this).** `handlers/repo/helpers.rs::get_property` (`helpers.rs:68`) and `handle_file_download` (`helpers.rs:173`) build on `state.node_service_for_context(...)` → `NodeService::get_property_by_path` / `get_by_path`. Every read passes through `NodeService::apply_rls_filter`, which **fails closed** -- it denies when no permission resolves. The byte fetch from `state.bin` (`helpers.rs:117`) happens only *after* `NodeService` has confirmed the caller may see that node/property.
- **The raw-storage signed-URL path (do NOT copy this).** `handlers/repo/assets.rs` serves `.../raisin:download|display?sig=&exp=` and calls `state.storage().nodes().get_by_path(...)` directly (`assets.rs:107`) -- raw repository access that bypasses `NodeService`/RLS entirely -- substituting its own HMAC signature check (`verify_asset_signature`, `assets.rs:87`). That is a deliberately different model: a short-lived, shareable capability URL, not a session. It stays, orthogonal to everything here, for expiring/shareable links.

**Why fail-closed RLS and not the signed-URL model?** The whole point of MCP-UI is that a widget shows the signed-in user their own data. If the static endpoint bypassed RLS, a folder made public by an HMAC URL would leak content the caller's role can't see, and there would be no per-user filtering at all. Routing through `NodeService` means a folder is public *precisely when the anonymous role already has read access to it* via `raisin:access_control` -- no new auth concept, no new "public flag," just the [access-control system](./access-control.md) already in place.

Concretely: the route sits behind the same `optional_auth_middleware` as `/api/repository/...`. That middleware always resolves an `AuthContext` -- either a real anonymous principal (via `PermissionService::resolve_anonymous_user`, the same way every other route does it) or an explicit deny-all when anonymous access is off for that repo/tenant -- never "absent." The node is then resolved through `node_service_for_context`, and `state.bin` is reached only for the final byte fetch after RLS has cleared it.

> This is recorded as a hard build constraint: the static endpoint must never use the raw `state.storage().nodes()` pattern from `assets.rs`. A future maintainer optimizing "just skip the service layer for a fast static path" would be silently disabling row-level security.

RLS is necessary but, as of the serving gate below, **no longer sufficient**: an earlier draft of this endpoint served *anything RLS allowed*, so any readable asset tree was reachable at `/resources`. That is no longer true — a coarse deny-by-default gate now runs **ahead of** the RLS read (next-but-one decision), and only a subtree explicitly published as a `raisin:StaticSiteFolder` is reachable at all.

## Decision: `raisin:StaticSiteFolder` as a subtype, not a mutation of `raisin:Folder`

Folder-level serving policy (embedding, CORS, caching, index document) lives on a **new subtype**, `raisin:StaticSiteFolder extends raisin:Folder`, with an optional `serving_config: Object` property:

```yaml
serving_config:
  frame_ancestors: ["https://host.example"]   # who may iframe this subtree
  cors_allowed_origins: ["https://host.example"]  # who may XHR back to RaisinDB
  cache_control: "public, max-age=3600"
  index_document: "index.html"
```

**Why a subtype and not a flag on `raisin:Folder`?** `raisin:Folder` is a core global type every repo shares. Adding a `serving_config` there would put a latent HTTP-header-injection capability on *every* folder in the system, visible or not. A subtype makes the capability an explicit, visible opt-in per subtree: you create or retype a folder as `raisin:StaticSiteFolder` to give it serving behavior, and a plain `raisin:Folder` never emits any of these headers. This mirrors the established "specialized folder" pattern already in the codebase -- `raisin:AclFolder extends raisin:Folder` (`raisin_acl_folder.yaml`) and `raisin:VirtualMount` both add engine configuration onto a folder subtype rather than onto the base.

The precedent for "config-driven HTTP response headers resolved per request from a content node" is also already here: `middleware/cors.rs` resolves `cors_allowed_origins` hierarchically (repo → tenant → global) from a `raisin:RepoAuthConfig` node (`cors.rs:274`, `cors.rs:356`). `StaticSiteFolder.serving_config` is that same idea, one level deeper: per-folder-subtree.

The static endpoint resolves the **nearest ancestor folder** for a path. If it is a plain `raisin:Folder`, no header overrides apply (the safe default). If it is a `raisin:StaticSiteFolder`, its `serving_config` drives the response headers for everything beneath it. Walking up the materialized path index (or resolving the immediate parent) is cheap for the same reason lookups are.

## Decision: `raisin:StaticSiteFolder` *presence* is a deny-by-default serving gate

The static endpoint does not serve everything RLS would allow. A path is servable through `/resources` **only when a `raisin:StaticSiteFolder` covers it** (the folder is the path itself or a `/`-boundary ancestor); if no folder in the workspace covers the path, the request is `404` (`resolve_serving_policy` → `ServingPolicy::Denied` → `NOT_FOUND` in `resolve_and_serve`, `static_site.rs`). The folder's *presence* is the allowlist — there is no `enabled`/`servable` flag and no path-glob list. This is the same content node introduced above (`serving_config` for headers); its second job is to gate serving at all.

**Why gate on `raisin:StaticSiteFolder` presence, and not on a repo-level config?** The gate has to be **package-shippable**. A `raisin:StaticSiteFolder` is ordinary content, so a package authors it at a fixed content path (`content/<encoded-ws>/<folder>/.node.yaml`) exactly like the imap-adapter package ships its `raisin:Integration` — install the package and the subtree becomes servable, no server TOML and no admin API. The repo-level CORS precedent (`raisin:RepoAuthConfig`, `cors.rs:274`) can't play this role: it lives at the **dynamic** `/config/repos/{repo_id}` path, keyed by a repo id a package can't know at authoring time, so a package could never ship one at a fixed target. Presence-of-a-content-node is the one allowlist a package *can* express.

**Why deny-by-default (and layered over RLS, not instead of it).** The gate is a **coarse** "is this subtree published as a static site?" check that runs **first**; RLS then decides "may *this caller* read *this specific node*?" when the bytes are served. The two are independent: a `StaticSiteFolder` over a subtree does not widen RLS (a caller still only sees nodes their role grants), and RLS never substitutes for the gate (a readable-but-unpublished asset tree still 404s). Failing closed means neither adding a package nor an RLS grant alone accidentally exposes a tree — publishing is a deliberate, visible act (create/retype the folder). A `StaticSiteFolder` whose path is the bare workspace root (`/`) covers the whole workspace (`covers` treats a root/empty folder path as covering everything); scope a subtree by publishing at a named path (`/site`).

**Why the gate is resolved with system auth — and why that makes the cache correct.** `resolve_serving_policy` enumerates the workspace's `raisin:StaticSiteFolder` set with `AuthContext::system()` (`NodeService::list_by_type`), so the decision is **principal-independent**: "is there a covering `StaticSiteFolder`?" has one answer for anonymous and authenticated callers alike. That is precisely what lets the result be cached in a **shared, not per-caller** cache without leaking one caller's view to another — the gate decision carries no principal-specific information. It would be a security bug to key this cache by caller *or* to let the gate stand in for RLS: RLS still runs per-request, per-principal, on the actual byte read (`node_service_for_context` with the real `AuthContext`), on top of the shared gate decision. The `ServingConfig` the gate caches is likewise principal-independent (header policy, not content).

**Why the cache is keyed by workspace, not by request path (a DoS boundary).** `AppState::static_site_cache` is keyed `{tenant}\0{repo}\0{branch}\0{ws}` and stores the *set* of `StaticSiteFolder`s in that workspace (`Arc<Vec<StaticSiteEntry>>`); the nearest-cover match (`nearest_covering`/`covers`) is then a pure in-memory scan. This is deliberate: `TtlCache` is an unbounded `DashMap` with only lazy per-key TTL checks (no capacity cap, no background eviction), so an earlier design that keyed by the full request path would let unauthenticated traffic to `/resources/{repo}/{branch}/{ws}/{unbounded-distinct-paths}` grow the map without limit — a memory-exhaustion DoS, and one this codebase has been bitten by before. A workspace-bounded key (the same shape `cors.rs` uses, and for the same reason) caps the keyspace at the number of real workspaces regardless of request-path cardinality. It also does *fewer* storage ops: one `list_by_type` per workspace per TTL instead of an ancestor walk per distinct path.

**Bounded staleness.** The gate cache uses the same ~60s TTL as the CORS resolver (`middleware/cors.rs`), so creating, deleting, or retyping a `raisin:StaticSiteFolder` takes effect within ~60s — the same bounded-staleness contract operators already accept for repo-level CORS config. Enumeration errors fail closed to `Denied` and are deliberately **not** cached, so a transient storage error can't poison an entry into a sticky 404.

## Decision: `mode: html` (srcdoc) is the safe default; `mode: uri-list` is the only path needing folder security

A tool's UI binding is `ui: { mode, entry }` (added to `CustomTool` in `server.rs` and to `tools[]` in the server's node schema). `entry` is a workspace-relative path, optionally with a `#fragment` (split on the first `#`). There are two delivery modes, matching the two ways real hosts render a widget:

- **`mode: html` (default).** The single-file case. The MCP engine reads the asset's bytes itself and returns them as a `text/html` MCP resource over `resources/read`; the host renders it via `srcdoc` in its own sandbox. **No cross-origin iframe is involved, so none of the folder security config applies.** This is the simplest, safest option and the right default for small widgets.
- **`mode: uri-list`.** The multi-file case the folder tree unlocks. The MCP resource returned is a `text/uri-list` pointing at the static endpoint's URL for that path; the host iframes it with a real `src=`. The browser then makes ordinary relative HTTP requests for CSS/JS/images back to RaisinDB. **This is the only place `StaticSiteFolder.serving_config` matters** -- `frame_ancestors` decides whether the host origin may embed the page at all, and `cors_allowed_origins` decides whether the page's JS may call back into RaisinDB.

The two modes solve the same problem with different exposure. `mode: html` never gives the widget an origin or a live connection -- it is inert HTML text in the host's sandbox, so there is nothing to configure and nothing to get wrong. `mode: uri-list` gives the widget a real origin and real network access, which is more powerful and is exactly why it (and only it) requires the security config to be set deliberately.

### `frame_ancestors` defaults to DENY

If a `StaticSiteFolder` has no `serving_config` (or none listing `frame_ancestors`), the endpoint emits **no** `Content-Security-Policy: frame-ancestors` header, and the effect is **not embeddable** by default. Cross-origin framing requires explicitly listing origins.

**Why deny-by-default?** An embeddable-by-default page is a clickjacking and data-exfiltration surface: any site could iframe a widget that renders the signed-in user's content. Requiring explicit opt-in per origin means a widget is only embeddable where its author said so. The endpoint uses CSP `frame-ancestors` (and deliberately omits `X-Frame-Options`, so CSP alone governs) because `frame-ancestors` takes an origin allowlist, whereas `X-Frame-Options` cannot express "these specific hosts."

Note that `frame-ancestors` (embeddability) and CORS (cross-origin script calls back to the API) are two different browser mechanisms solving two different problems. A real "iframe a widget hosted under RaisinDB from an external host" story needs *both* configured, which is why `serving_config` carries both.

### `cors_allowed_origins`: `"*"` and credentials are mutually exclusive

Per-folder CORS is resolved inside `serve_asset` (`static_site.rs`), **not** in `middleware/cors.rs` — deliberately, so a subtree-scoped policy can never affect any other route; the global unified CORS middleware still runs and is purely additive. The header logic makes wildcard and credentials mutually exclusive:

- An **explicitly-listed** origin ⇒ the response reflects that origin, plus `Access-Control-Allow-Credentials: true` and `Vary: Origin`. This is the credentialed path: the widget's JS may make cookie/session reads back into RaisinDB.
- `"*"` ⇒ `Access-Control-Allow-Origin: *` **without** `Access-Control-Allow-Credentials`.

**Why the split.** Browsers reject `*` + credentials outright, so the only way a wildcard is useful is credential-less (genuinely public assets). More importantly, *reflecting an arbitrary `Origin` alongside `Allow-Credentials: true` would let any site make credentialed cross-origin reads against RLS-gated content* — a confused-deputy hole. Restricting credentials to explicitly-listed origins closes it: an author must name the origin they trust with the signed-in user's session, and `"*"` degrades safely to non-credentialed. (An earlier formulation that reflected the origin with credentials regardless of whether it was `"*"` or explicit is the behavior this corrects.)

## Decision: the `#fragment` SPA-route mechanism, and its two plumbing paths

One single-file SPA build (`index.html` with a client-side hash router) can back several tools, each bound to a different in-app view, by giving each tool a different `#fragment`:

```yaml
tools:
  - name: order_card
    ui: { mode: uri-list, entry: "site/widgets/order/index.html#/order-card" }
  - name: inventory_panel
    ui: { mode: uri-list, entry: "site/widgets/order/index.html#/inventory" }
```

`entry` is parsed as `path` + optional `#fragment`. The fragment never affects *which file* to read (fragments are not part of file resolution); it only tells the SPA which route to show. The two modes need different plumbing to get that fragment to the widget, because only one produces a navigable URL:

- **`uri-list` mode -- nothing extra to build.** A URL fragment is never sent to the server (the browser strips it before the HTTP request), so the static endpoint just serves `index.html` normally. The fragment rides along on the iframe's `src=` URL, and the SPA's own hash router reads `location.hash` on mount, exactly as on a normal website. Two tools pointing at the same file with different fragments are correctly two distinct resource URIs / cache keys from the host's perspective, even though the underlying document is byte-identical.
- **`mode: html` -- the fragment travels out-of-band.** There is no navigable URL here (the host gets raw HTML text via `srcdoc`, whose location has no meaningful hash), so when `raisin-mcp` reads the asset's bytes for this mode, it strips the `#fragment` for file resolution and injects a small bootstrap global into the returned HTML before handing it to the host, right after `<head>`:

  ```html
  <script>window.__RAISIN_INITIAL_ROUTE__="/order-card";</script>
  ```

Widget authors write their router bootstrap once as `const route = window.__RAISIN_INITIAL_ROUTE__ ?? location.hash`, and it works unmodified under both modes. **This injection is the only place any server-side HTML transformation happens in the entire feature** -- everything else is pure byte passthrough. The global name `window.__RAISIN_INITIAL_ROUTE__` is kept as an internal convention (checked against common framework globals for collision).

## Decision: widget-initiated actions need NO new wire protocol

A widget with "two buttons, click one" does not need anything new on the wire between RaisinDB and the host. A button click becomes an ordinary `tools/call` against the same MCP session -- RaisinDB's existing dispatch (see [MCP Servers as Content](./mcp.md)) already handles it, running the same `raisin:Function`, RLS-scoped to the same caller, exactly like any other tool invocation. The whole round trip (button → `postMessage` → host decides → maybe re-invokes `tools/call` → result → widget re-renders) lives in the widget's JS and the host's bridge, not in RaisinDB.

Two host-side conventions exist in the wild and have **not** converged, so a widget author should not have to pick one:

- **MCP Apps** -- the official extension. JSON-RPC over `postMessage`: a widget calls `app.callServerTool({ name, arguments })`, receives results via `app.ontoolresult`, and can push structured content back into the conversation via `app.updateModelContext(...)`. Hosts *may* require explicit user approval before executing a UI-initiated tool call -- host policy, not something RaisinDB configures.
- **MCP-UI** (community project) -- a simpler fire-and-forget `postMessage({ type, payload }, '*')` with action kinds `tool`, `intent`, `prompt`, `notify`, `link`.

The resolution: **support both**, in the client helper, not the server. The widget helper prefers `ext-apps` (`callServerTool` present on `window`) and falls back to the raw `postMessage({type:'tool', ...})` convention. RaisinDB's server job is a good client-side helper and a good skill, not a new server mechanism.

There is no server-side gate for "safe vs. destructive one-click tools." Because hosts may or may not prompt for approval, the guidance -- keep destructive operations out of the set a widget can trigger without a second confirmation -- is **documented in the skill only**, using the existing `CustomTool.scopes` field. It is deliberately not enforced in server code, because the host, not RaisinDB, owns the approval UX.

## Raw-byte MCP resource reads (`mode: html`)

`mode: html` requires serving widget bytes over the MCP channel, which today's JSON-only `resources/read` cannot do (see [MCP Servers as Content](./mcp.md) -- `ResourceContents` has only `text`). Two additions close that gap:

- `ResourceContents` (`crates/raisin-mcp/src/resources.rs:57`) gains `blob: Option<String>` (base64), so a resource read can return raw bytes instead of JSON text.
- A new `AssetReader` service trait (plus `SharedAssetReader`) joins `FunctionInvoker` / `SearchProvider` / `EventSource` in `crates/raisin-mcp/src/services.rs`, so the engine keeps its "never touches storage directly" rule (`services.rs:29`). The HTTP side implements it as `HttpAssetReader` in `handlers/mcp/services/`, backed by `state.bin` -- which is already threaded into the MCP handler's dependency graph via `handlers/mcp/api_factory.rs`.

A useful side effect: any uploaded asset (not just widgets) becomes readable byte-for-byte over `resources/read`, which is generally desirable.

## ETag and caching

Static assets benefit from real HTTP caching that a JSON CRUD route would not bother with, so the static endpoint gets **its own** cache story:

- **ETag** is derived from the asset's `content_hash` (already stored on `raisin:Asset`, `raisin_asset.yaml`), giving cheap conditional requests.
- **`Cache-Control`** defaults so that ordinary assets (CSS/JS/images) are cacheable, but the **index document defaults to `no-cache`** unless `serving_config.cache_control` overrides it.

**Why the index-document exception?** A `mode: uri-list` SPA's `index.html` carries the client-side route table. If an over-broad `cache_control` let a stale `index.html` be cached, the host would keep serving an old route table after the widget was updated, and fragments would resolve against routes that no longer exist. Keeping `index.html` at `no-cache` by default (while other assets cache freely) avoids that specific footgun; an author who knows their index is stable can opt into caching it via `serving_config.cache_control`.

## RLS applies at two different surfaces

A subtle but important point for widget authors, worth being explicit about in the design:

- **`mode: html`** -- the widget's `data` comes from the tool call's `structuredContent`, which is already RLS-scoped to the caller because `raisin:Function` execution is. Nothing new; the widget only ever sees what the caller may see.
- **`mode: uri-list`** -- there are now *two* access-control surfaces. The tool call's `structuredContent` is one (RLS-scoped as above). But the iframed page can also make its *own* HTTP calls back into RaisinDB, and those are governed by the static endpoint's own auth/RLS -- a separate surface. A widget author must not assume "RLS on the tool call" implies "RLS on the iframe's direct API calls"; both are enforced, but they are different code paths clearing different requests.

## Decision: the widget helper is a separate package

The widget-side runtime helper ships as its own browser-only package, `@raisindb/mcp-ui-client`, not folded into `packages/raisin-client-js`. It provides:

- `getInitialRoute()` -- reads `window.__RAISIN_INITIAL_ROUTE__` (the `mode: html` injection) falling back to `location.hash` (`mode: uri-list`), so widget code never needs to know which mode served it.
- `getInitialData()` -- normalizes however the host delivered the tool's `structuredContent` on load.
- `callTool(name, args)` / `onToolResult(cb)` -- the both-conventions abstraction: tries `ext-apps` first, falls back to raw `postMessage`.
- `updateModelContext(content)` -- passthrough to `ext-apps` when present, a documented no-op otherwise.

**Why a separate package and not part of the SDK?** `raisin-client-js` runs inside a RaisinDB-*connected* backend -- it holds credentials, talks to the HTTP/WS APIs, and knows about nodes, queries, and functions. The widget helper runs *inside the widget iframe*: no credentials, no direct RaisinDB connection, its entire world is `postMessage` to a host bridge. That is a fundamentally different runtime and audience; bundling it into the SDK would drag connection code into a sandbox that must never have it. A sibling, `McpClient` (a thin typed wrapper over the `/mcp/{repo}/{branch}/{slug}` JSON-RPC methods, mirroring the existing `FunctionsApi` shape) *does* belong in `raisin-client-js`, because that one runs in a connected backend.

## Component map / touch list

For a maintainer navigating the implementation, the components and where they live:

| Component | Location | Change |
|-----------|----------|--------|
| MCP resource blob field | `crates/raisin-mcp/src/protocol.rs` / `resources.rs:57` | Add `ResourceContents.blob: Option<String>` (base64) |
| Byte-read path for `mode: html` | `crates/raisin-mcp/src/resources.rs` | Raw-bytes read via the new `AssetReader` |
| Asset-reader service seam | `crates/raisin-mcp/src/services.rs` | New `AssetReader` trait + `SharedAssetReader` |
| Tool UI binding | `crates/raisin-mcp/src/server.rs` (`CustomTool`, `server.rs:113`) | `ui: Option<UiBinding>` where `UiBinding { mode, entry }`; parse `entry` into `(path, fragment)` on first `#` |
| Result shaping + HTML injection | `crates/raisin-mcp/src/dispatch.rs` | Shape `CallToolResult` per mode; inject `window.__RAISIN_INITIAL_ROUTE__` for `mode: html` with a fragment |
| HTTP asset reader | `crates/raisin-transport-http/src/handlers/mcp/services/` | New `HttpAssetReader` backed by `state.bin` |
| Static-content route | new: `crates/raisin-transport-http/src/handlers/static_site.rs` (name TBD) | `GET /resources/{repo}/{branch}/{ws}/{*path}` (top-level, not under `/api`); path → Asset/Folder → bytes, index fallback. **Must** use `node_service_for_context` (like `helpers.rs`), never raw `state.storage().nodes()` (like `assets.rs`) |
| Per-folder header resolution | `crates/raisin-transport-http/src/middleware/cors.rs` | Extend the hierarchical resolver one level deeper for `StaticSiteFolder.serving_config` |
| New nodetype | new: `crates/raisin-core/global_nodetypes/raisin_static_site_folder.yaml` | `raisin:StaticSiteFolder extends raisin:Folder`, `serving_config: Object` |
| Server schema | `builtin-packages/raisin-mcp/nodetypes/mcp_server.yaml` | `tools[].ui` → `{ mode, entry }` |
| SDK MCP client | `packages/raisin-client-js` | New `McpClient` (`listTools`/`callTool`/`readResource`/`subscribeResource`), mirroring `FunctionsApi` |
| Widget runtime helper | new package: `@raisindb/mcp-ui-client` | `getInitialRoute`/`getInitialData`/`callTool`/`onToolResult`/`updateModelContext`, auto-detecting ext-apps vs. MCP-UI |
| Skill topic | new: `packages/raisindb-skills/skills/raisindb-mcp-ui-widgets/SKILL.md` | Authoring guide; cross-linked from `raisindb-mcp-servers` |

**Reused unchanged:** the entire upload pipeline (multipart + chunked), `BinaryStorage`, the `Resource`/`Asset` model, RLS/`DataPolicy` gating, `get_by_path`, and the existing signed-URL asset endpoint (kept for expiring/shareable links, orthogonal to all of the above).

**Explicitly not built:** an out-of-band YAML/config file holding relative URLs into RaisinDB. Everything above stays addressable by ordinary node paths, so it keeps RLS scoping, MVCC history, replication, and `resources/subscribe` live-update behavior for free -- an external file would get none of that.

## What is deferred

- **Dynamic per-result UI.** Today `ui` is static per tool (one tool always renders the same widget/route). Letting a tool's *result* choose the widget/route (a function returning a dynamic `_ui: { entry, mode }` that `dispatch.rs` reads in preference to the static binding) is cheap to add later and not needed for the "two tools, two views" case, which already works.
