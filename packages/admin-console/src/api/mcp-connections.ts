// SPDX-License-Identifier: BSL-1.1

//! Typed client for OUTBOUND MCP connections.
//!
//! RaisinDB as an MCP *client*: each connection is a remote server whose tools
//! become proxy functions agents can call. Distinct from the `mcp` workspace,
//! which is the inbound direction (RaisinDB serving its own tools).
//!
//! Unlike `integrations.ts`, everything here goes through dedicated endpoints
//! rather than node CRUD. That is deliberate: a connection URL must be checked
//! against the operator's egress policy and its slug against the path grammar
//! BEFORE the node is written, and generic node CRUD runs neither check.
//!
//! No endpoint here ever returns a credential or a token. `credentialSet` /
//! `oauthConnected` are booleans so the UI can show "is set" without ever
//! holding the secret.

import { api } from './client'

/** How a connection authenticates to its remote server. */
export type AuthKind = 'none' | 'static' | 'oauth'

/** State of one discovered tool. */
export type ToolState = 'active' | 'missing' | 'conflict'

/** Where a static credential is placed on the request. */
export interface StaticAuth {
  scheme?: 'Bearer' | { Header: { name: string } } | string
  name?: string
}

/** Which of a server's tools to expose. */
export interface ToolFilter {
  allow: string[]
  deny: string[]
}

/** When discovery runs. */
export interface RefreshPolicy {
  mode: 'manual' | 'interval'
  interval_secs: number
  on_save: boolean
  /**
   * Hold a notification stream open so tool changes arrive live.
   *
   * Off by default: a listener is a socket held open for hours against a third
   * party. The server must also promise the guarantee (2026-07-28, or
   * `tools.listChanged`), so enabling this does not always produce a stream.
   */
  notifications: boolean
  call_timeout_ms: number
}

/** Last probe result recorded on a connection. */
export interface ConnectionHealth {
  status?: 'ok' | 'auth_expired' | 'unreachable' | 'config_error' | 'rate_limited'
  checked_at?: string
  server_info?: { name?: string; version?: string }
  tool_count?: number
  error_code?: string
  error?: string
}

/** A connection as the API returns it — always redacted. */
export interface McpConnection {
  title: string
  slug: string
  url: string
  enabled: boolean
  protocol_version?: string | null
  auth_kind: AuthKind
  static_auth?: StaticAuth
  /** Whether a static credential is stored. Never the value itself. */
  credential_set: boolean
  /** Whether OAuth tokens are stored. Never the tokens themselves. */
  oauth_connected: boolean
  oauth_client?: Record<string, unknown> | null
  expires_at?: number | null
  tool_filter: ToolFilter
  refresh_policy: RefreshPolicy
  tool_count: number
  /** Present instead of the rest when the node could not be parsed. */
  invalid?: boolean
  error?: string
  path?: string
}

/** One tool discovered on a remote server. */
export interface DiscoveredTool {
  remote_name: string
  function_name: string
  function_path: string
  schema_hash: string
  enabled: boolean
  state: ToolState
}

/** Structured result of a live probe. Never an HTTP error — see `reachable`. */
export interface TestReport {
  reachable: boolean
  protocol_version?: string
  server_info?: { name?: string; version?: string }
  capabilities?: Record<string, unknown>
  instructions?: string
  tool_count: number
  tools: string[]
  permitted_tool_count: number
  error_code?: string
  error?: string
}

/** What `oauth/discover` learned from the server's 401. */
export interface DiscoverResult {
  requires_auth: boolean
  discovered?: boolean
  issuer?: string
  authorization_endpoint?: string
  token_endpoint?: string
  supports_dynamic_registration?: boolean
  registered?: boolean
  client_id?: string
  scopes?: string[]
  redirect_uri?: string
  status?: number
  message?: string
  /** Which auth method the server's metadata says it wants at the token endpoint. */
  auth_method?: string
  /** The server demands client authentication — a public client will be rejected. */
  client_secret_required?: boolean
  /** A secret is already stored for this connection. */
  client_secret_set?: boolean
  /**
   * The server requires a secret and none is stored. Connecting WILL fail at
   * the token exchange, AFTER consent — so this has to be surfaced before then.
   */
  needs_manual_client_secret?: boolean
}

/** Fields accepted when creating a connection. */
export interface CreateConnectionInput {
  slug: string
  url: string
  title?: string
  enabled?: boolean
  auth_kind?: AuthKind
  static_auth?: Record<string, unknown>
  tool_filter?: ToolFilter
  refresh_policy?: Partial<RefreshPolicy>
}

/** Fields accepted when updating. Omitted fields are left untouched. */
export type UpdateConnectionInput = Partial<Omit<CreateConnectionInput, 'slug'>>

const base = (repo: string) => `/api/mcp-connections/${encodeURIComponent(repo)}`
const one = (repo: string, slug: string) => `${base(repo)}/${encodeURIComponent(slug)}`

