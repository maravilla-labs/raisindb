---
name: raisindb-mcp-ui-widgets
description: "Attach interactive HTML widgets (MCP-UI) to your RaisinDB MCP tools, so a tool call renders a mini-app in the MCP host instead of plain JSON. Covers raisin:StaticSiteFolder, uploading a widget, the ui: { mode, entry } tool binding, html vs uri-list delivery, #fragment SPA routes, the @raisindb/mcp-ui-client helper, and widget-initiated tool calls. Use when adding a UI to an existing MCP server."
---

# MCP-UI Widgets

An MCP tool can return more than JSON. With **MCP-UI**, a `tools/call` result carries an HTML "mini-app" that the MCP host (an agent's client) renders inline — a rendered order card, an inventory panel, a form with buttons. The tool still runs server-side as the calling identity under your access control; the widget is just a renderer of whatever the tool handed back, plus a bridge for the user to trigger follow-up tool calls.

This skill builds **on top of** an existing MCP server. Define the server, its data tools, and its custom function tools first with the **`raisindb-mcp-servers`** skill; then come here to give one or more of its tools a UI. Widgets are ordinary files uploaded through the normal asset pipeline — there is no widget node type — so uploading is covered by the **`raisindb-file-uploads`** skill and only referenced here.

You declare **what** renders (`ui: { mode, entry }` on a tool) and ship the widget files; RaisinDB handles serving, the initial route, the initial data, and dispatching widget-initiated tool calls.

**MANDATORY**: After creating or modifying any `.yaml` / `.node.yaml` file in `package/`, run:

    npm run validate

## When to use this

- **Render a tool's result as UI** — an order summary, a chart, a status board — instead of a JSON blob the agent has to describe in prose.
- **Let the user act from the widget** — buttons that fire follow-up tool calls (approve, refresh, select) against the same MCP session.
- **One SPA, many tool views** — a single built widget serving several tools, each bound to a different in-app route.

If you only need tools that return data, stop at `raisindb-mcp-servers` — you don't need a widget.

---

## The two delivery modes

A tool's `ui.mode` decides how the widget reaches the host. Pick per tool.

| | `mode: html` | `mode: uri-list` |
|---|---|---|
| What ships to the host | the widget's HTML **bytes**, inlined | a **URL** to the widget, served live |
| How the host renders it | `srcdoc` in the host's own sandbox | a real cross-origin `<iframe src=…>` |
| Best for | single-file widgets, small self-contained apps | multi-file apps (separate css/js/images), SPAs |
| Folder security config | **does not apply** (no cross-origin frame) | **required** — `serving_config` governs framing/CORS |
| Default choice | **yes — start here** | only when you genuinely need multiple files served live |

**`mode: html` is the safe default.** The MCP engine reads the entry file's bytes and returns them as a `text/html` resource; the host renders them via `srcdoc` in its own sandbox. No cross-origin iframe exists, so none of the folder-level security config below is involved — simplest and safest for small widgets.

**`mode: uri-list`** returns a `text/uri-list` pointing at the static endpoint (below). The host iframes it with a real `src=`, and the browser then makes ordinary relative requests for `./style.css`, `./app.js`, images, etc. against RaisinDB. This is the only mode where `raisin:StaticSiteFolder.serving_config` matters, because a real cross-origin iframe is involved.

---

## Step 1 — Host the widget files: `raisin:StaticSiteFolder`

Widget files live in the ordinary asset tree. `mode: html` can point at a lone `raisin:Asset` (a single `index.html`), but any real widget — and every `uri-list` widget — is a folder of files. Use **`raisin:StaticSiteFolder`**, a narrow opt-in subtype of `raisin:Folder` that (a) marks a subtree as static-servable and (b) carries the `serving_config` that drives response headers.

### The static-content endpoint

A `raisin:StaticSiteFolder` and everything under it is served path-shaped at:

```
GET /resources/{repo}/{branch}/{ws}/{*path}
```

- **Deny-by-default serving gate (REQUIRED prerequisite).** A path is servable through `/resources` **only if a `raisin:StaticSiteFolder` ancestor covers it**. If no folder up the path is a `raisin:StaticSiteFolder`, the request returns **404** — the folder's *presence* is the allowlist (there is no `enabled`/`servable` flag and no path-glob list). This gate is a coarse layer resolved **first**; then RLS decides which caller may read the specific node. `mode: html` widgets never hit `/resources`, so this gate does **not** affect them (no StaticSiteFolder needed) — only `mode: uri-list` requires it.
- Under the gate, the same auth stack as the rest of the API still applies (`optional_auth_middleware` → row-level security, **fail-closed**). A folder is public exactly when the anonymous role already has read access to it via `raisin:access_control`. See the `raisindb-access-control` skill.
- Resolving a `raisin:Asset` streams its `file` bytes with the stored mime type.
- Resolving a folder (or a trailing-slash / empty path) serves that folder's **index document** (`serving_config.index_document`, default `index.html`).
- Relative references inside the HTML (`./app.js`, `./img/logo.png`) are just more path lookups under the same subtree — nothing special to configure.
- ETags derive from each asset's `content_hash`; the index document defaults to `no-cache` (so a new SPA build is picked up), other assets are cacheable, both overridable via `serving_config.cache_control`.

### Create or retype the folder

Create it directly as a `raisin:StaticSiteFolder`:

```sql
INSERT INTO content (path, node_type, properties)
VALUES ('/content/widgets/order', 'raisin:StaticSiteFolder', $1::jsonb)
```

…or retype an existing `raisin:Folder` you already uploaded into:

```sql
UPDATE content SET node_type = 'raisin:StaticSiteFolder'
WHERE path = '/content/widgets/order'
```

Allow both `raisin:StaticSiteFolder` and `raisin:Asset` in the workspace that will hold widgets (`manifest.yaml` `workspace_patches`, exactly as the `raisindb-file-uploads` skill shows for `raisin:Asset`/`raisin:Folder`):

```yaml
workspace_patches:
  content:
    allowed_node_types:
      add:
        - raisin:StaticSiteFolder
        - raisin:Asset
```

### `serving_config` (only consulted in `uri-list` mode)

```yaml
node_type: raisin:StaticSiteFolder
properties:
  description: Order widget
  serving_config:
    frame_ancestors:                    # origins allowed to iframe this subtree
      - https://host.example.com        # the MCP host origin(s) that will embed this
    cors_allowed_origins:               # origins whose page JS may fetch RaisinDB APIs
      - https://host.example.com
    cache_control: "public, max-age=3600"
    index_document: index.html
```

| Field | Controls | Header emitted |
|---|---|---|
| `frame_ancestors` | **whether a host origin may iframe these pages at all** | `Content-Security-Policy: frame-ancestors …` |
| `cors_allowed_origins` | whether that page's client-side JS may fetch/XHR back into RaisinDB from a different origin | CORS headers, resolved per-folder-subtree from this `serving_config` |
| `cache_control` | HTTP caching for assets under this folder | `Cache-Control` |
| `index_document` | folder-root document name | — |

**Deny-by-default framing.** With **no** `serving_config` (or no `frame_ancestors`), **no** `Content-Security-Policy: frame-ancestors` header is emitted and the pages are **not embeddable** cross-origin. A `uri-list` widget will not frame until you list the host origins explicitly. `frame-ancestors` (embeddability) and `cors_allowed_origins` (cross-origin script calls) are two different browser mechanisms — a working "iframe a widget served from RaisinDB" story usually needs both set for the host origin.

**`cors_allowed_origins`: `"*"` and credentials are mutually exclusive.** List the exact host origin(s) rather than `"*"`. An explicitly-listed origin gets the response reflected back with `Access-Control-Allow-Credentials: true` and `Vary: Origin`, so the widget's JS can make **credentialed** (cookie/session) reads. A `"*"` entry emits `Access-Control-Allow-Origin: *` **without** `Access-Control-Allow-Credentials` — browsers reject `*` + credentials, and reflecting an arbitrary origin with credentials would let any site make credentialed cross-origin reads against RLS-gated content. So use `"*"` only for genuinely public, non-credentialed assets; use an explicit origin for anything the signed-in user must be able to read.

> `mode: html` never touches `serving_config` — the host renders inlined bytes in its own sandbox, so there is nothing to frame and no CORS surface. If you can, prefer `html` and skip this whole section.

### Ship the folder in a package (install ⇒ subtree servable)

A `raisin:StaticSiteFolder` is an ordinary content node, so a **package can ship it** as content and the install makes the subtree servable via `/resources` — no server config, no admin API, no `[resources]` TOML. Author it exactly like any other package content node: a `.node.yaml` under `content/<encoded-workspace>/<folder>/` (the workspace segment is encoded, e.g. `content` → `content`, `raisin:system` → `_raisin__system`, mirroring how the imap-adapter package ships its `raisin:Integration` at `content/_raisin__system/integrations/imap/.node.yaml`):

```yaml
# content/content/widgets/order/.node.yaml   → serves /resources/{repo}/{branch}/content/widgets/order/**
node_type: raisin:StaticSiteFolder
properties:
  serving_config:
    index_document: index.html
    frame_ancestors: ["https://host.example.com"]
    cors_allowed_origins: ["https://host.example.com"]
    cache_control: "public, max-age=3600"
```

Ship the widget's built files as `raisin:Asset` content under the same subtree (or upload them post-install, below). Installing the package places the `raisin:StaticSiteFolder` at that fixed path, so the covering-ancestor gate is satisfied and the whole subtree is servable — this is how the allowlist is "configured through packages." Creating, deleting, or retyping the folder takes effect within ~60s (the gate is resolved with system auth and cached with the same TTL as the CORS resolver — bounded staleness).

> The gate discovers a `raisin:StaticSiteFolder` only at a **named** path (e.g. `/site`, `/content/widgets/order`); one placed at the bare workspace root `/` is not discovered.

### Upload the widget

Use the normal upload pipeline — see the **`raisindb-file-uploads`** skill. Upload your built `index.html` (and its css/js/assets) into the folder path:

```typescript
const batch = await client.uploadFiles(builtFiles, {
  repository: 'my-repo',
  workspace: 'content',
  basePath: '/content/widgets/order',   // the StaticSiteFolder
  concurrency: 3,
});
await batch.start();
```

Nothing about the upload is widget-specific; every file becomes a `raisin:Asset` as usual.

---

## Step 2 — Bind the widget to a tool: `ui: { mode, entry }`

Add a `ui` block to a tool on the `raisin:McpServer` node (or to a function's `mcp` block). `entry` is a **workspace-relative path**, optionally with a `#fragment` (see below).

```yaml
node_type: raisin:McpServer
properties:
  name: Catalog
  slug: catalog
  tools:
    - function: get_order
      name: order_card
      description: Show an order as a card.
      ui:
        mode: html                                   # safe default
        entry: content/widgets/order/index.html
```

- `mode`: `"html"` or `"uri-list"`.
- `entry`: path to the index document. For `uri-list`, it must live under a `raisin:StaticSiteFolder`.

The tool still runs its `raisin:Function` exactly as before, RLS-scoped to the caller. Its result becomes the widget's **initial data** (delivered as `structuredContent`); the `ui` block just tells the host to render that result through your widget instead of showing raw JSON.

### One SPA, many tool routes — the `#fragment` convention

Build **one** SPA (`index.html` + a hash router) and bind several tools to it, each pointing at a different in-app route via a fragment after the first `#`:

```yaml
tools:
  - function: get_order
    name: order_card
    ui: { mode: uri-list, entry: content/widgets/app/index.html#/order-card }
  - function: list_inventory
    name: inventory_panel
    ui: { mode: uri-list, entry: content/widgets/app/index.html#/inventory }
```

`entry` splits into `path` + `fragment` on the first `#`. The fragment never affects *which file* is served — it only tells the widget which view to show. How it reaches the widget differs by mode, and the helper (Step 3) hides the difference:

- **`uri-list`** — the fragment rides on the iframe `src=` URL. The browser strips it before the HTTP request (so the server just serves `index.html`), and the SPA reads `location.hash` on mount, exactly like any website. Two tools pointing at the same file with different fragments are two distinct resource URIs to the host, even though the document is byte-identical.
- **`html`** — there is no navigable URL (the host gets raw bytes via `srcdoc`, whose location has no meaningful hash), so the engine injects a bootstrap global into the returned HTML: `<script>window.__RAISIN_INITIAL_ROUTE__="/order-card";</script>`. The file path is resolved with the fragment stripped.

Write your router bootstrap so it works unchanged in both modes:

```js
const route = window.__RAISIN_INITIAL_ROUTE__ ?? location.hash;
```

…or just call `getInitialRoute()` from the helper, which does exactly this.

---

## Step 3 — Widget code: `@raisindb/mcp-ui-client`

The browser-only helper runs **inside the widget iframe** and abstracts over the two delivery modes and the competing host bridges, so you write one call site regardless of where the widget ends up.

```bash
npm install @raisindb/mcp-ui-client
```

| Helper | Returns / does |
|---|---|
| `getInitialRoute()` | reads `window.__RAISIN_INITIAL_ROUTE__` (html mode), falls back to `location.hash` (uri-list) — mode-agnostic |
| `getInitialData()` | normalizes however the host delivered the tool's `structuredContent` on load |
| `callTool(name, args)` | fire a follow-up `tools/call` on the same session (see Step 4) |
| `onToolResult(cb)` | register a callback for results of `callTool` (or any host-initiated tool result) |
| `updateModelContext(content)` | push structured content back into the conversation for the model to see; documented no-op on hosts that don't support it |

`callTool`/`onToolResult` auto-detect the host bridge: they try the official MCP Apps extension (`callServerTool` / `ontoolresult` on `window`) first, and fall back to the community MCP-UI raw `postMessage({ type: 'tool', payload: { toolName, params } }, '*')` convention otherwise. You don't pick a convention.

```html
<!-- content/widgets/order/index.html -->
<!doctype html>
<html>
<head><meta charset="utf-8" /><title>Order</title></head>
<body>
  <div id="app">Loading…</div>
  <script type="module">
    import {
      getInitialRoute, getInitialData, callTool, onToolResult,
    } from 'https://esm.sh/@raisindb/mcp-ui-client';

    const route = getInitialRoute();          // e.g. "/order-card"
    const order = getInitialData();           // the get_order tool result, RLS-scoped
    render(route, order);

    onToolResult((result) => render(route, result));  // re-render on follow-up calls

    function render(route, data) {
      const app = document.getElementById('app');
      app.innerHTML = `
        <h1>Order ${data.id}</h1>
        <p>Status: ${data.status}</p>
        <button id="refresh">Refresh</button>`;
      app.querySelector('#refresh').onclick = () =>
        callTool('get_order', { order_id: data.id });   // Step 4
    }
  </script>
</body>
</html>
```

> A widget bundled for offline hosts should ship the helper inlined by your build step rather than importing from a CDN. The import above is illustrative.

---

## Step 4 — Widget-initiated actions ("click a button")

A button click is just an **ordinary `tools/call`** against the same MCP session — `callTool(name, args)` in the widget. RaisinDB's existing dispatch runs the same `raisin:Function`, RLS-scoped to the same caller, exactly like any other invocation. **There is no new RaisinDB wire protocol for this** — the whole round trip (button → message → host decides → maybe re-invokes `tools/call` → result → `onToolResult` → re-render) lives in the widget's JS and the host's bridge.

The follow-up tool is a normal server tool — define it in `raisindb-mcp-servers` like any other. It does **not** need a `ui` block unless *its own* result should also render a widget.

```yaml
tools:
  - function: get_order
    name: order_card
    ui: { mode: html, entry: content/widgets/order/index.html }
  - function: get_order                 # the Refresh button target — a plain tool
    name: get_order
    description: Fetch an order's current state.
```

### Safety: keep destructive ops off one-click tools

Some hosts **prompt the user before running a widget-initiated tool call; some do not.** Treat a widget button as capable of firing without a confirmation dialog. Therefore:

- A tool meant to be one-click-triggerable from a widget should be **read/idempotent** (fetch, list, recompute) — a refresh, not a purge.
- Keep **destructive or irreversible** operations (cancel, delete, charge, publish) out of the set a widget can trigger in one click. Put a second, deliberate confirmation elsewhere, or gate them behind `scopes` the widget's caller does not hold.
- `CustomTool.scopes` still applies to widget-initiated calls (`tool.scopes ⊆ caller's roles/groups`) — use it to fence risky tools. This is a **documentation/design convention**, not a server-enforced gate specific to widgets; the server enforces scopes, but it does not know a call came from a button.

---

## End-to-end worked example

Goal: a `get_order` tool that renders an order card, with a **Refresh** button and an **Approve** action, in an existing `catalog` MCP server.

**1. Allow the types and host the widget** (`manifest.yaml`):

```yaml
workspace_patches:
  content:
    allowed_node_types:
      add: [raisin:StaticSiteFolder, raisin:Asset]
```

Create the folder and upload the built widget (Step 1). Single-file widget → `mode: html`, so a lone `index.html` asset under a folder is enough; no `serving_config` needed.

```sql
INSERT INTO content (path, node_type, properties)
VALUES ('/content/widgets/order', 'raisin:StaticSiteFolder',
        '{"description":"Order widget"}'::jsonb);
```

```typescript
await (await client.uploadFiles(files, {
  repository: 'my-repo', workspace: 'content',
  basePath: '/content/widgets/order', concurrency: 3,
})).start();
```

**2. Define the tools** on the `raisin:McpServer` node:

```yaml
node_type: raisin:McpServer
properties:
  name: Catalog
  slug: catalog
  data:
    workspaces: [orders]
  tools:
    - function: get_order
      name: order_card                         # renders the widget
      description: Show an order as a card.
      ui: { mode: html, entry: content/widgets/order/index.html }
    - function: get_order
      name: get_order                          # Refresh — read-only, one-click safe
      description: Fetch an order's current state.
    - function: approve_order
      name: approve_order                      # NOT one-click — gated by scope
      description: Approve an order.
      scopes: [orders:approve]
```

`get_order` / `approve_order` are ordinary `raisin:Function` nodes — see `raisindb-functions-triggers`.

**3. Widget JS** (`content/widgets/order/index.html`), using the helper:

```html
<script type="module">
  import { getInitialData, callTool, onToolResult, updateModelContext }
    from 'https://esm.sh/@raisindb/mcp-ui-client';

  let order = getInitialData();               // get_order result, RLS-scoped to caller
  paint(order);
  onToolResult((r) => { order = r; paint(order); });

  function paint(o) {
    const el = document.getElementById('app');
    el.innerHTML = `
      <h1>Order ${o.id}</h1>
      <p>Status: <b>${o.status}</b></p>
      <button id="refresh">Refresh</button>
      <button id="approve" ${o.status === 'approved' ? 'disabled' : ''}>Approve</button>`;
    el.querySelector('#refresh').onclick = () => callTool('get_order', { order_id: o.id });
    el.querySelector('#approve').onclick = async () => {
      await callTool('approve_order', { order_id: o.id });   // host may prompt for approval
      updateModelContext([{ type: 'text', text: `Order ${o.id} approval requested.` }]);
    };
  }
</script>
<div id="app">Loading…</div>
```

- **Refresh** → `get_order` (read-only, safe to fire without a prompt).
- **Approve** → `approve_order` (mutating) — gated by the `orders:approve` scope, and hosts that prompt will surface a confirmation. `updateModelContext` tells the model what happened so the conversation stays coherent.

**4. Validate and connect:**

```bash
npm run validate
```

Connect a client to `https://<host>/mcp/my-repo/main/catalog` (see `raisindb-mcp-servers`), call `order_card`, and the host renders the widget instead of JSON.

---

## Checklist

- [ ] An MCP server already exists (see `raisindb-mcp-servers`); this widget attaches to one of its tools.
- [ ] Widget files hosted under a `raisin:StaticSiteFolder`; types allowed in the workspace; uploaded via the normal pipeline (`raisindb-file-uploads`).
- [ ] For `uri-list`: a `raisin:StaticSiteFolder` ancestor covers the widget path (at a named path, not the bare ws root) — else `/resources` returns 404 (deny-by-default). `mode: html` is exempt.
- [ ] Tool has `ui: { mode, entry }`; `entry` is workspace-relative (with `#fragment` for SPA routes).
- [ ] Mode chosen: `html` (default, srcdoc, no `serving_config`) or `uri-list` (multi-file, needs `serving_config.frame_ancestors` for the host origin — deny-by-default).
- [ ] For credentialed cross-origin widget calls, `cors_allowed_origins` lists the exact host origin(s), not `"*"` (`"*"` is served without credentials).
- [ ] Widget uses `@raisindb/mcp-ui-client` (`getInitialRoute`/`getInitialData`/`callTool`/`onToolResult`/`updateModelContext`) — mode- and host-agnostic.
- [ ] Widget-triggerable tools are read/idempotent; destructive ops kept off one-click paths and/or gated by `scopes`.
- [ ] `npm run validate` passes; client renders the widget on `tools/call`.
