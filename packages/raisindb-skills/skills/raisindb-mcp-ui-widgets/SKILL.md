---
name: raisindb-mcp-ui-widgets
description: "Attach interactive HTML views (MCP Apps, SEP-1865) to your RaisinDB MCP tools, so a tool call renders a mini-app in the MCP host instead of plain JSON. Covers the ui: tool binding (entry/workspace/csp/permissions/prefersBorder/visibility), ui:// resources served via resources/read, the @raisindb/mcp-ui-client view runtime (connect, onToolResult, callTool, updateModelContext, pull fallback), the kind-discriminator pattern for shared SPAs, and static /resources serving for images. Use when adding a UI to an existing MCP server."
---

# MCP Apps Widgets (SEP-1865)

An MCP tool can return more than JSON. With **MCP Apps**, a tool advertises a
predeclared HTML "view" that Apps-capable hosts (e.g. Claude Desktop) render in
a sandboxed iframe and drive over JSON-RPC. The tool still runs server-side as
the calling identity under your access control; the view renders whatever the
tool returned and can trigger follow-up tool calls through the host.

This skill builds **on top of** an existing MCP server (see
`raisindb-mcp-servers`). Widgets are ordinary built HTML files shipped as
`raisin:Asset` content — there is no widget node type.

**MANDATORY**: After creating or modifying any `.yaml` / `.node.yaml` file in
`package/`, run `npm run validate`.

## How the pieces fit

1. **You** ship a self-contained HTML file (one `index.html`, all JS/CSS
   inlined) as an asset, and add a `ui:` block to one or more tools on the
   `raisin:McpServer` node.
2. **RaisinDB** then:
   - advertises the widget on the tool in `tools/list` as
     `_meta.ui.resourceUri: "ui://{workspace}/{entry-path}"` (plus the
     deprecated flat `ui/resourceUri` key for pre-GA hosts),
   - predeclares it in `resources/list` (mime `text/html;profile=mcp-app`,
     name/description, `_meta.ui` with csp/permissions/prefersBorder),
   - serves the bytes via `resources/read` on the `ui://` URI (RLS-scoped
     asset read; content-level `_meta.ui` takes spec precedence).
   - Tool results carry **data only** (`content` + `structuredContent` — a
     tool MUST declare an `output_schema` or the view gets no structured
     data). Nothing UI is embedded in results.
3. **The host** fetches the view, renders it sandboxed, and drives it over
   postMessage JSON-RPC: `ui/initialize` handshake, then
   `ui/notifications/tool-input` / `tool-result`; view-initiated calls are
   plain `tools/call` requests the host proxies (and may gate with a user
   prompt).

## The `ui:` tool binding

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
        mode: html                          # html = Apps view. uri-list is
                                            # reserved (spec's deferred
                                            # externalUrl content type).
        workspace: assets                   # workspace the entry resolves in;
                                            # defaults to the session workspace
                                            # (FIRST entry of data.workspaces)
        entry: /widgets/order/index.html    # ABSOLUTE node path (exact match)
        name: Order Card                    # resources/list display name
        description: Renders an order.
        prefersBorder: true
        # csp: declare external origins the view needs. When OMITTED, the
        # engine declares this server's own origin (derived from
        # RAISINDB_BASE_URL or the request Host header) for connect+resource,
        # so same-instance images/API calls work out of the box.
        # csp:
        #   connectDomains: ["https://api.example.com"]
        #   resourceDomains: ["https://cdn.example.com"]
        # permissions: { clipboardWrite: {} }
        # visibility: [model, app]   # "app"-only tools are callable from the
        #                            # view but hidden from the agent
