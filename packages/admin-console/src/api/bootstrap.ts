// SPDX-License-Identifier: BSL-1.1

/**
 * Bootstrap module
 *
 * The admin SPA is multi-tenant but the SPA itself is identical for every
 * tenant — the server decides which tenant a request belongs to via the
 * `x-tenant-id` header (injected by an edge proxy like Caddy, or defaulted
 * to "default" by the server middleware for local single-operator dev).
 *
 * On boot the SPA calls `/api/admin/bootstrap` once. The server reflects
 * the resolved tenant + server version + dev-mode flag back. We cache the
 * values in a module-level singleton so that:
 *   - React components can read them via AuthContext (re-renders on change)
 *   - The non-React API client can read them synchronously when building
 *     request headers (no need to thread context through every helper)
 *
 * The cache is seeded from localStorage so the very first requests after
 * a hard reload already carry a plausible `x-tenant-id`; the bootstrap
 * fetch then overwrites with the authoritative value from the server.
 */

const TENANT_STORAGE_KEY = 'raisindb_tenant_id'
const VERSION_STORAGE_KEY = 'raisindb_server_version'
export const AUTH_TOKEN_STORAGE_KEY = 'raisindb_auth_token'

export interface BootstrapResponse {
  tenant_id: string
  version: string
  /** True when the server detects a dev-mode build (single-operator local dev). */
  dev_mode?: boolean
  /**
   * Superadmin token, only ever returned when `dev_mode === true`.
   * In production the server never includes this field.
   */
  superadmin_token?: string | null
}

let cachedTenantId: string = (() => {
  try {
    return localStorage.getItem(TENANT_STORAGE_KEY) || 'default'
  } catch {
    return 'default'
  }
})()

let cachedServerVersion: string = (() => {
  try {
    return localStorage.getItem(VERSION_STORAGE_KEY) || '0.1.0'
  } catch {
    return '0.1.0'
  }
})()

let cachedDevMode: boolean = false
let cachedSuperadminToken: string | null = null

// Dev-only build-time fallback: when the server doesn't return a superadmin
// token in the bootstrap response, allow Vite env to supply one for local dev.
const BUILD_TIME_SUPERADMIN_TOKEN: string | null =
  (import.meta.env.VITE_RAISIN_SUPERADMIN_TOKEN as string | undefined) || null

export function getCurrentTenantId(): string {
  return cachedTenantId
}

export function getCurrentServerVersion(): string {
  return cachedServerVersion
}

export function getCurrentDevMode(): boolean {
  return cachedDevMode
}

export function getCurrentSuperadminToken(): string | null {
  return cachedSuperadminToken || BUILD_TIME_SUPERADMIN_TOKEN
}

/**
 * Synchronous accessor for the auth token. Pre-bootstrap and outside React
 * we read directly from localStorage; the client and AuthContext share the
 * same storage key via this helper so renames stay in one place.
 */
export function getCurrentAuthToken(): string | null {
  try {
    return localStorage.getItem(AUTH_TOKEN_STORAGE_KEY)
  } catch {
    return null
  }
}

/**
 * Fetch the bootstrap response from the server.
 *
 * The endpoint is auth-not-required and returns the tenant resolved from
 * the request's `x-tenant-id` header (or "default" in dev). Response is
 * marked `Cache-Control: no-store` on the server because it varies per
 * request, so we never cache it on the client either.
 */
export async function fetchBootstrap(): Promise<BootstrapResponse> {
  const response = await fetch('/api/admin/bootstrap', {
    method: 'GET',
    headers: { Accept: 'application/json' },
    cache: 'no-store',
    credentials: 'same-origin',
  })

  if (!response.ok) {
    throw new Error(`Bootstrap failed: ${response.status} ${response.statusText}`)
  }

  const data = (await response.json()) as BootstrapResponse

  const nextTenantId = data.tenant_id || 'default'
  const nextServerVersion = data.version || cachedServerVersion

  cachedTenantId = nextTenantId
  cachedServerVersion = nextServerVersion
  cachedDevMode = data.dev_mode === true
  cachedSuperadminToken = data.superadmin_token ?? null

  // Only write back to localStorage when something actually changed; cuts
  // unnecessary storage events that other tabs would otherwise see.
  try {
    if (localStorage.getItem(TENANT_STORAGE_KEY) !== nextTenantId) {
      localStorage.setItem(TENANT_STORAGE_KEY, nextTenantId)
    }
    if (localStorage.getItem(VERSION_STORAGE_KEY) !== nextServerVersion) {
      localStorage.setItem(VERSION_STORAGE_KEY, nextServerVersion)
    }
  } catch {
    /* localStorage unavailable — non-fatal */
  }

  return data
}
