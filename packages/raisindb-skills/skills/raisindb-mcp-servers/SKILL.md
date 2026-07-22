---
name: raisindb-mcp-servers
description: "Expose your RaisinDB data and functions as Model Context Protocol (MCP) servers that agents and MCP clients connect to. Covers the raisin:McpServer node, auto data tools, custom function tools, auth (public/scopes/OAuth), and connecting a client. Use when adding an MCP server to a RaisinDB project."
---

# MCP Servers

RaisinDB can expose your content and server-side logic as one or more **Model Context Protocol (MCP) servers**. You declare a server as content — a `raisin:McpServer` node — and the database serves it over the MCP Streamable HTTP binding. Agents then call its tools to read/write your data and run your functions, under your access control.

You declare **what** the server exposes; RaisinDB handles the protocol, tool generation, auth, and dispatch.

**MANDATORY**: After creating or modifying any `.yaml` / `.node.yaml` file in `package/`, run:

    npm run validate

## When to use this

- **Zero-onboarding data server** — auto-generated tools (and optional live resources) over your NodeTypes, no code to write.
- **Custom app server** — your own tools (RaisinDB functions) exposed to an agent for your end-users.
- A repository can hold **many** servers, each with its own slug, policy, and auth.

---

## The endpoint

Each `raisin:McpServer` node is served at a branch-aware endpoint:

```
POST /mcp/{repo}/{branch}/{slug}
```

- `{slug}` is the server node's `slug` property.
- `{branch}` makes it publish-aware — agents hit `live` (or `main`) by default; an editor agent can target a working branch by changing `{branch}`.
- The body is one JSON-RPC 2.0 message (`initialize`, `tools/list`, `tools/call`, `resources/*`).

---

## File organization

