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

export interface OidcProviderConfig {
  display_name: string
  icon?: string
  client_id: string
  client_secret: string
  issuer_url: string
  scopes?: string[]
  groups_claim?: string
  attribute_mapping?: {
    email?: string
    name?: string
    picture?: string
    groups?: string
  }
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
   * Add a new authentication provider
   */
  addProvider: async (
    strategyType: string,
    config: OidcProviderConfig
  ): Promise<{ provider_id: string }> => {
    return api.post<{ provider_id: string }>('/auth/providers', {
      strategy_type: strategyType,
      config,
    })
  },

  /**
   * Update an authentication provider
   */
  updateProvider: async (
    providerId: string,
    config: Partial<OidcProviderConfig & { enabled: boolean }>
  ): Promise<AuthProvider> => {
    return api.put<AuthProvider>(`/auth/providers/${providerId}`, config)
  },

  /**
   * Remove an authentication provider
   */
  removeProvider: async (providerId: string): Promise<void> => {
    return api.delete(`/auth/providers/${providerId}`)
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
    settings: Partial<TenantAuthSettings>
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