export const mcpConnectionsApi = {
  async list(repo: string): Promise<McpConnection[]> {
    const res = await api.get<{ connections: McpConnection[] }>(base(repo))
    return res.connections || []
  },

  get(repo: string, slug: string): Promise<McpConnection> {
    return api.get<McpConnection>(one(repo, slug))
  },

  create(repo: string, input: CreateConnectionInput): Promise<McpConnection> {
    return api.post<McpConnection>(base(repo), input)
  },

  update(repo: string, slug: string, input: UpdateConnectionInput): Promise<McpConnection> {
    return api.patch<McpConnection>(one(repo, slug), input)
  },

  /**
   * Delete a connection. Refused with 409 while proxies still exist unless
   * `force` — agents may reference them, and a silent cascade would break those
   * agents with no indication why.
   */
  remove(repo: string, slug: string, force = false): Promise<{ ok: boolean }> {
    return api.delete<{ ok: boolean }>(`${one(repo, slug)}${force ? '?force=true' : ''}`)
  },

  /** Write-only. There is no read counterpart by design. */
  setCredential(
    repo: string,
    slug: string,
    value: string,
    staticAuth?: Record<string, unknown>,
  ): Promise<{ ok: boolean; credential_set: boolean; auth_kind: AuthKind }> {
    return api.put(`${one(repo, slug)}/credential`, { value, static_auth: staticAuth })
  },

  clearCredential(
    repo: string,
    slug: string,
  ): Promise<{ ok: boolean; credential_set: boolean; auth_kind: AuthKind }> {
    return api.delete(`${one(repo, slug)}/credential`)
  },

  /** Live probe. Answers 200 with a report even when the connection is broken. */
  test(repo: string, slug: string): Promise<TestReport> {
    return api.post<TestReport>(`${one(repo, slug)}/test`, {})
  },

  /** Enqueue a discovery run; returns the job id. */
  refreshTools(repo: string, slug: string): Promise<{ ok: boolean; job_id: string }> {
    return api.post(`${one(repo, slug)}/refresh-tools`, {})
  },

  listTools(
    repo: string,
    slug: string,
  ): Promise<{ tools: DiscoveredTool[]; tool_filter: ToolFilter; health?: ConnectionHealth }> {
    return api.get(`${one(repo, slug)}/tools`)
  },

  /**
   * Enable or disable one tool. Records intent on `tool_filter` and enqueues a
   * discovery run; the proxy nodes are only ever written by that job.
   */
  setToolEnabled(
    repo: string,
    slug: string,
    remoteName: string,
    enabled: boolean,
  ): Promise<{ ok: boolean; refresh_job_id: string }> {
    return api.patch(`${one(repo, slug)}/tools/${encodeURIComponent(remoteName)}`, { enabled })
  },

  /**
   * Delete every proxy whose tool is gone upstream.
   *
   * Refused with 409 naming the agents that still reference the paths unless
   * `force`. Discovery never deletes — it disables — so this is the only way a
   * `missing` entry goes away.
   */
  pruneTools(
    repo: string,
    slug: string,
    force = false,
  ): Promise<{ ok: boolean; pruned: number; tools: string[] }> {
    return api.post(`${one(repo, slug)}/prune-tools${force ? '?force=true' : ''}`, {})
  },

  /** Delete one proxy, whatever its state. Same 409 contract as prune. */
  deleteTool(
    repo: string,
    slug: string,
    remoteName: string,
    force = false,
  ): Promise<{ ok: boolean; pruned: number; tools: string[] }> {
    return api.delete(
      `${one(repo, slug)}/tools/${encodeURIComponent(remoteName)}${force ? '?force=true' : ''}`,
    )
  },

  /** Probe the 401, follow RFC 9728 → RFC 8414, and register dynamically. */
  oauthDiscover(repo: string, slug: string): Promise<DiscoverResult> {
    return api.post<DiscoverResult>(`${one(repo, slug)}/oauth/discover`, {})
  },

  oauthStart(repo: string, slug: string): Promise<{ auth_url: string; state: string }> {
    return api.post(`${one(repo, slug)}/oauth/start`, {})
  },

  /**
   * Supply an OAuth client by hand.
   *
   * For servers with no dynamic registration, and for ones that demand a client
   * secret registration did not issue — Entra answers `AADSTS7000218` in that
   * case. Write-only: the secret is never returned by any read.
   */
  setOauthClient(
    repo: string,
    slug: string,
    body: {
      client_id: string
      client_secret?: string
      issuer?: string
      authorization_endpoint?: string
      token_endpoint?: string
      scopes?: string[]
    },
  ): Promise<{ ok: boolean; client_id_set: boolean; client_secret_set: boolean }> {
    return api.put(`${one(repo, slug)}/oauth/client`, body)
  },

  clearOauthClient(repo: string, slug: string): Promise<{ ok: boolean }> {
    return api.delete(`${one(repo, slug)}/oauth/client`)
  },

  oauthDisconnect(repo: string, slug: string): Promise<{ ok: boolean }> {
    return api.post(`${one(repo, slug)}/oauth/disconnect`, {})
  },
}

/**
 * Open the authorize URL in a popup and resolve when the callback relays back.
 *
 * The callback page posts a `raisin-oauth-result` message and closes itself.
 * The interval is the fallback for a user who closes the popup by hand — without
 * it the promise would never settle and the button would spin forever.
 */
export function runOAuthPopup(authUrl: string): Promise<{ connected?: string; error?: string }> {
  return new Promise((resolve) => {
    const popup = window.open(authUrl, 'raisin-mcp-oauth', 'width=560,height=760')
    let settled = false

    const finish = (result: { connected?: string; error?: string }) => {
      if (settled) return
      settled = true
      window.removeEventListener('message', onMessage)
      clearInterval(poll)
      resolve(result)
    }

    const onMessage = (event: MessageEvent) => {
      // Same-origin only: the callback is served by this server.
      if (event.origin !== window.location.origin) return
      if (event.data?.type !== 'raisin-oauth-result') return
      finish({ connected: event.data.connected, error: event.data.error })
    }

    window.addEventListener('message', onMessage)
    const poll = setInterval(() => {
      if (popup?.closed) finish({ error: 'Authorization window was closed' })
    }, 700)
  })
}
