import { api } from './client'

// ============================================================================
// Types
// ============================================================================

export interface AuthProvider {
  id: string
  strategy_type: string
  display_name: string
  icon: string
  enabled: boolean
  created_at: string
}

export interface AuthProvidersResponse {
  providers: AuthProvider[]
  local_enabled: boolean
  magic_link_enabled: boolean
}

/**
 * One OIDC provider as `PUT /api/tenants/{tenant}/auth/config` accepts it in
 * `oidc_providers`. The list is a full replacement; an entry that omits
 * `client_secret` keeps the secret already stored for that `provider_id`.
 */
export interface OidcProviderConfig {
  /** Slug in the login URL: /auth/oidc/{provider_id} */
  provider_id: string
  display_name?: string
  icon?: string
  enabled?: boolean
  priority?: number
  client_id: string
  /** Plaintext on the way in only; sealed at rest, never echoed back */
  client_secret?: string
  /** Discovery base; `{issuer_url}/.well-known/openid-configuration` is fetched */
  issuer_url?: string
  /** Callback URL registered with the provider, byte for byte */
  redirect_uri?: string
  scopes?: string[]
  groups_claim?: string
  allowed_email_domains?: string[]
  attribute_mapping?: {
    email?: string
    name?: string
    picture?: string
    email_verified?: string
  }
  /** Manual endpoints for a provider without discovery */
  authorization_url?: string
  token_url?: string
  userinfo_url?: string
  jwks_url?: string
}

/** One OIDC provider as the config endpoints report it. Never carries the secret. */
export interface OidcProviderView extends Omit<OidcProviderConfig, 'client_secret'> {
  display_name: string
  icon: string
  enabled: boolean
  priority: number
  has_client_secret: boolean
  scopes: string[]
  allowed_email_domains: string[]
  /** Server-relative URL that starts a login with this provider */
  authorize_url: string
}

export interface LocalAuthConfig {
  enabled: boolean
  allow_registration?: boolean
}

export interface MagicLinkConfig {
  enabled: boolean
  token_ttl_minutes?: number
}

export interface PasswordPolicy {
  min_length: number
  require_uppercase: boolean
  require_lowercase: boolean
  require_numbers: boolean
  require_special: boolean
  max_age_days?: number
}

export interface SessionSettings {
  duration_hours: number
  refresh_token_duration_days: number
  max_sessions_per_user: number
  single_session_mode: boolean
}

export interface AccessSettings {
  allow_access_requests: boolean
  allow_invitations: boolean
  require_approval: boolean
  default_roles: string[]
}

export interface TenantAuthSettings {
  tenant_id: string
  local_auth: LocalAuthConfig
  magic_link: MagicLinkConfig
  /** External OpenID Connect providers (read shape) */
  oidc_providers?: OidcProviderView[]
  password_policy: PasswordPolicy
  session_settings: SessionSettings
  access_settings: AccessSettings
  /** Whether anonymous (unauthenticated) access is enabled globally */
  anonymous_enabled?: boolean
  /** CORS allowed origins for this tenant (fallback when repo-level is not configured) */
  cors_allowed_origins?: string[]
}

export interface IdentityInfo {
  identity_id: string
  email: string
  display_name?: string
  avatar_url?: string
  email_verified: boolean
  is_active: boolean
  linked_providers: string[]
  created_at: string
  last_login_at?: string
}

export interface IdentitiesResponse {
  identities: IdentityInfo[]
  total: number
}

export interface SessionInfo {
  id: string
  identity_id: string
  auth_strategy: string
  user_agent?: string
  ip_address?: string
  created_at: string
  last_active_at: string
}

export interface SessionsResponse {
  sessions: SessionInfo[]
  total: number
}

export interface AccessRequestInfo {
  id: string
  identity_id: string
  email: string
  display_name?: string
  repo_id: string
  status: 'pending' | 'approved' | 'denied'
  message?: string
  requested_roles: string[]
  created_at: string
}

export interface AccessRequestsResponse {
  requests: AccessRequestInfo[]
  total: number
}

// ============================================================================
// API Functions
// ============================================================================

