// SPDX-License-Identifier: BSL-1.1

//! Typed client for outbound integrations + virtual mounts.
//!
//! Two kinds of call live here:
//!  1. The dedicated `/api/integrations/...` endpoints (OAuth connect/disconnect,
//!     manual mount sync). These are admin-gated on the server.
//!  2. Node CRUD for the `raisin:Integration` and `raisin:VirtualMount` config
//!     nodes, which live in the `raisin:system` workspace on the `main` branch
//!     under `/integrations/{name}` and `/mounts/{name}`.

import { api, ApiError } from './client'
import { nodesApi, type Node } from './nodes'
import { sqlApi } from './sql'

/**
 * List every node of a type in a workspace, by TYPE rather than by walking a
 * parent folder.
 *
 * Listing children of `/mounts` looked equivalent and was not: creating a node
 * under a parent path does NOT create the parent folder node, so
 * `POST .../raisin:system/mounts` returns 201 while `GET .../raisin:system/mounts/`
 * returns 404 "Parent node not found" forever after. The old helper swallowed
 * that 404 as an empty list, so a repo full of working mounts rendered "No
 * mounts yet" — invisible in the console while the engine synced them happily,
 * because the engine finds them by TYPE (`list_by_type`), which is what this
 * now mirrors. Same source of truth, no dependency on a folder that may never
 * have existed.
 */
async function listByNodeType(repo: string, workspace: string, nodeType: string): Promise<Node[]> {
  // nodeType is a compile-time constant, never user input.
  const sql = `SELECT id, name, path, properties FROM "${workspace}" WHERE node_type = '${nodeType}'`
  try {
    const res = await sqlApi.executeQuery(repo, sql, [])
    return (res.rows || []).map((r: any) => ({
      id: r.id,
      name: r.name,
      path: r.path,
      node_type: nodeType,
      properties: r.properties || {},
    })) as Node[]
  } catch (e) {
    // A fresh repo with no such nodes (or no workspace yet) is an empty list,
    // not an error worth surfacing to the page.
    if (e instanceof ApiError && e.status === 404) return []
    throw e
  }
}

/**
 * Ensure a config root folder exists before creating a child under it.
 *
 * Without this the child is created but the folder is not, which is how the
 * mounts list broke in the first place (see [[listByNodeType]]).
 */
