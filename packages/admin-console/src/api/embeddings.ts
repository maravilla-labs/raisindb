import { api } from './client'

export interface NodeTypeEmbeddingConfig {
  enabled: boolean
  properties_to_embed: string[]
}

export interface ConfigResponse {
  tenant_id: string
  enabled: boolean
  provider: 'OpenAI' | 'Claude' | 'Ollama'
  model: string
  dimensions: number
  has_api_key: boolean
  include_name: boolean
  include_path: boolean
  node_type_settings: Record<string, NodeTypeEmbeddingConfig>
  max_embeddings_per_repo: number | null
}

export interface SetConfigRequest {
  enabled: boolean
  provider: 'OpenAI' | 'Claude' | 'Ollama'
  model: string
  dimensions: number
  api_key_plain?: string
  include_name: boolean
  include_path: boolean
  node_type_settings: Record<string, NodeTypeEmbeddingConfig>
  max_embeddings_per_repo: number | null
}

export interface TestConnectionResponse {
  success: boolean
  dimensions?: number
  model: string
  error?: string
}

export const embeddingsApi = {
  /**
   * GET /api/tenants/{tenant}/embeddings/config
   */
  getConfig: (tenant: string) =>
    api.get<ConfigResponse>(`/api/tenants/${tenant}/embeddings/config`),

  /**
   * POST /api/tenants/{tenant}/embeddings/config
   */
  setConfig: (tenant: string, request: SetConfigRequest) =>
    api.post<ConfigResponse>(`/api/tenants/${tenant}/embeddings/config`, request),

  /**
   * POST /api/tenants/{tenant}/embeddings/config/test
   */
  testConnection: (tenant: string) =>
    api.post<TestConnectionResponse>(`/api/tenants/${tenant}/embeddings/config/test`, {}),
}
