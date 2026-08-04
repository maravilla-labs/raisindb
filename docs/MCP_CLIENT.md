# MCP Client — connecting RaisinDB to external servers

RaisinDB can act as an MCP **client**: your agents call tools that live on
somebody else's Model Context Protocol server (Linear, GitHub, an internal team
server). This is the outbound direction. For the inbound one — serving
RaisinDB's own tools to Claude and other clients — see
`book/src/architecture/mcp.md`.

## The shape

A **connection** is a `raisin:McpConnection` node describing one remote server.
Discovery calls that server's `tools/list` and writes one **proxy
`raisin:Function`** per remote tool, under `/mcp/{slug}/{tool}` in the
`functions` workspace.

An agent then references those paths in its `tools:` array exactly like a local
function:

```yaml
node_type: raisin:AIAgent
properties:
  tools:
    - /lib/raisin/ai/remember          # local
    - /mcp/linear/search-issues        # remote, via the Linear connection
```

Nothing about the agent, the permission model, or the tool-call machinery is
special-cased. A proxy is a function that happens to have no code.

## Setting one up

1. **Add the connection.** Admin console → *MCP Connections* → *Add connection*.
   Give it a title, a slug and the server's Streamable HTTP endpoint. It is
   created disabled.

   The **slug is permanent**: it is part of every generated tool path, so
   changing it later would orphan every proxy and silently break any agent
   holding one.

2. **Authenticate.** Three modes:
   - **No auth** — public servers.
   - **Token / API key** — paste it once. It is encrypted immediately and no
     endpoint ever returns it; the console shows only whether one is set.
   - **OAuth 2.1** — press *Discover*. RaisinDB probes the server, reads the
     RFC 9728 pointer out of its 401, follows it to the authorization server,
     and registers itself dynamically. Then press *Connect* and consent in the
     popup. For servers without dynamic registration, the console shows the
     redirect URI to register by hand.

3. **Enable it, then refresh tools.** Discovery runs on save (and on the
   connection's interval). The *Tools* table lists what the server offers and
   the path to paste into an agent.

4. **Expose only what you need.** Every tool has an *Exposed* toggle. Narrowing
   this is the cheapest way to bound what a remote server can be asked to do.

## Configuration

The optional `[mcp_client]` TOML section is the operator-owned half — where the
client may connect, and how much it may buffer:

```toml
[mcp_client]
# Empty = any PUBLIC host. Entries are exact ("mcp.linear.app") or a wildcard
# suffix ("*.example.com", which matches sub-domains but NOT the bare apex).
allowed_hosts = []
# Permit loopback/private addresses AND plain http. Local development only.
allow_private_addresses = false
max_response_bytes = 8388608
default_timeout_ms = 30000
```

Omitting the section keeps the safe defaults. Everything per-connection lives on
the node instead, so adding a connection never needs a restart.

## Things worth knowing before you rely on it

**A connection has ONE identity.** Every agent and every user calling one of its
tools acts as that credential. There is no per-user delegation — if two people
need different permissions on the remote server, they need two connections.

**A remote tool can see whatever the model sends it.** The model has seen the
conversation, and it chooses the arguments. Nothing structurally prevents a
remote server from being handed data you did not intend. Bound it by exposing
few tools, by the egress allowlist, and by who you let attach a proxy to an
agent.

**Egress is restricted by default.** `https` only, and private, loopback and
link-local addresses (including cloud metadata at `169.254.169.254`) are
refused. This is checked when you save a connection *and* again before every
dial — the second check resolves the hostname and judges every address it
returns, because a name that resolved publicly at save time can be re-pointed at
`127.0.0.1` afterwards. To reach an MCP server on localhost, set
`allow_private_addresses = true`.

**The policy covers the whole OAuth chain, not just the endpoint you typed.**
Every URL after the connection's own is named by the remote side: its `401`
names the metadata document, that names the issuer, and the issuer's metadata
names the registration and token endpoints. All of them are checked. If you set
`allowed_hosts`, list the **authorization server's host too** — it is usually a
different host (`auth.linear.app` vs `mcp.linear.app`), and a missing entry
makes `oauth/discover` fail with a message saying exactly that.

**Multi-node clusters need `[locks]` with the `redis` backend.** Discovery
writes shared content, and the lease is what makes it run once per cluster.
Without it every node writes the same proxy nodes.

**A tool that vanishes upstream is disabled, not deleted.** You will see it in
the table as `missing`. Deleting it would make the tool silently disappear from
any agent referencing it, with no error anywhere.

**`tools/call` is never retried automatically.** MCP has no idempotency key, so
a retry could charge a card or file a ticket twice.

## HTTP API

All admin-gated except the OAuth callback, which is a browser redirect
authenticated by its single-use `state`.

```
GET    /api/mcp-connections/{repo}                          list
POST   /api/mcp-connections/{repo}                          create
GET    /api/mcp-connections/{repo}/{slug}                   read (secrets elided)
PATCH  /api/mcp-connections/{repo}/{slug}                   update
DELETE /api/mcp-connections/{repo}/{slug}[?force=true]      delete
PUT    /api/mcp-connections/{repo}/{slug}/credential        write-only
DELETE /api/mcp-connections/{repo}/{slug}/credential        clear
POST   /api/mcp-connections/{repo}/{slug}/test              probe (200 + report)
POST   /api/mcp-connections/{repo}/{slug}/refresh-tools     enqueue discovery
GET    /api/mcp-connections/{repo}/{slug}/tools             discovered tools
PATCH  /api/mcp-connections/{repo}/{slug}/tools/{name}      { enabled }
POST   /api/mcp-connections/{repo}/{slug}/oauth/discover    401 → RFC 9728 → DCR
POST   /api/mcp-connections/{repo}/{slug}/oauth/start       → { auth_url }
POST   /api/mcp-connections/{repo}/{slug}/oauth/disconnect  clear tokens
GET    /api/mcp-connections/{repo}/oauth/callback           (public)
```

`test` always answers **200 with a structured report**, even when the connection
is completely broken — `{ reachable: false, error_code: "auth_expired" }` tells
the console what to render, where a 502 would only produce an error toast.

## Troubleshooting

| Symptom | Cause |
|---|---|
| `config_error` on save: "must use https" | Set `allow_private_addresses` for a local server, or use https. |
| `auth_expired` after working fine | The OAuth token lapsed and could not be refreshed. Press *Connect* again. |
| Tools show but calls fail unauthenticated | `RAISIN_MASTER_KEY` differs from the key the credential was stored under. Re-enter it. |
| A tool is `conflict` | Its generated function name collides with an existing function. Rename the local one, or rename the connection slug before first discovery. |
| Discovery never runs | The connection is disabled, or its refresh policy is `manual`. |
