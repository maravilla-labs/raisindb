import { api } from './client'

/**
 * What this SERVER can do with a binary asset.
 *
 * Process-wide, not repo-scoped: plugins are loaded once at startup, before any
 * function runs, and registration is append-only into a process global. The
 * answer cannot change without a restart and is identical for every tenant on
 * the box.
 */

/** One loaded plugin and the methods it services. */
export interface PluginManifestEntry {
  name: string
  /** Fully-qualified method names, e.g. "media.doc.toMarkdown". */
  methods: string[]
}

/** A plugin file the loader REFUSED, and why. */
export interface PluginRejection {
  path: string
  reason: string
}

/** Who handles a media kind here. Mirrors `media_capabilities::Provider`. */
export type CapabilityProvider =
  | { provider: 'plugin'; plugin: string; method: string }
  | { provider: 'core'; how: string }
  | { provider: 'unsupported' }

export type CapabilityRow = {
  kind: string
  mime_types: string[]
} & CapabilityProvider

export interface PluginsResponse {
  plugins: PluginManifestEntry[]
  methods: string[]
  /**
   * Non-empty here is the failure this endpoint exists for: the server is up
   * and green, and a capability the estate assumes it has is silently gone.
   * An ABI bump between server and plugin produces exactly this.
   */
  rejected: PluginRejection[]
  capabilities: CapabilityRow[]
  /**
   * False means the task planner answers "no" to everything by default rather
   * than by observation — a wiring fault, not an empty plugin directory. The
   * two are indistinguishable from the plan alone.
   */
  capability_probe_installed: boolean
}

export const pluginsApi = {
  /** GET /api/admin/management/plugins */
  list: () => api.get<PluginsResponse>('/api/admin/management/plugins'),
}