```

Several tools may bind the SAME entry file — the resource is listed once and
every result flows into the one view. Discriminate results **by shape**: give
every bound tool's output a `kind` field and route views off it (fragments on
the entry are tolerated but hosts don't reliably deliver them — don't rely on
them).

## The view runtime: `@raisindb/mcp-ui-client`

Browser-only, dependency-free, speaks the Apps JSON-RPC protocol. Not on npm
yet — consume via a workspace `link:` to `packages/mcp-ui-client` (build its
dist with `npm run build` there).

| Export | Purpose |
|---|---|
| `connect()` | starts the `ui/initialize` handshake (auto on import; retries until the host answers — a fire-at-parse-time init can be lost) |
| `onToolResult(cb)` | every `CallToolResult` the host delivers (initiating tool + view-initiated calls) |
| `onToolInput(cb)` / `getToolInput()` | the initiating call's arguments |
| `getInitiatingToolName()` | from `hostContext.toolInfo` — see pull fallback |
| `callTool(name, args)` | `tools/call` through the host; result also fans out to `onToolResult` |
| `updateModelContext(content)` | push what the user did back into the conversation |
| `openLink(url)` / `sendMessage(text)` | `ui/open-link` / `ui/message` |
| `getHostContext()` / `onHostContext(cb)` | theme, style variables (applied to `:root` automatically), dimensions |
| `getBridgeDebug()` / `onBridgeDebug(cb)` | live diagnostics (handshake state, received message counts/methods) — surface in a debug footer while developing |

Content size is reported automatically (`ui/notifications/size-changed` via
ResizeObserver) — never use `100vh`.

### The pull fallback (IMPORTANT)

The host only guarantees `tool-result` when the view is displayed **during**
execution. When the tool finished before the view initialized, no result push
comes — the view must PULL: read `getInitiatingToolName()` + `getToolInput()`
and re-issue the same (read-only!) call:

```ts
let got = false;
onToolResult((r) => { got = true; render(r.structuredContent); });
setTimeout(() => {
  const name = getInitiatingToolName();
  if (!got && name && READ_ONLY_TOOLS.has(name)) callTool(name, getToolInput() ?? {});
}, 1200);
```

The host may show a permission prompt for view-initiated calls — keep pulled
tools read/idempotent, and keep destructive tools off one-click paths (or gate
them with `scopes` / `visibility`).

## Building & shipping the view

Compile each widget to ONE self-contained HTML (vite + `vite-plugin-singlefile`,
`cssCodeSplit: false`, `assetsInlineLimit` huge) and ship it as package content:
any non-yaml file under `content/<ws>/<dirs>/<file>` installs as a
`raisin:Asset` at `/<dirs>/<file>` in `<ws>`. Allow `raisin:Asset` (+ your
folder type) in that workspace. Reference implementation:
`maravilla-labs/studio` → `packages/mcp-widgets` (Svelte 5 SPA, three views
routed by result `kind`, shared design tokens).

## Images and static serving (`/resources`)

Views run under a host CSP built from the declared domains. With the engine's
default CSP (your server's origin), a view can load images from
`GET /resources/{repo}/{branch}/{ws}/{*path}` — which is deny-by-default:

- A path is servable only when a **`raisin:StaticSiteFolder`** ancestor covers
  it (an ordinary content node a package can ship; retype = delete + recreate,
  node_type is immutable).
- The iframe's requests are unauthenticated → the subtree must be readable by
  the **anonymous** role AND anonymous access must be enabled for the repo via
  a `raisin:RepoAuthConfig` node at `/config/repos/{repo}` in the
  `raisin:system` workspace (`anonymous_enabled: true`). Grant narrowly, e.g.
  `{workspace: assets, path: /widgets/**, operations: [read]}`.
- Stored `application/octet-stream` mimes are re-guessed from the filename at
  serve time, so built HTML/CSS/JS render instead of downloading.

Heavier alternative when serving can't be public: resolve images server-side to
small data URLs in the tool result (`Resource.resize({width}).toDataUrl()` in
the function runtime), capped hard — the payload rides in `structuredContent`.

## Checklist

- [ ] Tool has `ui: { mode: html, entry: /abs/path.html, workspace?, ... }`;
      the backing function declares an `output_schema` and its result carries a
      `kind` discriminator.
- [ ] Widget built single-file, shipped as `raisin:Asset`; `resources/read` of
      the `ui://` URI returns `text/html;profile=mcp-app`.
- [ ] View uses `@raisindb/mcp-ui-client`, renders a waiting state, routes
      results by `kind`, and implements the pull fallback.
- [ ] View-triggerable tools are read/idempotent; destructive ops gated by
      `scopes` or `visibility: [model]`.
- [ ] For image URLs: StaticSiteFolder + anonymous read + RepoAuthConfig, or
      data-URL fallback.
- [ ] `npm run validate` passes; `tools/list` shows `_meta.ui.resourceUri`.
