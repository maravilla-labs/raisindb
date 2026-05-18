// SPDX-License-Identifier: BSL-1.1

import {
  AUTH_TOKEN_STORAGE_KEY,
  getCurrentAuthToken,
  getCurrentSuperadminToken,
  getCurrentTenantId,
} from './bootstrap'

export interface ApiErrorResponse {
  code: string
  message: string
  details?: string
  field?: string
  timestamp: string
}

export class ApiError extends Error {
  constructor(
    message: string,
    public status: number,
    public response?: ApiErrorResponse
  ) {
    super(message)
    this.name = 'ApiError'
  }
  
  get code(): string | undefined {
    return this.response?.code
  }
  
  get details(): string | undefined {
    return this.response?.details
  }
  
  get field(): string | undefined {
    return this.response?.field
  }
}

function isFormData(body: unknown): body is FormData {
  return typeof FormData !== 'undefined' && body instanceof FormData
}

// Storage keys for impersonated user
export const IMPERSONATE_USER_KEY = 'raisindb_impersonate_user'
export const IMPERSONATE_USER_NAME_KEY = 'raisindb_impersonate_user_name'

// Get the currently impersonated user ID (if any)
export function getImpersonatedUserId(): string | null {
  return localStorage.getItem(IMPERSONATE_USER_KEY)
}

// Get the currently impersonated user's display name (if any)
export function getImpersonatedUserName(): string | null {
  return localStorage.getItem(IMPERSONATE_USER_NAME_KEY)
}

// Set the impersonated user ID and display name
export function setImpersonatedUserId(userId: string | null, displayName?: string): void {
  if (userId) {
    localStorage.setItem(IMPERSONATE_USER_KEY, userId)
    if (displayName) {
      localStorage.setItem(IMPERSONATE_USER_NAME_KEY, displayName)
    }
  } else {
    localStorage.removeItem(IMPERSONATE_USER_KEY)
    localStorage.removeItem(IMPERSONATE_USER_NAME_KEY)
  }
}

/**
 * Should the request be augmented with the dev-mode superadmin token?
 *
 * Only `/management/admin/*` is gated by the superadmin token middleware on
 * the server. We never attach it to any other path so the token can't leak
 * to handlers that don't need it.
 */
function shouldAttachSuperadminToken(path: string): boolean {
  return path.startsWith('/management/admin/')
}

/**
 * Build the auth + tenant + impersonation headers for a request.
 *
 * When `path` starts with `/management/admin/`, the user JWT in
 * `Authorization` is replaced by the dev-mode superadmin token (admin routes
 * don't run user-JWT auth — server gates them via
 * `require_superadmin_token_middleware` reading `Authorization: Bearer`).
 *
 * Pass `path` from `requestRaw`; omit it for SSE/manual fetch helpers where
 * the call site isn't on an admin route.
 */
function buildAuthHeaders(path?: string): Record<string, string> {
  const headers: Record<string, string> = {}

  const token = getCurrentAuthToken()
  if (token) {
    headers['Authorization'] = `Bearer ${token}`
  }

  const impersonateUserId = getImpersonatedUserId()
  if (impersonateUserId) {
    headers['X-Raisin-Impersonate'] = impersonateUserId
  }

  // Always carry the resolved tenant. The middleware defaults to "default"
  // when absent, so this is also safe in single-operator dev.
  headers['x-tenant-id'] = getCurrentTenantId()

  if (path && shouldAttachSuperadminToken(path)) {
    const superadminToken = getCurrentSuperadminToken()
    if (superadminToken) {
      headers['Authorization'] = `Bearer ${superadminToken}`
    }
  }

  return headers
}

// Get auth headers for manual fetch calls (SSE, streaming, etc.)
export function getAuthHeaders(): Record<string, string> {
  return buildAuthHeaders()
}

// Export for use in cases where we need the raw Response (e.g., file downloads)
export async function requestRaw(
  path: string,
  options: RequestInit = {}
): Promise<Response> {
  const url = path.startsWith('http') ? path : `${path}`

  const headers: Record<string, string> = {
    ...buildAuthHeaders(path),
    ...(options.headers as Record<string, string>),
  }

  const hasBody = options.body !== undefined && options.body !== null
  if (hasBody && !isFormData(options.body) && !('Content-Type' in headers)) {
    headers['Content-Type'] = 'application/json'
  }

  const response = await fetch(url, {
    ...options,
    headers,
  })

  if (!response.ok) {
    // Handle 401 Unauthorized - clear auth data and redirect to login
    if (response.status === 401) {
      localStorage.removeItem(AUTH_TOKEN_STORAGE_KEY)
      localStorage.removeItem('raisindb_auth_user')
      // Redirect to login if not already there
      if (!window.location.pathname.includes('/login')) {
        window.location.href = '/admin/login'
      }
    }

    const contentType = response.headers.get('content-type')

    // Try to parse structured error response
    if (contentType?.includes('application/json')) {
      try {
        const errorData = await response.json() as ApiErrorResponse
        throw new ApiError(
          errorData.message || `Request failed: ${response.statusText}`,
          response.status,
          errorData
        )
      } catch (e) {
        if (e instanceof ApiError) throw e
      }
    }

    // Fallback to plain text error
    const text = await response.text()
    throw new ApiError(
      text || `Request failed: ${response.statusText}`,
      response.status
    )
  }

  return response
}

async function request<T>(
  path: string,
  options: RequestInit = {}
): Promise<T> {
  const response = await requestRaw(path, options)

  // Handle empty responses
  const contentType = response.headers.get('content-type')
  if (!contentType?.includes('application/json')) {
    return null as T
  }

  return response.json()
}

export const api = {
  get: <T>(path: string, options?: RequestInit) =>
    request<T>(path, { ...options, method: 'GET' }),

  post: <T>(path: string, body?: unknown, options?: RequestInit) =>
    request<T>(path, {
      ...options,
      method: 'POST',
      body: body
        ? isFormData(body)
          ? body
          : JSON.stringify(body)
        : undefined,
    }),

  put: <T>(path: string, body?: unknown, options?: RequestInit) =>
    request<T>(path, {
      ...options,
      method: 'PUT',
      body: body ? JSON.stringify(body) : undefined,
    }),

  delete: <T>(path: string, body?: unknown, options?: RequestInit) =>
    request<T>(path, {
      ...options,
      method: 'DELETE',
      body: body ? JSON.stringify(body) : undefined,
    }),

  patch: <T>(path: string, body?: unknown, options?: RequestInit) =>
    request<T>(path, {
      ...options,
      method: 'PATCH',
      body: body ? JSON.stringify(body) : undefined,
    }),
}
