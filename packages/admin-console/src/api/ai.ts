import { api } from './client'

// Provider types
export type AIProvider = 'openai' | 'anthropic' | 'google' | 'azure_openai' | 'ollama' | 'groq' | 'openrouter' | 'bedrock' | 'local' | 'custom'

// Model capability types
export type ModelCapability = 'chat' | 'embedding' | 'vision' | 'tools'

// Model use case types
export type ModelUseCase = 'embedding' | 'chat' | 'agent' | 'completion'

// Overlap configuration for chunking
export type OverlapConfig =
  | { type: 'Tokens'; value: number }
  | { type: 'Percentage'; value: number }

// Splitter type for text chunking
export type SplitterType = 'recursive' | 'fixed_size' | 'markdown' | 'code'

// Distance metric for vector search
export type DistanceMetric = 'Cosine' | 'L2' | 'InnerProduct' | 'Hamming'

// Quantization type for vector storage
export type QuantizationType = 'F32' | 'F16' | 'Int8'

// HNSW index parameters
export interface HnswParams {
  connectivity: number   // M parameter, 0 = auto
  expansion_add: number  // ef_construction, 0 = auto
  expansion_search: number  // ef_search, 0 = auto
}

// Default HNSW params (all auto)
export const DEFAULT_HNSW_PARAMS: HnswParams = {
  connectivity: 0,
  expansion_add: 0,
  expansion_search: 0,
}

// Chunking settings for embedding generation
export interface ChunkingSettings {
  chunk_size: number
  overlap: OverlapConfig
  splitter: SplitterType
  tokenizer_id?: string
}

// Default chunking settings
export const DEFAULT_CHUNKING_SETTINGS: ChunkingSettings = {
  chunk_size: 256,
  overlap: { type: 'Tokens', value: 64 },
  splitter: 'recursive',
}

// Embedding settings
export interface EmbeddingSettings {
  enabled: boolean
  // Unified provider reference (preferred) - references a provider from TenantAIConfig
  ai_provider_ref?: AIProvider
  ai_model_ref?: string
  include_name: boolean
  include_path: boolean
  max_embeddings_per_repo?: number
  dimensions: number
  chunking?: ChunkingSettings
  default_max_distance?: number
  distance_metric?: DistanceMetric
  quantization?: QuantizationType
  hnsw_params?: HnswParams
}

// List of providers that support embeddings
export const EMBEDDING_CAPABLE_PROVIDERS: AIProvider[] = ['openai', 'ollama']

// Default embedding models per provider
export const DEFAULT_EMBEDDING_MODELS: Record<string, string> = {
  openai: 'text-embedding-3-small',
  ollama: 'nomic-embed-text',
}

// AI Model config (backend format)
export interface AIModelConfig {
  model_id: string
  display_name: string
  use_cases: ModelUseCase[]
  default_temperature: number
  default_max_tokens: number
  // Optional metadata from provider (architecture, embedding_length, etc.)
  metadata?: {
    architecture?: string
    embedding_length?: number
    [key: string]: unknown
  }
}

// Provider configuration response (GET)
export interface ProviderConfigResponse {
  provider: AIProvider
  has_api_key: boolean
  api_endpoint?: string
  enabled: boolean
  models: AIModelConfig[]
}

// AI Config response from backend (GET)
export interface AIConfigResponse {
  tenant_id: string
  providers: ProviderConfigResponse[]
  embedding_settings?: EmbeddingSettings
}

// Backward compatibility alias
export type AIConfig = AIConfigResponse

// Provider configuration request (PUT)
export interface ProviderConfigRequest {
  provider: AIProvider
  enabled: boolean
  api_key_plain?: string
  api_endpoint?: string
  models?: AIModelConfig[]
}

// Request to update AI configuration (PUT)
export interface UpdateAIConfigRequest {
  providers: ProviderConfigRequest[]
  embedding_settings?: EmbeddingSettings
}

// Success response from PUT
export interface SuccessResponse {
  success: boolean
  message: string
}

// Test connection response
export interface TestConnectionResponse {
  success: boolean
  provider: AIProvider
  message?: string
  error?: string
}

// Models response
export interface ModelsResponse {
  models: AIModelConfig[]
}

// Model capabilities response
export interface ModelCapabilitiesResponse {
  model_id: string
  provider: AIProvider
  capabilities: {
    chat: boolean
    embeddings: boolean
    vision: boolean
    tools: boolean
    streaming: boolean
  }
}

// ============================================================================
// HuggingFace Models API Types
// ============================================================================

// Download status for HuggingFace model
export type HuggingFaceDownloadStatus =
  | { type: 'not_downloaded' }
  | { type: 'downloading'; progress: number; downloaded_bytes: number; total_bytes?: number }
  | { type: 'ready' }
  | { type: 'failed'; error: string }

// HuggingFace model info
export interface HuggingFaceModel {
  model_id: string
  display_name: string
  model_type: string
  capabilities: string[]
  estimated_size_bytes?: number
  actual_size_bytes?: number
  status: HuggingFaceDownloadStatus
  description?: string
  model_url: string
  size_display: string
}

