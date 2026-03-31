// SPDX-License-Identifier: BSL-1.1

import { api } from './client'

export interface LoginRequest {
  username: string
  password: string
  interface?: 'console' | 'cli' | 'api'
}

export interface AdminAccessFlags {
  console_login: boolean
  cli_access: boolean
  api_access: boolean
  pgwire_access: boolean
  can_impersonate: boolean
}

export interface LoginResponse {
  token: string
  user_id: string
  username: string
  must_change_password: boolean
  expires_at: number
  access_flags?: AdminAccessFlags
}

export interface ChangePasswordRequest {
  old_password: string
  new_password: string
}

export const authApi = {
  /**
   * Authenticate a user with username and password
   */
  login: async (tenantId: string, request: LoginRequest): Promise<LoginResponse> => {
    return api.post<LoginResponse>(
      `/api/raisindb/sys/${tenantId}/auth`,
      {
        ...request,
        interface: request.interface || 'console'
      }
    )
  },

  /**
   * Change password for the authenticated user
   */
  changePassword: async (
    tenantId: string,
    request: ChangePasswordRequest,
    token: string
  ): Promise<void> => {
    return api.post<void>(
      `/api/raisindb/sys/${tenantId}/auth/change-password`,
      request,
      {
        headers: {
          Authorization: `Bearer ${token}`
        }
      }
    )
  }
}
