# raisin-mcp

The Model Context Protocol **server** surface — serving RaisinDB's own tools,
resources and UI widgets to external MCP clients.

## Overview

The inbound half of RaisinDB's MCP support. The outbound half — RaisinDB calling
*other* servers' tools — lives in
[`raisin-mcp-protocol`](../raisin-mcp-protocol), which this crate depends on and
re-exports.

- **`registry`** — `Tool` / `DynTool` / `ToolRegistry`, plus the assembly that
  turns a server descriptor into a live tool set (`assemble_registry`,
  `resolve_plan`, `discover_function_tools`).
- **`server`** — the parsed `raisin:McpServer` node: `McpServerDescriptor`,
  `DataPolicy`, `CustomTool`, and the MCP-UI binding.
- **`data_tools`** — the built-in node/search tools a server's data policy
  switches on.
- **`dispatch`** — the JSON-RPC router: `initialize`, `server/discover`,
  `tools/*`, `resources/*`, `subscriptions/listen`.
- **`resources`** — `NodeResourceProvider`, serving nodes as `raisin://`
  resources with live change subscriptions.
- **`services`** — the narrow traits the host implements (`FunctionInvoker`,
  `SearchProvider`, `EventSource`, `AssetReader`).
- **`identity`** — `McpIdentity`, and the bridge to `AuthContext` / RLS.

## The one decision that shapes everything

**An MCP server is content, not configuration.** There is no server registry, no
config file, no restart-to-reload: a server is a `raisin:McpServer` node in the
`mcp` workspace, resolved by slug at request time. Multi-tenancy, RLS,
versioning, replication and live subscriptions all fall out of that for free,
because a descriptor is subject to the same machinery as any other node.

## Transport-agnostic by construction

This crate never touches RocksDB, `AppState` or the index engines. It depends on
the narrow traits in `services.rs`, which `raisin-transport-http` implements
against the real services — the same separation used for the storage traits and
the audit sink, and what lets these tests run without a server.

One deliberate exception, documented at `services.rs:29`: the node
read/write/query/SQL path is *not* behind one of those traits. It uses
`raisin_functions::FunctionApi` directly, which is the same RLS-scoped backend
server-side functions run against.

## Usage

```rust,ignore
use raisin_mcp::{assemble_for_slug, Dispatcher, McpIdentity};

// Resolve the `raisin:McpServer` node by slug and bind its tools to a caller.
let plan = assemble_for_slug(&services, "my-server").await?;
let registry = assemble_from_plan(&plan, &identity, &services);
let dispatcher = Dispatcher::new(identity, registry);

let response = dispatcher.handle(&request).await;
```

Served over HTTP at `POST /mcp/{repo}/{branch}/{slug}`
(`raisin-transport-http/src/routes/mcp.rs`).

## Protocol notes

- Streamable HTTP: one JSON-RPC message per POST, no session store.
  `subscriptions/listen` upgrades to SSE for change notifications.
- Several revisions are accepted (see `SUPPORTED_PROTOCOL_VERSIONS`). 2026-07-28
  moved negotiation into per-request `_meta`, but every shipping client still
  uses `initialize`, so both handshakes are served.
- MCP Apps (SEP-1865) widgets are `ui://` resources referenced from a tool's
  `_meta.ui.resourceUri`; a tool result carries data only.

## Auth

Callers arrive with a resolved `AuthContext`; `McpIdentity` narrows it to
granted scopes (roles ∪ groups, intersected with an OAuth `scope` claim when
present — consent can only narrow, never widen). RLS always uses the real roles.
Scopes gate at two levels: the server (`authorize_session`) and each tool
(`dispatch/tools.rs`).

Note the caveat at `services/invoker.rs:36`: `McpIdentity::to_auth_context`
leaves `resolved_permissions: None`, which RLS reads as deny-all — which is why
the invoker carries the middleware-resolved `AuthContext` separately.

## Further reading

- `book/src/architecture/mcp.md` — how the engine is put together
- `book/src/architecture/mcp-ui.md` — MCP-UI widgets
- `book/src/architecture/mcp-client.md` — the outbound direction