export const identityAuthApi = {
  /**
   * Get available authentication providers
   */
  getProviders: async (): Promise<AuthProvidersResponse> => {
    return api.get<AuthProvidersResponse>('/auth/providers')
  },

  /**
   * Replace the OIDC provider list through the tenant config endpoint.
   *
   * There is no per-provider route on the server; providers are a section of
   * `PUT /api/tenants/{tenant}/auth/config`. Existing entries are re-sent
   * WITHOUT a secret, which tells the server to keep the one it has.
   */
  _replaceOidcProviders: async (
    tenantId: string,
    mutate: (current: OidcProviderConfig[]) => OidcProviderConfig[]
  ): Promise<void> => {
    const settings = await api.get<TenantAuthSettings>(`/api/tenants/${tenantId}/auth/config`)
    const current: OidcProviderConfig[] = (settings.oidc_providers ?? []).map(
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      ({ has_client_secret, authorize_url, ...rest }) => rest
    )
    await api.put(`/api/tenants/${tenantId}/auth/config`, { oidc_providers: mutate(current) })
  },

  /**
   * Add a new OIDC provider. `strategyType` is `oidc:{provider_id}`.
   */
  addProvider: async (
    tenantId: string,
    strategyType: string,
    config: Omit<OidcProviderConfig, 'provider_id'>
  ): Promise<{ provider_id: string }> => {
    const provider_id = strategyType.replace(/^oidc:/, '')
    await identityAuthApi._replaceOidcProviders(tenantId, current => [
      ...current.filter(p => p.provider_id !== provider_id),
      { ...config, provider_id },
    ])
    return { provider_id }
  },

  /**
   * Update fields of an existing OIDC provider (enabled, scopes, ...).
   */
  updateProvider: async (
    tenantId: string,
    providerId: string,
    patch: Partial<Omit<OidcProviderConfig, 'provider_id'>>
  ): Promise<void> => {
    await identityAuthApi._replaceOidcProviders(tenantId, current =>
      current.map(p => (p.provider_id === providerId ? { ...p, ...patch } : p))
    )
  },

  /**
   * Remove an OIDC provider.
   */
  removeProvider: async (tenantId: string, providerId: string): Promise<void> => {
    await identityAuthApi._replaceOidcProviders(tenantId, current =>
      current.filter(p => p.provider_id !== providerId)
    )
  },

  /**
   * GET /api/tenants/{tenantId}/auth/config
   * Get tenant authentication settings
   */
  getSettings: async (tenantId: string): Promise<TenantAuthSettings> => {
    return api.get<TenantAuthSettings>(`/api/tenants/${tenantId}/auth/config`)
  },

  /**
   * PUT /api/tenants/{tenantId}/auth/config
   * Update tenant authentication settings
   */
  updateSettings: async (
    tenantId: string,
    settings: Partial<Omit<TenantAuthSettings, 'oidc_providers'>> & {
      /** Full replacement of the OIDC provider list; secrets go in as plaintext here only */
      oidc_providers?: OidcProviderConfig[]
    }
  ): Promise<TenantAuthSettings> => {
    return api.put<TenantAuthSettings>(`/api/tenants/${tenantId}/auth/config`, settings)
  },

  /**
   * List identities (admin only)
   */
  listIdentities: async (params?: {
    page?: number
    per_page?: number
    search?: string
  }): Promise<IdentitiesResponse> => {
    const searchParams = new URLSearchParams()
    if (params?.page) searchParams.set('page', params.page.toString())
    if (params?.per_page) searchParams.set('per_page', params.per_page.toString())
    if (params?.search) searchParams.set('search', params.search)

    const query = searchParams.toString()
    return api.get<IdentitiesResponse>(`/auth/identities${query ? `?${query}` : ''}`)
  },

  /**
   * Get identity by ID
   */
  getIdentity: async (identityId: string): Promise<IdentityInfo> => {
    return api.get<IdentityInfo>(`/auth/identities/${identityId}`)
  },

  /**
   * Deactivate an identity
   */
  deactivateIdentity: async (identityId: string): Promise<void> => {
    return api.post(`/auth/identities/${identityId}/deactivate`, {})
  },

  /**
   * Reactivate an identity
   */
  reactivateIdentity: async (identityId: string): Promise<void> => {
    return api.post(`/auth/identities/${identityId}/reactivate`, {})
  },

  /**
   * List sessions for an identity (admin) or current user
   */
  listSessions: async (identityId?: string): Promise<SessionsResponse> => {
    const path = identityId
      ? `/auth/identities/${identityId}/sessions`
      : '/auth/sessions'
    return api.get<SessionsResponse>(path)
  },

  /**
   * Revoke a session
   */
  revokeSession: async (sessionId: string): Promise<void> => {
    return api.delete(`/auth/sessions/${sessionId}`)
  },

  /**
   * Revoke all sessions for an identity
   */
  revokeAllSessions: async (identityId: string): Promise<void> => {
    return api.post(`/auth/identities/${identityId}/revoke-sessions`, {})
  },

  /**
   * List access requests (admin only)
   */
  listAccessRequests: async (params?: {
    repo_id?: string
    status?: 'pending' | 'approved' | 'denied'
    page?: number
    per_page?: number
  }): Promise<AccessRequestsResponse> => {
    const searchParams = new URLSearchParams()
    if (params?.repo_id) searchParams.set('repo_id', params.repo_id)
    if (params?.status) searchParams.set('status', params.status)
    if (params?.page) searchParams.set('page', params.page.toString())
    if (params?.per_page) searchParams.set('per_page', params.per_page.toString())

    const query = searchParams.toString()
    return api.get<AccessRequestsResponse>(`/auth/access-requests${query ? `?${query}` : ''}`)
  },

  /**
   * Approve an access request
   */
  approveAccessRequest: async (
    requestId: string,
    roles: string[],
    message?: string
  ): Promise<void> => {
    return api.post(`/auth/access-requests/${requestId}/approve`, { roles, message })
  },

  /**
   * Deny an access request
   */
  denyAccessRequest: async (requestId: string, reason?: string): Promise<void> => {
    return api.post(`/auth/access-requests/${requestId}/deny`, { reason })
  },

  /**
   * Test OIDC provider configuration
   */
  testProvider: async (providerId: string): Promise<{ success: boolean; error?: string }> => {
    return api.post<{ success: boolean; error?: string }>(
      `/auth/providers/${providerId}/test`,
      {}
    )
  },
}
