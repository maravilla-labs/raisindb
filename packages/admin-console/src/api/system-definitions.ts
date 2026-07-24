import { api } from './client'

/**
 * The system-definition stack: where this server's built-in NodeTypes,
 * Workspaces and packages come from.
 *
 * Built-ins are compiled into the binary, but an on-disk overlay (and,
 * optionally, a remote registry cached into that overlay) can override any of
 * them by name — which is how a schema fix ships without a new server release.
 * These endpoints change what the server *offers*; applying it to a repository
 * is still the system-updates flow.
 */

/** Where one resolved definition comes from */
export interface DefinitionOriginInfo {
  /** Resource name, e.g. `raisin:Package` */
  name: string
  /** Winning layer: `embedded`, `overlay`, or a registry name */
  layer: string
  /** Layers this definition shadows, lowest first */
  shadowed: string[]
}

/** The server's current definition stack */
export interface SystemDefinitionsResponse {
  /** Layer names, lowest precedence first */
  layers: string[]
  /** Resolved overlay directory (may not exist) */
  overlay_dir: string
  /** Whether that directory currently exists */
  overlay_present: boolean
  /** Startup auto-apply policy: `Off`, `NonBreaking` or `All` */
  auto_apply: string
  /** Every resolved definition and its winning layer */
  definitions: DefinitionOriginInfo[]
}

/** A configured registry (credentials are never returned) */
export interface RegistryInfo {
  name: string
  url: string
  enabled: boolean
}

/** One artifact offered by a registry */
export interface RegistryEntry {
  name: string
  kind: 'node_type' | 'workspace' | 'package'
  version: string | null
  sha256: string
  url: string
  description: string | null
}

/** Result of a registry fetch */
export interface FetchResponse {
  /** Artifacts written into the overlay */
  fetched: string[]
  message: string
}

const BASE = '/api/management/system-definitions'

export const systemDefinitionsApi = {
  /** Current layers and which layer each definition resolves from */
  get: () => api.get<SystemDefinitionsResponse>(BASE),

  /**
   * Re-read the overlay directory. Writes nothing to any repository — changed
   * definitions then show up as pending system updates per repo.
   */
  reload: () => api.post<SystemDefinitionsResponse>(`${BASE}/reload`, {}),

  /** Configured registries (all of them, enabled or not) */
  listRegistries: () => api.get<RegistryInfo[]>(`${BASE}/registries`),

  /** Fetch a registry's catalog. Only works for an enabled registry. */
  getCatalog: (name: string) =>
    api.get<RegistryEntry[]>(`${BASE}/registries/${encodeURIComponent(name)}`),

  /**
   * Download artifacts into the overlay, verifying each declared SHA256.
   *
   * @param resources - artifact names; empty means the whole catalog
   */
  fetch: (name: string, resources: string[] = []) =>
    api.post<FetchResponse>(
      `${BASE}/registries/${encodeURIComponent(name)}/fetch`,
      { resources }
    ),
}
