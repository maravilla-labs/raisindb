# MCP Servers as Content

RaisinDB ships a complete Model Context Protocol (MCP) JSON-RPC engine in the `raisin-mcp` crate, exposed over HTTP by `raisin-transport-http`. This chapter documents how that engine is put together, because the newer [Native MCP-UI Support](./mcp-ui.md) feature builds directly on the pieces described here and only makes sense against them.

The one design decision that colors everything else: **an MCP server is content, not configuration.** There is no server registry, no config file, no restart-to-reload. A server is a `raisin:McpServer` node living in a workspace, resolved by slug at request time. Everything that follows -- multi-tenancy, RLS, versioning, replication, live subscriptions -- falls out of that choice for free, because a server descriptor is subject to exactly the same machinery as any other node.

## The transport-agnostic core

`raisin-mcp` never touches RocksDB, `AppState`, or the index engines directly. It is a pure protocol engine that depends on a small set of narrow traits, which the HTTP layer implements against the real services. This is the same separation used elsewhere in the codebase (the storage traits, the audit sink), and it is what lets the MCP tests run without a full server.

The service seams are declared in `crates/raisin-mcp/src/services.rs`:

| Trait | Responsibility | HTTP-side implementation |
|-------|----------------|--------------------------|
| `FunctionInvoker` | Resolve + execute a `raisin:Function` by name, as the calling identity | `HttpFunctionInvoker` (`handlers/mcp/services/invoker.rs`) |
| `SearchProvider` | Full-text (Tantivy) and vector (HNSW) search over a workspace | `HttpSearchProvider` (`handlers/mcp/services/search.rs`) |
| `EventSource` | Live node-change stream, bridged from the in-process event bus | `BusEventSource` (`handlers/mcp/services/events.rs`) |

Note the deliberate exception documented in `services.rs:29`: the node read/write/query/SQL data path is **not** abstracted behind one of these traits. It uses `raisin_functions::FunctionApi` directly (see `crate::data_tools`), which is the same RLS-scoped backend that server-side functions run against. That matters for the MCP-UI work, because it establishes the rule the new `AssetReader` trait has to respect -- the engine reaches storage only through a declared seam, never `AppState` directly.

## The server descriptor

`McpServerDescriptor` (`crates/raisin-mcp/src/server.rs:276`) is the parsed, validated shape of a `raisin:McpServer` node. It is read off the node's properties by `from_node` / `from_properties` (`server.rs:315`), tolerating absent optional keys, and the same parser serves both the typed-`Node` path and SQL-row resolution.

A descriptor carries:

- `name`, `version`, `slug` -- identity; `slug` is the routing key and the only strictly required field (`server.rs:332`).
- `instructions` -- natural-language usage guidance surfaced to the agent.
- `public` -- whether the server is reachable without authentication.
- `scopes` -- scopes a caller must hold to open the server at all.
- `data_policy` -- a `DataPolicy` (`server.rs:88`) declaring which `workspaces`, which `operations`, and whether `raisin://` `resources` are exposed by the auto-generated data tools.
- `custom_tools` -- a list of `CustomTool` (`server.rs:113`), each mapping a tool name to a `raisin:Function` node.

### Two kinds of tools

There are two ways a tool comes to exist on a server, and both resolve to the same `CustomTool` shape:

1. **Server-side declaration.** The `raisin:McpServer` node lists a tool in its `tools[]` array (`{ function, name, description, inputSchema, scopes }`), parsed by `parse_custom_tools` (`server.rs:422`). Omitted fields are backfilled from the referenced function's own metadata via `fill_defaults_from` (`server.rs:258`).
2. **Function-side opt-in.** A `raisin:Function` node carries an `mcp` block (`mcp: { enabled: true, ... }`), and `CustomTool::from_function_properties` (`server.rs:194`) turns it into a tool, defaulting `name`/`description`/`inputSchema` to the function's own `name`/`description`/`input_schema`.

Either way, a tool is ultimately a `raisin:Function` invoked as the caller. This is the anchor point for the MCP-UI feature: a widget is bound to a tool, and the tool is already an RLS-scoped function call.

