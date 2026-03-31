import { api } from './client'

export interface IdentityUser {
  id: string
  email: string
  display_name: string | null
  avatar_url: string | null
  email_verified: boolean
  is_active: boolean
  linked_providers: string[]
  created_at: string
  updated_at: string | null
  last_login_at: string | null
}

export interface UpdateIdentityUserRequest {
  display_name?: string
  is_active?: boolean
  email_verified?: boolean
}

export interface ListIdentityUsersParams {
  page?: number
  per_page?: number
  email?: string
  is_active?: boolean
}

export const identityUsersApi = {
  /**
   * List all identity users for a tenant
   */
  list: async (tenantId: string, params?: ListIdentityUsersParams): Promise<IdentityUser[]> => {
    const searchParams = new URLSearchParams()
    if (params?.page) searchParams.set('page', params.page.toString())
    if (params?.per_page) searchParams.set('per_page', params.per_page.toString())
    if (params?.email) searchParams.set('email', params.email)
    if (params?.is_active !== undefined) searchParams.set('is_active', params.is_active.toString())

    const query = searchParams.toString()
    return api.get<IdentityUser[]>(
      `/api/raisindb/sys/${tenantId}/identity-users${query ? `?${query}` : ''}`
    )
  },

  /**
   * Get a specific identity user
   */
  get: async (tenantId: string, identityId: string): Promise<IdentityUser> => {
    return api.get<IdentityUser>(`/api/raisindb/sys/${tenantId}/identity-users/${identityId}`)
  },

  /**
   * Update an identity user
   */
  update: async (
    tenantId: string,
    identityId: string,
    request: UpdateIdentityUserRequest
  ): Promise<IdentityUser> => {
    return api.patch<IdentityUser>(
      `/api/raisindb/sys/${tenantId}/identity-users/${identityId}`,
      request
    )
  },

  /**
   * Delete an identity user
   */
  delete: async (tenantId: string, identityId: string): Promise<void> => {
    return api.delete<void>(`/api/raisindb/sys/${tenantId}/identity-users/${identityId}`)
  },
}
