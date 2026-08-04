# raisin-mcp-protocol

Model Context Protocol wire types, and the outbound MCP **client**.

## Overview

RaisinDB speaks MCP in both directions. This crate holds everything the two
directions genuinely share, plus the client half:

- **`protocol`** — JSON-RPC 2.0 envelopes and the typed MCP payloads
  (`initialize`, `server/discover`, `tools/list`, `tools/call`, `resources/*`),
  with the supported revision list and the `_meta` negotiation keys.
- **`content`** — `ContentBlock`, the entry type of a tool result.
- **`resource_types`** — `ResourceContents` / `ResourceDescriptor` and the
  `raisin://` URI helpers.
- **`props`** — small readers for a node's serialized property map, shared by
  the server's `McpServerDescriptor` and the client's `McpConnectionDescriptor`.
- **`client`** — RaisinDB calling somebody else's MCP server (feature `client`,
  on by default).

The *server* half — serving RaisinDB's own tools to external clients — lives in
[`raisin-mcp`](../raisin-mcp), which depends on this crate and re-exports it.

## Why this crate exists

It is a split, not a new layer. `raisin-mcp` serves tools, so it depends on
`raisin-functions` — which depends on `raisin-rocksdb`:

```
raisin-functions → raisin-rocksdb → raisin-mcp → raisin-functions   ✗ cycle
```

Anything at or below the storage layer (the tool-discovery job handler, for
instance) therefore could not use MCP at all. Cargo rejects package cycles
outright, and feature-gating does not help — the cycle is detected at the
package level regardless of which features are enabled.

Splitting the shared types and the client into a crate with **no
`raisin-functions` dependency** breaks it:

```
raisin-rocksdb → raisin-mcp-protocol                                ✓
raisin-mcp     → raisin-mcp-protocol  (re-exports it)               ✓
```

Because `raisin-mcp` re-exports this crate, every existing
`raisin_mcp::protocol::…` / `raisin_mcp::client::…` path still resolves. Nothing
is duplicated — the alternative (a second copy of the JSON-RPC envelopes for the
client) is exactly the mirrored-code-path failure mode this codebase keeps
getting bitten by.

## Usage

```rust,ignore
use std::time::Duration;
use raisin_mcp_protocol::client::{McpClientSession, StreamableHttpTransport};
use url::Url;

let transport = StreamableHttpTransport::new(
    http_client,                                  // process-wide reqwest::Client
    Url::parse("https://mcp.example.com/mcp")?,
    Duration::from_secs(30),
);
let session = McpClientSession::new(transport);

let auth = vec![("Authorization".to_string(), format!("Bearer {token}"))];
let tools = session.list_tools(&auth).await?;          // follows pagination
let result = session.call_tool("search_issues", args, &auth).await?;
```

## Client design notes

Four decisions that are load-bearing rather than incidental:

- **Streamable HTTP only.** No stdio transport: spawning a subprocess inside the
  database would mean arbitrary process execution and node-local state that does
  not replicate.
- **`initialize` first, `server/discover` as fallback.** The 2026-07-28 revision
  replaced the handshake, but essentially every server deployed today predates
  it — leading with the new one fails against real servers on the first message.
- **`tools/call` is never retried automatically.** MCP has no idempotency key, so
  a blind retry can charge a card or file a ticket twice. Only session recovery
  replays, and only once. `tools/list` is idempotent and may be retried freely.
- **Every response type tolerates the legacy shape.** `resultType` / `ttlMs` /
  `cacheScope` are 2026-07-28 additions; requiring them means being unable to
  parse any currently-deployed server. `ContentBlock` likewise keeps unknown
  block types verbatim instead of dropping them.

## Session cache

`shared_session_cache()` keeps negotiated sessions so a tool call costs one
round trip rather than two — an agent turn calling three tools would otherwise
pay six. The key is `(tenant, repo, branch, slug, url)`:

- **tenant/repo/branch** because a slug-only key would serve one tenant's
  session to another. That is an isolation boundary, not a tuning detail.
- **url** so editing a connection's endpoint misses rather than keeps dialling
  the old host — no invalidation hook to forget to call.

Credentials are deliberately *not* in the key: they travel per-request as
headers and are never stored on a session, so rotating one cannot leave a stale
token cached.

Consumers must register `clear()` with `raisin-core`'s derived-cache registry
(`raisin-functions` does this on first use). Replication checkpoint ingestion
copies column families straight into the live database and emits no events, so
an unregistered cache would keep serving pre-checkpoint state.

## Egress

`client::EgressPolicy` is the SSRF guard for connection URLs, configured by the
operator through the server's `[mcp_client]` TOML section. It requires `https`
(except loopback, behind an explicit flag) and rejects RFC1918, loopback,
link-local — which covers cloud instance metadata at `169.254.169.254` — CGNAT
and IPv6 ULA addresses. It is checked when a connection is saved **and** again
against the resolved addresses before dialling, because a hostname that resolved
publicly at save time can be re-pointed afterwards.

## Feature flags

| Feature  | Default | Effect                                              |
|----------|---------|-----------------------------------------------------|
| `client` | on      | The outbound client (`reqwest`, `url`, `sha2`)      |

Disabling `client` leaves the wire types alone, for a consumer that only needs
to parse or emit MCP messages.
