import { api } from './client'
import type { ProviderSlug } from './ai'

/**
 * The legacy embedding provider enum.
 *
 * Superseded by `ai_provider_ref`, which names one of the tenant's configured
 * AI providers by SLUG. The enum is still sent because the server still
 * requires the field, but it only records which wire protocol the referenced
 * entry speaks — it cannot distinguish two gateways of the same kind.
 */
export type LegacyEmbeddingProvider = 'OpenAI' | 'Claude' | 'Ollama' | 'HuggingFace'

export interface NodeTypeEmbeddingConfig {
  enabled: boolean
  properties_to_embed: string[]
}

export interface ConfigResponse {
  tenant_id: string
  enabled: boolean
  /** The configured AI provider, by slug. Preferred over `provider`. */
  ai_provider_ref?: ProviderSlug
  /** The model within that provider. Preferred over `model`. */
  ai_model_ref?: string
  provider: LegacyEmbeddingProvider
  model: string
  dimensions: number
  has_api_key: boolean
  include_name: boolean
  include_path: boolean
  node_type_settings: Record<string, NodeTypeEmbeddingConfig>
  max_embeddings_per_repo: number | null
  base_url?: string
  default_max_distance?: number
  quantization?: 'F32' | 'F16' | 'Int8'
}

export interface SetConfigRequest {
  enabled: boolean
  /**
   * The configured AI provider, by slug.
   *
   * POST replaces the whole document, so this MUST be resent on every save: a
   * payload without it wipes the tenant's provider reference and drops
   * embeddings back to whatever the legacy `provider` enum resolves to.
   */
  ai_provider_ref?: ProviderSlug
  ai_model_ref?: string
  provider: LegacyEmbeddingProvider
  model: string
  dimensions: number
  api_key_plain?: string
  include_name: boolean
  include_path: boolean
  node_type_settings: Record<string, NodeTypeEmbeddingConfig>
  max_embeddings_per_repo: number | null
  base_url?: string
  default_max_distance?: number
  quantization?: 'F32' | 'F16' | 'Int8'
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
