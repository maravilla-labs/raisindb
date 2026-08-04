# MCP Client: Calling Other Servers

The previous chapter describes RaisinDB as an MCP **server** — external clients calling its tools. This one is the other direction: RaisinDB as an MCP **client**, so its agents can call tools that live on somebody else's server.

The design goal was that a remote tool should be indistinguishable from a local one at the point of use. An agent's `tools:` array holds paths; after discovery, some of those paths happen to resolve to a remote server. Nothing in the agent, the tool resolvers, the permission model or the `AIToolCall` machinery needed to learn a new concept.

## Remote tools are materialized as proxy functions

The alternative — a second tool kind that every consumer branches on — would have forked the tool path in the JS chat resolver, the Rust flow resolver, the tool-call dispatcher and the MCP re-export. That is the mirrored-code-path failure mode this codebase keeps paying for.

Instead, tool discovery writes one ordinary `raisin:Function` node per remote tool, under `/mcp/{connection-slug}/{tool-slug}` in the `functions` workspace. It carries the remote `inputSchema` verbatim, plus an `mcp_proxy` block naming its connection and the **verbatim** remote tool name:

```yaml
node_type: raisin:Function
name: linear__search-issues
properties:
  name: linear__search-issues     # node name and props.name MUST match
  language: javascript            # placeholder; never executed
  input_schema: { ... }           # remote inputSchema, verbatim
  mcp_proxy:
    connection: /mcp-connections/linear
    remote_tool: "search_issues"  # the ONLY thing sent in tools/call
    schema_hash: "sha256:..."
    state: active
```

Two details are load-bearing. The node name and `props.name` must be equal, because the JS resolver reads the node name and the Rust resolver reads `props.name` — a mismatch shows the model two different names for one tool. And `props.name` is namespaced `{slug}__{tool}` because `raisin:Function.name` is `unique` and *enforced*: two connections each exposing `search` would otherwise make the second connection's discovery fail outright.

## One execution branch

`mcp_proxy` is the discriminator, and it is checked in `execute_function` (`raisin-functions/src/execution/executor.rs`) between loading the function node and loading its code.

The position is deliberate. It cannot be `execution_mode`: an unrecognised value there silently parses as `Async`, so a "remote" mode would be indistinguishable from a normal function. It cannot be `language` either: an unrecognised value is a hard error, and the check has to run *before* the language is read.

Because all three execution paths — the JS chat path via the `AIToolCall` job, `raisin.functions.execute()`, and the Rust flow runtime — funnel through `execute_function`, this single branch serves all of them. There is no second implementation to drift.

A tool that runs and reports failure (`isError: true`) becomes `success: false` with a message, not a transport error. That is what makes the JS path (which writes an `error` property the model sees next turn) and the flow path (which produces `{"error": …}`) yield identical shapes.

## Connections are content too

A `raisin:McpConnection` node in `raisin:system` describes one remote server: URL, auth mode, tool filter, refresh policy, discovered tools, health. Credentials are AES-256-GCM ciphertext, written through a write-only endpoint and never returned by any read.

A proxy carries the **connection's** authority, not the calling user's — every caller shares one service account. That is why proxies are created under a restrictively-ACL'd `/mcp` folder: attaching one to an agent should be a deliberate act.

## Discovery reconciles; it does not rebuild

`reconcile_plan` (`raisin-mcp-protocol/src/client/reconcile.rs`) is a pure function of *what exists* and *what the server offers*, so the two properties the design leans on are directly testable:

- **A steady-state refresh writes nothing.** Discovery may run hourly forever; a schema-hash comparison skips unchanged tools. Without it each connection would mint thousands of function revisions a year. The hash is over *canonical* JSON — this workspace builds `serde_json` with `preserve_order`, so key order alone would otherwise change the hash and rewrite everything on every run.
- **Proxy paths never change.** An agent holds a path; a rename makes the tool vanish from that agent with no error anywhere. Slugs are derived deterministically from the remote name, and collision suffixes are assigned in sorted remote-name order — so a server that reorders its listing between calls cannot renumber them.

A tool that disappears upstream is **disabled, not deleted**: a missing node makes the tool silently vanish, while a disabled one stays visible in the console and in the connection's tool list. A *failed probe* touches no proxies at all — a remote being down means the tool list is unknown, not empty, and treating it as empty would tear down every proxy on a transient outage.

## Cluster safety

Discovery writes shared content, so it must run once per cluster. The job registry's dedup key does **not** provide that: its map is in-memory and per-process, so every node runs its own scan. Single-fire comes from a per-connection `raisin_locks` lease inside the handler, which needs the `redis` backend to span nodes. An in-process `KeyedMutex` sits in front of it so a manual refresh racing a scheduled one *queues* rather than being rejected.

## Egress

A connection URL is operator-supplied and dialled by the server itself — textbook SSRF. `EgressPolicy` requires `https` (loopback only behind an explicit flag) and rejects private, loopback, link-local (including cloud metadata at `169.254.169.254`), CGNAT and IPv6 ULA addresses.

It is enforced **twice**: when a connection is saved, and again before every dial. A save-time-only check misses a hostname that resolved publicly and was re-pointed afterwards. Configuration is the `[mcp_client]` TOML section, installed process-wide, so every path enforces one policy rather than each wiring its own.

## Crate layout

The client lives in `raisin-mcp-protocol`, not `raisin-mcp`, and that is structural rather than tidiness. `raisin-mcp` serves tools, so it depends on `raisin-functions` — which depends on `raisin-rocksdb`. Anything at or below the storage layer (the discovery job handler, for one) therefore cannot depend on `raisin-mcp` without closing a package cycle, and Cargo rejects package cycles regardless of features.

`raisin-mcp-protocol` holds the wire types and the client with no `raisin-functions` dependency; `raisin-mcp` depends on it and re-exports it, so existing `raisin_mcp::protocol::…` paths still resolve and nothing is duplicated.
