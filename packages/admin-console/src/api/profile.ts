import { api } from './client'
import type { AdminAccessFlags } from './admin-users'

export interface UserProfile {
  user_id: string
  username: string
  email: string | null
  tenant_id: string
  access_flags: AdminAccessFlags
  must_change_password: boolean
}

export const profileApi = {
  /**
   * Get the current user's profile
   */
  get: async (): Promise<UserProfile> => {
    return api.get<UserProfile>('/api/raisindb/me')
  },

  /**
   * List available repositories for the current user's tenant
   * Useful for building PostgreSQL connection strings
   */
  getRepositories: async (): Promise<string[]> => {
    return api.get<string[]>('/api/raisindb/me/repositories')
  }
}