## The DataOperation set

`DataOperation` (`server.rs:34`) enumerates the seven data operations a server may auto-expose as tools: `query_nodes`, `get_node`, `search_nodes`, `create_node`, `update_node`, `delete_node`, `list_workspaces`. `is_write()` (`server.rs:64`) flags the three mutating ones. The `DataPolicy` gates which of these are actually turned on, per server.

## Dispatch

The `Dispatcher` (`crates/raisin-mcp/src/dispatch.rs:36`) is the transport-agnostic router. Given an `McpIdentity` and a decoded `JsonRpcRequest`, it:

1. **Enforces the session scope gate** (`authorize_session`, `dispatch.rs:114`). A `public` server is open; otherwise the caller must be non-anonymous and hold every scope the server declares. Note `public: false` means "not anonymous" even when the server lists no scopes.
2. **Routes one of six MCP methods** (`dispatch.rs:98`): `initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `resources/subscribe`.
3. **Enforces the per-tool scope gate** inside `tools/call` (`dispatch.rs:164`) before invoking anything.

A function-level failure (`McpError::FunctionFailed`) is mapped onto an MCP `isError` result rather than a JSON-RPC protocol error (`dispatch.rs:188`) -- the model sees the failure and can react, rather than the transport aborting.

### Tool results and `structuredContent`

When a tool declares an `outputSchema`, its result is surfaced as `structuredContent` alongside the plain content block (`dispatch.rs:180`). This is the exact channel the MCP-UI feature reuses to deliver a widget's initial `data`: no new result type is needed, because a tool bound to a widget already returns structured content the host can hand to the renderer.

## Resources: JSON today

Resources are read-only, addressable content under a `raisin://{workspace}/{node/path}` URI (`crates/raisin-mcp/src/resources.rs`). `NodeResourceProvider::read` (`resources.rs:133`) resolves the node through `FunctionApi` (RLS-scoped) and returns its properties as a JSON `text` block. `subscribe` (`resources.rs:154`) bridges the `EventSource` into a filtered stream of `resources/updated` notifications, driving live updates over Server-Sent Events.

The load-bearing limitation for MCP-UI is visible in the wire type: `ResourceContents` (`resources.rs:57`) has a `text: String` field and **no `blob` field**. Nothing can be served byte-for-byte over the MCP channel today -- every resource read is JSON text. Serving an HTML widget or an image through `resources/read` is exactly what the MCP-UI work has to unlock, and it does so by adding `ResourceContents.blob` plus a byte-reading service seam. See [Native MCP-UI Support](./mcp-ui.md).

## The HTTP binding

A single route, `POST /mcp/{repo}/{branch}/{slug}`, carries one JSON-RPC 2.0 message per request over the MCP Streamable HTTP binding (`crates/raisin-transport-http/src/handlers/mcp/mod.rs`). The handler:

- authenticates through the existing auth middleware (the resolved `AuthContext` arrives in request extensions),
- projects that onto an `McpIdentity`,
- resolves the `{slug}` against the `mcp` workspace (`MCP_DISCOVERY_WORKSPACE`, `mod.rs:58`) -- note the server *declarations* live in `mcp`, while each server's `dataPolicy.workspaces` govern which *content* workspaces its tools may touch,
- assembles the live tool set and dispatches the method against the real services in `AppState`.

A JSON-RPC notification (no `id`) gets an empty `202`; `resources/subscribe` upgrades to an SSE stream.

## Why this shape matters for MCP-UI

Everything the MCP-UI feature needs is a small extension of a seam that already exists here:

- Widgets are bound to **tools**, which are already RLS-scoped `raisin:Function` calls.
- A widget's initial data rides the existing **`structuredContent`** channel.
- Serving widget bytes needs one new field on **`ResourceContents`** and one new **service trait** (`AssetReader`) that honors the "engine never touches storage directly" rule.
- Live widget updates get **`resources/subscribe`** for free.

The next chapter works through those extensions and the decisions behind them.
