import { api } from './client'

export interface ApiKey {
  key_id: string
  name: string
  key_prefix: string
  created_at: string
  last_used_at: string | null
  is_active: boolean
}

export interface CreateApiKeyRequest {
  name: string
}

export interface CreateApiKeyResponse {
  key: ApiKey
  token: string // Full token - only shown once!
}

export const apiKeysApi = {
  /**
   * List all API keys for the current user
   */
  list: async (): Promise<ApiKey[]> => {
    return api.get<ApiKey[]>('/api/raisindb/me/api-keys')
  },

  /**
   * Create a new API key
   * Note: The token is only returned once at creation time!
   */
  create: async (name: string): Promise<CreateApiKeyResponse> => {
    return api.post<CreateApiKeyResponse>('/api/raisindb/me/api-keys', { name })
  },

  /**
   * Revoke an API key
   */
  revoke: async (keyId: string): Promise<void> => {
    return api.delete<void>(`/api/raisindb/me/api-keys/${keyId}`)
  }
}
