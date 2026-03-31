import { api } from './client'

export interface AdminAccessFlags {
  console_login: boolean
  cli_access: boolean
  api_access: boolean
  pgwire_access: boolean
  can_impersonate: boolean
}

export interface AdminUser {
  user_id: string
  username: string
  email: string | null
  tenant_id: string
  access_flags: AdminAccessFlags
  must_change_password: boolean
  created_at: string
  last_login: string | null
  is_active: boolean
}

export interface CreateAdminUserRequest {
  username: string
  email?: string
  password: string
  access_flags: AdminAccessFlags
}

export interface UpdateAdminUserRequest {
  email?: string
  access_flags?: AdminAccessFlags
  must_change_password?: boolean
  is_active?: boolean
}

export const adminUsersApi = {
  /**
   * List all admin users for a tenant
   */
  list: async (tenantId: string): Promise<AdminUser[]> => {
    return api.get<AdminUser[]>(`/api/raisindb/sys/${tenantId}/admin-users`)
  },

  /**
   * Get a specific admin user
   */
  get: async (tenantId: string, username: string): Promise<AdminUser> => {
    return api.get<AdminUser>(`/api/raisindb/sys/${tenantId}/admin-users/${username}`)
  },

  /**
   * Create a new admin user
   */
  create: async (tenantId: string, request: CreateAdminUserRequest): Promise<AdminUser> => {
    return api.post<AdminUser>(`/api/raisindb/sys/${tenantId}/admin-users`, request)
  },

  /**
   * Update an existing admin user
   */
  update: async (
    tenantId: string,
    username: string,
    request: UpdateAdminUserRequest
  ): Promise<AdminUser> => {
    return api.put<AdminUser>(`/api/raisindb/sys/${tenantId}/admin-users/${username}`, request)
  },

  /**
   * Delete an admin user
   */
  delete: async (tenantId: string, username: string): Promise<void> => {
    return api.delete<void>(`/api/raisindb/sys/${tenantId}/admin-users/${username}`)
  }
}