// List HuggingFace models response
export interface HuggingFaceModelsListResponse {
  models: HuggingFaceModel[]
  total_disk_usage: string
}

// HuggingFace model download response
export interface HuggingFaceModelDownloadResponse {
  model_id: string
  job_id: string
  message: string
}

// HuggingFace model delete response
export interface HuggingFaceModelDeleteResponse {
  model_id: string
  success: boolean
  message: string
}

export const aiApi = {
  /**
   * GET /api/tenants/{tenantId}/ai/config
   * Get current AI configuration
   */
  getConfig: (tenantId: string) =>
    api.get<AIConfigResponse>(`/api/tenants/${tenantId}/ai/config`),

  /**
   * PUT /api/tenants/{tenantId}/ai/config
   * Update AI configuration
   */
  updateConfig: (tenantId: string, request: UpdateAIConfigRequest) =>
    api.put<SuccessResponse>(`/api/tenants/${tenantId}/ai/config`, request),

  /**
   * GET /api/tenants/{tenantId}/ai/models
   * Get available models (dynamically fetched from configured providers)
   *
   * @param tenantId - Tenant ID
   * @param options.provider - Filter by specific provider
   * @param options.refresh - If true, fetch models from provider APIs instead of cached
   */
  getAvailableModels: (
    tenantId: string,
    options?: { provider?: AIProvider; refresh?: boolean }
  ) => {
    const params = new URLSearchParams()
    if (options?.provider) params.set('provider', options.provider)
    if (options?.refresh) params.set('refresh', 'true')
    const queryString = params.toString()
    return api.get<ModelsResponse>(
      `/api/tenants/${tenantId}/ai/models${queryString ? `?${queryString}` : ''}`
    )
  },

  /**
   * GET /api/tenants/{tenantId}/ai/providers
   * List all configured providers
   */
  listProviders: (tenantId: string) =>
    api.get<{ providers: ProviderConfigResponse[] }>(`/api/tenants/${tenantId}/ai/providers`),

  /**
   * POST /api/tenants/{tenantId}/ai/providers/{provider}/test
   * Test provider connection
   */
  testProvider: (tenantId: string, provider: AIProvider) =>
    api.post<TestConnectionResponse>(`/api/tenants/${tenantId}/ai/providers/${provider}/test`, {}),

  /**
   * GET /api/tenants/{tenantId}/ai/providers/{provider}/models/{model}/capabilities
   * Get capabilities for a specific model (including tool calling support)
   */
  getModelCapabilities: (tenantId: string, provider: AIProvider, modelId: string) =>
    api.get<ModelCapabilitiesResponse>(
      `/api/tenants/${tenantId}/ai/providers/${provider}/models/${encodeURIComponent(modelId)}/capabilities`
    ),

  // ============================================================================
  // HuggingFace Models API
  // ============================================================================

  /**
   * GET /api/tenants/{tenantId}/ai/models/huggingface
   * List all available HuggingFace models
   */
  listHuggingFaceModels: (tenantId: string) =>
    api.get<HuggingFaceModelsListResponse>(`/api/tenants/${tenantId}/ai/models/huggingface`),

  /**
   * GET /api/tenants/{tenantId}/ai/models/huggingface/{modelId}
   * Get info for a specific HuggingFace model
   */
  getHuggingFaceModel: (tenantId: string, modelId: string) =>
    api.get<HuggingFaceModel>(
      `/api/tenants/${tenantId}/ai/models/huggingface/${encodeURIComponent(modelId)}`
    ),

  /**
   * POST /api/tenants/{tenantId}/ai/models/huggingface/{modelId}/download
   * Start downloading a HuggingFace model
   */
  downloadHuggingFaceModel: (tenantId: string, modelId: string) =>
    api.post<HuggingFaceModelDownloadResponse>(
      `/api/tenants/${tenantId}/ai/models/huggingface/${encodeURIComponent(modelId)}/download`,
      {}
    ),

  /**
   * DELETE /api/tenants/{tenantId}/ai/models/huggingface/{modelId}
   * Delete a downloaded HuggingFace model
   */
  deleteHuggingFaceModel: (tenantId: string, modelId: string) =>
    api.delete<HuggingFaceModelDeleteResponse>(
      `/api/tenants/${tenantId}/ai/models/huggingface/${encodeURIComponent(modelId)}`
    ),

  // ============================================================================
  // Local Captioning Models API
  // ============================================================================

  /**
   * GET /api/ai/models/local/caption
   * List available local image captioning models
   */
  listLocalCaptionModels: () =>
    api.get<LocalCaptionModelsResponse>('/api/ai/models/local/caption'),
}

// ============================================================================
// Local Captioning Models Types
// ============================================================================

/** Information about a local captioning model */
export interface LocalCaptionModel {
  /** Model ID (e.g., "Salesforce/blip-image-captioning-large") */
  id: string
  /** Human-readable name */
  name: string
  /** Approximate model size in MB */
  size_mb: number
  /** Whether this model is currently supported */
  supported: boolean
  /** Brief description */
  description: string
}

/** Response for listing local captioning models */
export interface LocalCaptionModelsResponse {
  models: LocalCaptionModel[]
  default_model: string
}