async function ensureConfigRoot(repo: string, branch: string, workspace: string, root: string) {
  try {
    await nodesApi.getAtHead(repo, branch, workspace, root)
    return
  } catch (e) {
    if (!(e instanceof ApiError) || e.status !== 404) throw e
  }
  try {
    await nodesApi.create(repo, branch, workspace, '/', {
      name: root.replace(/^\//, ''),
      node_type: 'raisin:Folder',
      properties: { title: root.replace(/^\//, '') },
    })
  } catch {
    // Racing another tab, or a repo that disallows the write — the child create
    // below still succeeds either way, so this must never be fatal.
  }
}

/** Workspace holding all integration + mount config nodes. */
export const SYSTEM_WORKSPACE = 'raisin:system'
/** Branch holding integration config (server uses `main` unconditionally). */
export const CONFIG_BRANCH = 'main'
const INTEGRATIONS_ROOT = '/integrations'
const MOUNTS_ROOT = '/mounts'

// ============================================================
// Types
// ============================================================

/** OAuth client configuration stored on an integration (secret excluded). */
export interface OAuthConfig {
  client_id?: string
  auth_url?: string
  token_url?: string
  revoke_url?: string
  redirect_uri?: string
  scopes?: string[]
  access_type?: string
  prompt?: string
}

/** A connected account entry. Tokens are never echoed back in plaintext. */
export interface ConnectedAccount {
  id: string
  label?: string
  subject?: string
  provider_type?: string
  /** Unix seconds when the access token expires. */
  expires_at?: number
}

/**
 * Adapter capability report (§3.3 of the virtual-node-adapter contract).
 *
 * The sync engine caches this on the connector node after a `capabilities`
 * probe so the admin console can drive its forms without invoking the adapter.
 * All flags absent (`undefined`) means "never probed" — treat as unknown and
 * fall back to the conservative (read-only, full-sync) shape.
 */
export interface Capabilities {
  can_read?: boolean
  can_write?: boolean
  can_create_folders?: boolean
  /** true = real delta API (get_changes); false = full-listing reconcile only. */
  supports_changes?: boolean
  supports_webhooks?: boolean
  supports_search?: boolean
  supports_push?: boolean
  /**
   * Adapter implements the optional `browse` operation, so the mount editor can
   * offer pickers instead of free-text ids. Absent/false is a normal state, not
   * an error — the editor keeps manual entry either way.
   */
  supports_browse?: boolean
  /** Suggested TTL (seconds) for ephemeral nodes. */
  default_ttl?: number | null
  /** Bytes; the engine skips larger items. */
  max_file_size?: number | null
}

export interface Integration {
  /** Node id (present on loaded nodes). */
  id?: string
  /** Node path within `raisin:system`, e.g. `/integrations/google-drive`. */
  path?: string
  /** Node name (last path segment). */
  name: string
  title: string
  provider_type: string
  adapter_function: string
  oauth_config?: OAuthConfig
  api_config?: Record<string, unknown>
  enabled: boolean
  connected_accounts?: ConnectedAccount[]
  /** True when a client secret has been stored (never the value itself). */
  client_secret_set?: boolean
  /**
   * Engine-cached capability report. Absent until a connection test runs.
   * Read-only in the UI — the engine owns it, the console never writes it.
   */
  capabilities?: Capabilities
  /** ISO 8601 timestamp of the last capabilities probe (engine-owned). */
  capabilities_checked_at?: string
  /**
   * Markdown describing the external-side setup steps (create an OAuth client,
   * register the redirect URI, enable the provider API, etc.). Authored on the
   * connector template so user-authored connectors ship their own instructions.
   */
  setup_instructions?: string
  /** Optional link to the provider's setup / OAuth docs. */
  docs_url?: string
  /**
   * NodeType declaring this connector's connector-level config fields. When set,
   * the editor renders it dynamically instead of relying on hardcoded fields.
   */
  config_type?: string
  /** NodeType declaring the per-connection config fields ("Add connection"). */
  connection_config_type?: string
  /** Connector-level non-secret config values, validated against `config_type`. */
  config?: Record<string, unknown>
  /** Names only of the connector-level secrets that are stored. */
  config_secret_fields?: string[]
  /**
   * The node's full property bag exactly as read from the server.
   *
   * `PUT /api/repository/...` **replaces** the whole property map
   * (`NodeUpdateBuilder::save` does `node.properties = props`), so a save that
   * omits a key deletes it. Rebuilding properties from the typed fields above
   * therefore used to drop `connected_accounts`, `client_secret_encrypted`,
   * `capabilities` and `api_config` on every edit. We keep the raw bag on read
   * and merge the edited fields over it on write.
   *
   * Internal — never sent as-is, never rendered.
   */
  _raw?: Record<string, unknown>
}

/** Calendar sync window (days relative to now). Read by calendar connectors. */
export interface SyncWindow {
  days_ahead?: number
  days_back?: number
}

export interface SyncConfig {
  mode?: 'poll' | 'webhook' | 'hybrid'
  interval_seconds?: number
  include_patterns?: string[]
  exclude_patterns?: string[]
  ephemeral?: boolean
  ttl_seconds?: number | null
  max_items_per_sync?: number
  /**
   * Folder hierarchy for materialized items, as a template over the item's
   * fields — e.g. `{conversation_id}/{name}` groups mail one folder per thread,
   * `{date:%Y}/{date:%m}/{name}` by year and month. Empty keeps the flat
   * provider path. An unresolvable placeholder falls back to flat rather than
   * producing a half-built path.
   */
  path_template?: string
  /**
   * Let a full reconcile delete the whole mount subtree when the provider
   * returns zero items. Off by default: an empty listing is more often a
   * hiccup or a permissions change than a genuinely emptied folder, and
   * deleted content is not recoverable.
   */
  allow_empty_reconcile?: boolean
  /**
   * ms-graph connector: which resource to materialize. Absent/`mail` syncs the
   * mailbox; `calendar` syncs events; `files` syncs a drive (OneDrive or a
   * SharePoint document library — see `drive_scope`). Ignored by other
   * connectors.
   */
  resource?: 'mail' | 'calendar' | 'files'
  /**
   * ms-graph connector: WHOSE mailbox / calendar / OneDrive to read. Absent
   * means the connected account's own (`/me`). An address makes this a SHARED
   * MAILBOX mount, which additionally requires the `.Shared` delegated scopes
   * on the app registration and Full Access in Exchange.
   *
   * May also be set once on the connection; the mount value wins.
   */
  principal?: string
  /**
   * ms-graph connector, `resource: 'files'` only: which drive to read.
   * `me` = the connected account's OneDrive, `user` = the OneDrive of
   * `principal`, `site` = a SharePoint document library (`site_id` required).
   * Absent is inferred — a `site_id` implies `site`, a `principal` implies
   * `user`, otherwise `me`.
   */
  drive_scope?: 'me' | 'user' | 'site'
  /** ms-graph: composite SharePoint site id, `hostname,siteGuid,webGuid`. */
  site_id?: string
  /** ms-graph: document library within the site. Blank = the default library. */
  drive_id?: string
  /**
   * Calendar connectors (google-calendar, ms-graph with `resource: 'calendar'`):
   * the time window of events to sync, in days relative to now.
   */
  window?: SyncWindow
  /**
   * Provider-specific sync settings that the typed fields above don't cover are
   * preserved verbatim on round-trip (e.g. `mailbox`, `username`, `auth`).
   */
  [key: string]: unknown
}

export interface WriteConfig {
  writeback?: 'off' | 'write_through'
  conflict?: 'remote_wins' | 'error'
}

/** Engine-managed mount state (read-only in the UI). */
export interface MountState {
  status?: string
  last_sync_token?: string
  last_sync_at?: string
  /**
   * Last sync ATTEMPT, successful or not. The scheduler backs off from this,
   * not from `last_sync_at`, so it is what tells you when a failing mount will
   * next be tried.
   */
  last_attempt_at?: string
  last_error?: string
  consecutive_failures?: number
  /** Items materialized so far by the current full walk / backfill / remap. */
  backfill_items_done?: number
  /** True once a full walk has reached the end at least once. */
  backfill_complete?: boolean
  /** Present while a walk is mid-flight (resume point for the next chunk). */
  backfill_cursor?: string
  /**
   * False when the mount requested write_through but the engine cannot honour
   * it. v1 has no write-through implementation, so the engine sets this false
   * whenever writeback is requested; the UI reads it to keep the write section
   * hidden and explain the limitation.
   */
  writeback_supported?: boolean
  /**
   * Push (webhook) subscription lifecycle state, engine-managed. Populated only
   * for mounts whose connector supports push and whose `sync_config.mode` is
   * `webhook` or `hybrid`. `push_status` is a free-form status string
   * (e.g. `active`, `pending`, `failed`); `push_expires_at` is the ISO 8601
   * subscription expiry the renewal job tracks; `push_last_error` is the last
   * subscribe/renew failure, if any. All read-only in the UI.
   */
  push_status?: string
  push_expires_at?: string
  push_last_error?: string
}

export interface VirtualMount {
  id?: string
  path?: string
  name: string
  title: string
  integration_ref: string
  account_ref?: string
  target_workspace: string
  /**
   * Branch the mount materializes into. The mount config itself always lives on
   * the repo config branch; this only selects where synced virtual nodes land.
   */
  target_branch: string
  mount_path: string
  remote_root?: string
  adapter_function?: string
  mapping_function?: string
  sync_config?: SyncConfig
  write_config?: WriteConfig
  state?: MountState
  enabled: boolean
  /**
   * The node's full property bag as read from the server. See `Integration._raw`
   * — a save replaces the whole map, so this is what keeps the engine-owned
   * `state` alive across an edit. Internal; never rendered.
   */
  _raw?: Record<string, unknown>
}

/** Adapter package discovered via the packages catalogue. */
export interface AdapterPackage {
  name: string
  title?: string
  version?: string
  description?: string
  installed?: boolean
}

// ============================================================
// Node <-> model mapping
// ============================================================

function nodeToIntegration(node: Node): Integration {
  const p = node.properties || {}
  const encrypted = p.client_secret_encrypted
  return {
    id: node.id,
    path: node.path,
    name: node.name,
    title: (p.title as string) || node.name,
    provider_type: (p.provider_type as string) || 'custom',
    adapter_function: (p.adapter_function as string) || '',
    oauth_config: (p.oauth_config as OAuthConfig) || {},
    api_config: (p.api_config as Record<string, unknown>) || undefined,
    enabled: p.enabled !== false,
    connected_accounts: (p.connected_accounts as ConnectedAccount[]) || [],
    client_secret_set: typeof encrypted === 'string' && encrypted.length > 0,
    capabilities: (p.capabilities as Capabilities) || undefined,
    capabilities_checked_at: (p.capabilities_checked_at as string) || undefined,
    setup_instructions: (p.setup_instructions as string) || undefined,
    docs_url: (p.docs_url as string) || undefined,
    config_type: (p.config_type as string) || undefined,
    connection_config_type: (p.connection_config_type as string) || undefined,
    config: (p.config as Record<string, unknown>) || undefined,
    config_secret_fields: (p.config_secret_fields as string[]) || [],
    _raw: p,
  }
}

function integrationToProperties(i: Integration): Record<string, unknown> {
  // Never include the plaintext client secret here — it is written only via the
  // dedicated encrypting endpoint (setClientSecret).
  //
  // `_raw` first: the PUT replaces the whole property map, so anything omitted
  // is deleted. Spreading the server's own bag underneath preserves engine- and
  // server-owned properties (connected_accounts, client_secret_encrypted,
  // capabilities, capabilities_checked_at, api_config) plus anything a future
  // connector adds, while the typed fields below still win.
  return {
    ...(i._raw || {}),
    title: i.title,
    provider_type: i.provider_type,
    adapter_function: i.adapter_function,
    oauth_config: i.oauth_config || {},
    ...(i.api_config ? { api_config: i.api_config } : {}),
    // Schema-driven connector config. Secrets never travel here — they go
    // through setConfigSecrets, which encrypts them server-side.
    ...(i.config ? { config: i.config } : {}),
    enabled: i.enabled,
    // Round-trip the author-supplied provider setup docs so editing a connector
    // in the UI never drops instructions shipped by its template.
    ...(i.setup_instructions ? { setup_instructions: i.setup_instructions } : {}),
    ...(i.docs_url ? { docs_url: i.docs_url } : {}),
  }
}

function nodeToMount(node: Node): VirtualMount {
  const p = node.properties || {}
  return {
    id: node.id,
    path: node.path,
    name: node.name,
    title: (p.title as string) || node.name,
    integration_ref: (p.integration_ref as string) || '',
    account_ref: (p.account_ref as string) || undefined,
    target_workspace: (p.target_workspace as string) || '',
    target_branch: (p.target_branch as string) || 'main',
    mount_path: (p.mount_path as string) || '/',
    remote_root: (p.remote_root as string) || undefined,
    adapter_function: (p.adapter_function as string) || undefined,
    mapping_function: (p.mapping_function as string) || undefined,
    sync_config: (p.sync_config as SyncConfig) || {},
    write_config: (p.write_config as WriteConfig) || {},
    state: (p.state as MountState) || {},
    enabled: p.enabled !== false,
    _raw: p,
  }
}

function mountToProperties(m: VirtualMount): Record<string, unknown> {
  // `state` is engine-owned and the UI never *edits* it — but the PUT replaces
  // the whole property map, so omitting it DELETES it, losing the sync cursor
  // (`last_sync_token`) and orphaning the push subscription
  // (`push_subscription_id` / `push_mount_token`) on every mount edit. Spread
  // the server's own bag underneath so it survives untouched.
  return {
    ...(m._raw || {}),
    title: m.title,
    integration_ref: m.integration_ref,
    ...(m.account_ref ? { account_ref: m.account_ref } : {}),
    target_workspace: m.target_workspace,
    target_branch: m.target_branch || 'main',
    mount_path: m.mount_path,
    ...(m.remote_root ? { remote_root: m.remote_root } : {}),
    ...(m.adapter_function ? { adapter_function: m.adapter_function } : {}),
    ...(m.mapping_function ? { mapping_function: m.mapping_function } : {}),
    sync_config: m.sync_config || {},
    write_config: m.write_config || {},
    enabled: m.enabled,
  }
}

// ============================================================
// OAuth + sync endpoints (dedicated /api/integrations/...)
// ============================================================

export interface OAuthStartResponse {
  auth_url: string
  state: string
}

/**
 * Provider-side setup URLs the operator must register, resolved by the server
 * from `RAISINDB_BASE_URL`.
 *
 * - `oauth_redirect_uri` — always present; register it as the redirect / reply
 *   URL in the provider's OAuth client.
 * - `notification_url` / `mount_token` — present only on the per-mount variant
 *   (null on the connector-level call). The URL is what a push provider (e.g.
 *   Gmail Pub/Sub) must POST notifications to.
 * - `base_url_configured` — false when `RAISINDB_BASE_URL` is unset; URLs then
 *   carry a literal `{base}` placeholder that must be substituted server-side.
 */
export interface SetupUrls {
  oauth_redirect_uri: string
  notification_url: string | null
  mount_token: string | null
  base_url_configured: boolean
}

export interface SyncResponse {
  job_id: string | null
  status: string
}

/** Request body for `POST /api/integrations/{repo}/test`. */
export interface TestConnectionRequest {
  integration_path: string
  account_id?: string
  remote_root?: string
  /**
   * Mount `sync_config` keys to forward into the probe — notably ms-graph's
   * `resource`, without which the probe always tests the mailbox and a calendar
   * or OneDrive mount can never be verified. The server layers these *under*
   * its own probe limits, so this cannot widen the probe.
   */
  sync_config?: SyncConfig
}

/**
 * The three ordered stages of a connection test — a presentation-only view.
 *
 * The server does NOT send stages; it reports `auth`, `probe` and `error`
 * (`handlers/integrations/test_connection/mod.rs`). Derive these with
 * {@link deriveStages} rather than reading them off the response: the previous
 * type declared a `stages` field that never existed, so every stage rendered as
 * a red cross even on a successful test.
 */
export interface TestConnectionStages {
  adapter_loaded: boolean
  auth_valid: boolean
  listing_succeeded: boolean
}

/** Result of the bounded probe `list`. Item names only — never URLs or tokens. */
export interface TestConnectionProbe {
  items_seen: number
  sample: string[]
}

/**
 * Diagnostic report returned by the connection test. Mirrors the engine's
 * `capabilities` probe + bounded probe `list`; the engine also refreshes the
 * cached `capabilities` on the connector node as a side effect.
 *
 * This mirrors the server's `TestConnectionResponse` exactly.
 */
export interface TestConnectionResult {
  ok: boolean
  /** Round-trip latency of the probe in milliseconds. */
  latency_ms?: number
  /**
   * Credential state: `valid`, `not_required` (adapter needs none), `missing`
   * (no account selected / no decryptable token), or `expired`.
   */
  auth?: string
  /** Freshly probed capabilities (also cached on the connector node). */
  capabilities?: Capabilities
  /** Probe result — present only when the `list` probe succeeded. */
  probe?: TestConnectionProbe
  /** Populated on failure — `code` mirrors the adapter error convention (§4). */
  error?: { code?: string; message: string }
}

/**
 * Reconstruct the three-stage breakdown from what the server actually returns.
 *
 * - `adapter_loaded` — false only for errors raised before the adapter ran.
 * - `auth_valid` — the credential was accepted, or the adapter needs none.
 * - `listing_succeeded` — the bounded probe `list` returned items.
 */
export function deriveStages(r: TestConnectionResult): TestConnectionStages {
  const code = r.error?.code
  return {
    adapter_loaded: code !== 'adapter_not_found' && code !== 'invocation_failed',
    auth_valid: r.auth === 'valid' || r.auth === 'not_required',
    listing_succeeded: r.ok && !!r.probe,
  }
}

/**
 * One connection on a connector, as projected by the connections endpoint.
 *
 * Read connections through that endpoint rather than off the raw node: the node
 * carries `tokens_encrypted` / `secrets_encrypted` ciphertext, and this shape
 * deliberately does not.
 */
/**
 * What class of remote container to list. Adapter-defined and deliberately open:
 * providers do not agree on what is selectable, so the server passes the slug
 * through verbatim. These are the kinds ms-graph implements.
 */
export type BrowseKind = 'folder' | 'calendar' | 'site' | 'drive' | 'driveItem' | 'mailbox'

export interface BrowseRequest {
  integration_path: string
  account_id?: string
  kind?: BrowseKind | string
  /** List this container's children. Absent = the top level for `kind`. */
  parent_id?: string
  /** Free-text filter, where the provider supports one. */
  query?: string
  cursor?: string
  limit?: number
  /**
   * Mount `sync_config` keys that select *what* to browse — ms-graph's
   * `principal` (whose mail folders), `drive_scope`/`site_id` (which drive).
   * Without it the picker lists the connected account's own containers while
   * the mount syncs someone else's.
   */
  sync_config?: SyncConfig
}

export interface BrowseItem {
  /** Provider id — written verbatim into the mount, never a display path. */
  id: string
  name: string
  kind: string
  /** Show an expander; browse again with `parent_id = id`. */
  has_children: boolean
  /** Optional secondary line (address, URL, item count). */
  hint?: string | null
}

export interface BrowseResult {
  ok: boolean
  /**
   * False when this connector's adapter has no `browse` operation. Fall back to
   * manual entry — this is a supported state, not a failure to report.
   */
  supported: boolean
  items: BrowseItem[]
  next_cursor?: string | null
  error?: { code: string; message: string } | null
}

export interface Connection {
  id: string
  label: string
  subject?: string
  auth_kind: 'oauth' | 'config'
  expires_at?: number
  /** Per-connection non-secret configuration. */
  config: Record<string, unknown>
  /** Names only of the secret fields that are stored. */
  secret_fields: string[]
  created_at?: string
}

export interface UpsertConnectionBody {
  label?: string
  config?: Record<string, unknown>
  /**
   * Field → value. `null` clears the field; an omitted field is left unchanged,
   * so an edit form never has to resend a secret it was not shown.
   */
  secrets?: Record<string, string | null>
}

export const integrationsApi = {
  // ---- Adapter packages ----

  /** List installed adapter packages (category = 'integrations'). */
  listAdapterPackages: async (repo: string): Promise<AdapterPackage[]> => {
    const sql = `
      SELECT name,
             properties->>'title' as title,
             properties->>'version' as version,
             properties->>'description' as description,
             properties->>'installed' as installed
      FROM "packages"
      WHERE properties->>'category'::String = 'integrations'
      ORDER BY name
    `
    try {
      const res = await sqlApi.executeQuery(repo, sql, [])
      return res.rows.map((r) => ({
        name: r.name,
        title: r.title || undefined,
        version: r.version || undefined,
        description: r.description || undefined,
        installed: r.installed === 'true' || r.installed === true,
      }))
    } catch {
      // Packages workspace may be empty on a fresh repo — treat as no adapters.
      return []
    }
  },

  // ---- Integration config CRUD ----

  listIntegrations: async (repo: string): Promise<Integration[]> => {
    const nodes = await listByNodeType(repo, SYSTEM_WORKSPACE, 'raisin:Integration')
    return nodes.map(nodeToIntegration)
  },

  getIntegration: async (repo: string, name: string): Promise<Integration> => {
    const node = await nodesApi.getAtHead(
      repo,
      CONFIG_BRANCH,
      SYSTEM_WORKSPACE,
      `${INTEGRATIONS_ROOT}/${name}`
    )
    return nodeToIntegration(node)
  },

  createIntegration: async (repo: string, i: Integration): Promise<Integration> => {
    await ensureConfigRoot(repo, CONFIG_BRANCH, SYSTEM_WORKSPACE, INTEGRATIONS_ROOT)
    const node = await nodesApi.create(repo, CONFIG_BRANCH, SYSTEM_WORKSPACE, INTEGRATIONS_ROOT, {
      name: i.name,
      node_type: 'raisin:Integration',
      properties: integrationToProperties(i),
    })
    return nodeToIntegration(node)
  },

  updateIntegration: async (repo: string, name: string, i: Integration): Promise<Integration> => {
    const node = await nodesApi.update(
      repo,
      CONFIG_BRANCH,
      SYSTEM_WORKSPACE,
      `${INTEGRATIONS_ROOT}/${name}`,
      { properties: integrationToProperties(i) }
    )
    return nodeToIntegration(node)
  },

  deleteIntegration: (repo: string, name: string) =>
    nodesApi.delete(repo, CONFIG_BRANCH, SYSTEM_WORKSPACE, `${INTEGRATIONS_ROOT}/${name}`),

  /**
   * Store (or replace) the write-only OAuth client secret for an integration.
   *
   * The server encrypts the value into `client_secret_encrypted` (AES-256-GCM)
   * and never echoes it back. Passing an empty string clears it.
   */
  setClientSecret: (repo: string, integrationPath: string, clientSecret: string) =>
    api.post<{ ok: boolean; client_secret_set: boolean }>(`/api/integrations/${repo}/oauth/client-secret`, {
      integration_path: integrationPath,
      client_secret: clientSecret,
    }),

  /**
   * Store (or clear) connector-level secrets declared by `config_type`.
   *
   * Generalizes `setClientSecret`. Values are encrypted server-side and never
   * returned; pass `null` to clear a field, and omit a field to leave it alone.
   */
  setConfigSecrets: (
    repo: string,
    integrationPath: string,
    secrets: Record<string, string | null>
  ) =>
    api.put<{ ok: boolean; secret_fields_set: string[] }>(
      `/api/integrations/${repo}/config-secrets`,
      { integration_path: integrationPath, secrets }
    ),

  // ---- Connections (per-account config + credentials) ----

  listConnections: (repo: string, integrationPath: string) =>
    api
      .get<{ connections: Connection[] }>(
        `/api/integrations/${repo}/connections?integration_path=${encodeURIComponent(
          integrationPath
        )}`
      )
      .then((r) => r.connections),

  /** Create a credential-based connection (the non-OAuth path, e.g. IMAP). */
  createConnection: (repo: string, integrationPath: string, body: UpsertConnectionBody) =>
    api.post<Connection>(`/api/integrations/${repo}/connections`, {
      integration_path: integrationPath,
      ...body,
    }),

  updateConnection: (
    repo: string,
    integrationPath: string,
    accountId: string,
    body: UpsertConnectionBody
  ) =>
    api.patch<Connection>(`/api/integrations/${repo}/connections/${accountId}`, {
      integration_path: integrationPath,
      ...body,
    }),

  deleteConnection: (repo: string, integrationPath: string, accountId: string) =>
    api.delete<{ deleted: boolean }>(
      `/api/integrations/${repo}/connections/${accountId}?integration_path=${encodeURIComponent(
        integrationPath
      )}`
    ),

  // ---- OAuth connect / disconnect ----

  oauthStart: (repo: string, integrationPath: string) =>
    api.post<OAuthStartResponse>(`/api/integrations/${repo}/oauth/start`, {
      integration_path: integrationPath,
    }),

  oauthDisconnect: (repo: string, integrationPath: string, accountId: string) =>
    api.post<{ disconnected: boolean }>(`/api/integrations/${repo}/oauth/disconnect`, {
      integration_path: integrationPath,
      account_id: accountId,
    }),

  /**
   * Fetch the provider-side setup URLs. Omit `mountId` for the connector-level
   * variant (OAuth redirect URI only); pass a mount node id for the per-mount
   * variant that also returns the push notification URL (minting its token).
   */
  getSetupUrls: (repo: string, mountId?: string) =>
    api.get<SetupUrls>(
      mountId
        ? `/api/integrations/${repo}/mounts/${mountId}/setup-urls`
        : `/api/integrations/${repo}/setup-urls`
    ),

  // ---- Mount config CRUD ----

  listMounts: async (repo: string): Promise<VirtualMount[]> => {
    const nodes = await listByNodeType(repo, SYSTEM_WORKSPACE, 'raisin:VirtualMount')
    return nodes.map(nodeToMount)
  },

  getMount: async (repo: string, name: string): Promise<VirtualMount> => {
    const node = await nodesApi.getAtHead(
      repo,
      CONFIG_BRANCH,
      SYSTEM_WORKSPACE,
      `${MOUNTS_ROOT}/${name}`
    )
    return nodeToMount(node)
  },

  createMount: async (repo: string, m: VirtualMount): Promise<VirtualMount> => {
    await ensureConfigRoot(repo, CONFIG_BRANCH, SYSTEM_WORKSPACE, MOUNTS_ROOT)
    const node = await nodesApi.create(repo, CONFIG_BRANCH, SYSTEM_WORKSPACE, MOUNTS_ROOT, {
      name: m.name,
      node_type: 'raisin:VirtualMount',
      properties: mountToProperties(m),
    })
    return nodeToMount(node)
  },

  updateMount: async (repo: string, name: string, m: VirtualMount): Promise<VirtualMount> => {
    const node = await nodesApi.update(
      repo,
      CONFIG_BRANCH,
      SYSTEM_WORKSPACE,
      `${MOUNTS_ROOT}/${name}`,
      { properties: mountToProperties(m) }
    )
    return nodeToMount(node)
  },

  deleteMount: (repo: string, name: string) =>
    nodesApi.delete(repo, CONFIG_BRANCH, SYSTEM_WORKSPACE, `${MOUNTS_ROOT}/${name}`),

  /** Enqueue a manual "sync now" for a mount. `mountId` is the mount node id. */
  syncMount: (repo: string, mountId: string, mode: 'delta' | 'full' | 'remap' = 'delta') =>
    api.post<SyncResponse>(`/api/integrations/${repo}/mounts/${mountId}/sync`, { mode }),

  /**
   * Run a synchronous connection test against a connector: the server invokes
   * the adapter's `capabilities` op plus a bounded probe `list`, caches the
   * resulting capabilities on the connector node, and returns a diagnostic
   * report (stages, latency, sample, error). Re-fetch the connector afterwards
   * to pick up the refreshed cached capabilities.
   */
  testConnection: (repo: string, req: TestConnectionRequest) =>
    api.post<TestConnectionResult>(`/api/integrations/${repo}/test`, req),

  /**
   * List what a connector can see remotely — mail folders, calendars, SharePoint
   * sites, document libraries — so the operator picks a mount's remote root
   * instead of pasting a provider id.
   *
   * A connector whose adapter has no `browse` operation answers
   * `{ supported: false }` rather than failing; callers must fall back to manual
   * entry on that, not surface it as an error.
   */
  browse: (repo: string, req: BrowseRequest) =>
    api.post<BrowseResult>(`/api/integrations/${repo}/browse`, req),
}