MCP server nodes live in the **`mcp`** workspace (the engine's discovery workspace). The builtin `raisin-mcp` package provisions that workspace and allows the node type there.

```
content/mcp/{slug}/
└── .node.yaml          # raisin:McpServer definition
```

Declare it in your package manifest so it ships and installs:

```yaml
# manifest.yaml
provides:
  mcp_servers:
    - /mcp/{slug}        # path of the raisin:McpServer node (workspace `mcp`, path /{slug})
  functions:             # only if the server exposes custom function tools
    - /functions/lib/{ns}/{fn}
```

---

## The `raisin:McpServer` node

```yaml
node_type: raisin:McpServer
properties:
  name: Catalog                       # advertised at initialize (required)
  slug: catalog                       # → /mcp/{repo}/{branch}/catalog (required, unique)
  version: "1.0.0"
  instructions: Query and manage the product catalog.
  public: false                       # see Auth below
  scopes: []                          # roles/groups required to open the server

  # (A) Auto data tools — generated from your NodeTypes. No code.
  data:
    workspaces: [products, categories]  # content workspaces the tools may touch
    operations:                         # one built-in tool per entry
      - query_nodes
      - get_node
      - search_nodes
      - create_node
      - update_node
      - delete_node
      - list_workspaces
    resources: false                    # expose raisin:// resources + live subscribe

  # (B) Custom tools — each maps to a raisin:Function node.
  tools:
    - function: recommend               # name of the raisin:Function node
      name: recommend                   # tool name advertised to the client
      description: Recommend products for a customer.
      inputSchema:                      # JSON Schema for the arguments object
        type: object
        properties:
          customer_id: { type: string }
        required: [customer_id]
      scopes: [catalog:read]            # roles/groups required to call this tool
```

Set only `data:` for a pure auto server, only `tools:` for a pure custom server, or both. Every tool's `scopes` are checked as `tool.scopes ⊆ caller's roles/groups`.

> **Want a tool to render an interactive UI instead of JSON?** Add an `ui: { mode, entry }` block to a tool to attach an HTML widget (MCP-UI) — a rendered card, panel, or form with buttons that fire follow-up tool calls. See the **`raisindb-mcp-ui-widgets`** skill.

### Auto data tools

`operations` are generated verbatim — the exact tool names are:
`query_nodes`, `get_node`, `search_nodes`, `create_node`, `update_node`, `delete_node`, `list_workspaces`.
They operate on the `data.workspaces` you list, and **every call runs under the caller's row-level security** — a tool can never read or write what the caller couldn't. `search_nodes` uses full-text and vector search.

### Custom function tools

A custom tool runs an existing `raisin:Function` (see the `raisindb-functions-triggers` skill) **as the calling identity** (no admin escalation).

**Schemas are inherited — don't repeat them.** A `raisin:Function` already declares `input_schema` and `output_schema`. A custom tool reuses them: an omitted `name`/`description`/`inputSchema`/`outputSchema` is filled from the function in *both* forms below. The function's `output_schema` becomes the tool's `outputSchema`, and the result is returned as `structuredContent`.

1. **Function-side** — add an `mcp` block to the `raisin:Function` node; a bare `enabled: true` is enough (it's exposed on every server — the `functions` workspace is always scanned):

```yaml
# content/functions/lib/acme/recommend/.node.yaml
node_type: raisin:Function
properties:
  name: recommend
  entry_file: index.js:recommend
  language: javascript
  enabled: true
  input_schema: { type: object, properties: { customer_id: { type: string } }, required: [customer_id] }
  output_schema: { type: object, properties: { items: { type: array } } }
  mcp:                                  # promoted to a tool, inheriting the schemas above
    enabled: true
    scopes: [catalog:read]             # add name/description/inputSchema only to override
```

2. **Server-side** — the `tools:` list on the `raisin:McpServer` node. `inputSchema`/`outputSchema`/`description` are inherited from the referenced function when omitted, so a minimal entry is just `{ function, name, scopes }`. On a name collision the server-side entry wins.

The function receives the tool arguments as its input and returns the tool result. A failed function surfaces as a tool error (`isError: true`), not a transport error.

---

## Auth

- **`public: true`** — anyone may open the server (no credential). Public tools must not rely on per-user data; tools that declare `scopes` still require them.
- **`public: false`** — the caller must be authenticated. **Important:** if anonymous access is enabled for the repository, an unauthenticated request resolves to the *anonymous principal*, which still satisfies a non-public server that declares **no** `scopes`. To restrict a non-public server to specific callers, declare `scopes` the anonymous role does not hold. (Data tools are RLS-scoped to the caller either way.)
- **Scopes are roles/groups** from `raisin:access_control` (see the `raisindb-access-control` skill). Consent/login narrows a caller to the scopes they hold — it never widens them.

### How clients authenticate

- **Interactive MCP clients** discover the OAuth 2.1 authorization server automatically via `/.well-known/oauth-authorization-server` and `/.well-known/oauth-protected-resource/mcp/{repo}/{branch}/{slug}`, register dynamically, and log in with PKCE. No tokens to paste.
- **Headless / first-party agents** present a RaisinDB access token as `Authorization: Bearer <token>`.

---

## Resources (optional)

Set `data.resources: true` to expose each node as a `raisin://{workspace}/{path}` MCP resource. Clients can `resources/read` a node and `resources/subscribe` to receive live `notifications/resources/updated` over Server-Sent Events as nodes change.

---

## Connect a client

Point any MCP client at the Streamable HTTP URL:

```
https://<host>/mcp/{repo}/main/catalog
```

Most MCP clients let you add an HTTP server by URL (e.g. an `mcp add --transport http <name> <url>` command, or an entry in the client's MCP config). Interactive clients trigger the OAuth login on first connect; for headless use, send the bearer token your client supports.

### Quick manual check

```bash
# initialize
curl -s https://<host>/mcp/{repo}/main/catalog \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"cli","version":"1.0"}}}'

# list tools (add -H 'authorization: Bearer <token>' for a non-public server)
curl -s https://<host>/mcp/{repo}/main/catalog \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
```

---

## Note: indexing delay

A newly created or edited `raisin:McpServer` node is discovered via an indexed query, which settles a moment after the write. If a brand-new server returns "no raisin:McpServer with slug …" on the very first call, retry after a short pause — it resolves once the index catches up. (In the normal publish flow this is a non-issue: the index is built by the time you merge to the live branch.)

## Checklist

- [ ] `raisin:McpServer` node in the **`mcp`** workspace with a unique `slug`.
- [ ] `provides.mcp_servers` lists the node path; `provides.functions` lists any custom-tool functions.
- [ ] Chose a tool surface: `data.operations`, `tools`, or both.
- [ ] Auth decided: `public: true`, or `public: false` **with `scopes`** to truly restrict.
- [ ] `npm run validate` passes.
- [ ] Connected a client and confirmed `initialize` + `tools/list`.
