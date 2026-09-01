import { api } from './client'

/**
 * Provider KIND — which wire protocol an entry speaks.
 *
 * NOT an identity. A tenant can configure N entries of the same kind, each
 * addressed by its own `slug`; the wire field keeps the name `provider` for
 * compatibility, which is why `ProviderConfigResponse.provider` is a kind while
 * `.slug` is the key.
 */
export type AIProvider = 'openai' | 'anthropic' | 'google' | 'azure_openai' | 'ollama' | 'groq' | 'openrouter' | 'bedrock' | 'local' | 'custom'

/**
 * A per-tenant provider identifier, e.g. `openai`, `marvel`, `eu-gateway`.
 *
 * Also the model-id prefix (`<slug>:<model>`), the value of an agent node's
 * `provider` property, and of `ai_provider_ref`. A plain string with no
 * referential integrity behind it — aliased for documentation only.
 */
export type ProviderSlug = string

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
  /**
   * Which configured provider generates embeddings — a SLUG, not a kind. A
   * tenant with two OpenAI-compatible gateways picks one of them here.
   */
  ai_provider_ref?: ProviderSlug
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

// Which providers can embed is decided per ENTRY, not per kind — a `custom`
// gateway that publishes an embedding model qualifies. See
// `isEmbeddingCapable` in `utils/aiProviders`.

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
  /** Per-tenant identity, and the model-id prefix. Unique within a tenant. */
  slug: ProviderSlug
  /** The kind — which wire protocol this entry speaks. Not unique. */
  provider: AIProvider
  /** Label shipped by a provisioned entry; how two same-kind entries are told apart. */
  display_name?: string
  icon_url?: string
  has_api_key: boolean
  api_endpoint?: string
  enabled: boolean
  models: AIModelConfig[]
}

/** One row of GET /ai/providers — the same identity, with a model count instead of models. */
export interface ProviderSummary {
  slug: ProviderSlug
  provider: AIProvider
  display_name?: string
  icon_url?: string
  api_endpoint?: string
  enabled: boolean
  has_api_key: boolean
  model_count: number
}

/**
 * Tenant-wide asset processing defaults.
 *
 * These sit UNDER any per-repository processing rule: a rule that names its own
 * value wins, and these fill the gaps. Setting them here means a tenant says
 * "we work in German" once instead of on every rule in every repository.
 */
export interface ProcessingDefaults {
  caption_model?: string
  embedding_model?: string
  generate_image_caption?: boolean
  generate_image_embedding?: boolean
  extract_pdf_text?: boolean
  /**
   * TESSERACT language codes (eng, deu, fra) — NOT ISO 639-1 (en, de), which
   * name no traineddata and fail to initialise.
   *
   * All listed languages are read in a single pass, so a mixed-language page
   * needs no detection. Each costs roughly 10 MB of resident model, so this is
   * a short list; the server refuses more than eight.
   */
  ocr_languages?: string[]
  /** Drop OCR words scored below this (0-100). Server default is 50. */
  ocr_min_word_confidence?: number
}

// AI Config response from backend (GET)
export interface AIConfigResponse {
  tenant_id: string
  providers: ProviderConfigResponse[]
  embedding_settings?: EmbeddingSettings
  processing_defaults?: ProcessingDefaults
}

// Backward compatibility alias
export type AIConfig = AIConfigResponse

/**
 * One entry of a PUT /ai/config payload.
 *
 * `slug` is optional on the wire (absent defaults to the kind's serde name, for
 * clients written before slugs existed) but this client always sends it: for a
 * second entry of the same kind the default addresses the wrong row.
 *
 * The descriptive fields are three-state on the server: omitting one keeps the
 * stored value, sending `null` clears it, sending a value sets it. `enabled`
 * and `models` are NOT — they are written whole, so an omitted `models` clears
 * the entry's model list.
 */
export interface ProviderConfigRequest {
  slug: ProviderSlug
  /** The kind. Immutable once the slug exists; the server rejects a change. */
  provider: AIProvider
  /** Omit to keep the stored name; `null` clears it. */
  display_name?: string | null
  /** Omit to keep the stored icon; `null` clears it. */
  icon_url?: string | null
  enabled: boolean
  /** Omit to keep the stored key. Never send `''`. */
  api_key_plain?: string
  /** Omit to keep the stored endpoint; `null` clears it. */
  api_endpoint?: string | null
  models?: AIModelConfig[]
}

/**
 * Request to update AI configuration (PUT).
 *
 * `providers` is a MERGE keyed by slug: entries present are created or updated,
 * entries only in storage are left alone. Send only what changed — an entry you
 * omit is safe, and removal goes through `deleteProvider`.
 */
export interface UpdateAIConfigRequest {
  providers: ProviderConfigRequest[]
  embedding_settings?: EmbeddingSettings
  /** Omitted keeps whatever is stored — it is never cleared by absence. */
  processing_defaults?: ProcessingDefaults
}

// Success response from PUT
export interface SuccessResponse {
  success: boolean
  message: string
}

// Test connection response
export interface TestConnectionResponse {
  success: boolean
  /** The slug that was tested. */
  slug: ProviderSlug
  /** The kind behind that slug. */
  provider: AIProvider
  message?: string
  error?: string
}

/** One discovered model, tagged with the entry that serves it. */
export interface ModelInfo {
  model_id: string
  display_name: string
  /** The kind — what a client switches on to pick an icon or a protocol hint. */
  provider: AIProvider
  /** The slug that addresses this model as `<provider_slug>:<model_id>`. */
  provider_slug: ProviderSlug
  use_cases: ModelUseCase[]
  default_temperature: number
  default_max_tokens: number
}

// Models response
export interface ModelsResponse {
  models: ModelInfo[]
}

// Model capabilities response
export interface ModelCapabilitiesResponse {
  model_id: string
  provider: AIProvider
  /** The slug that was queried. */
  provider_slug: ProviderSlug
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
   * Merge provider entries into the AI configuration.
   *
   * Entries are matched by slug: present -> created or updated, absent -> left
   * alone. Nothing is ever deleted here — use `deleteProvider`.
   */
  updateConfig: (tenantId: string, request: UpdateAIConfigRequest) =>
    api.put<SuccessResponse>(`/api/tenants/${tenantId}/ai/config`, request),

  /**
   * DELETE /api/tenants/{tenantId}/ai/providers/{slug}
   * Remove one provider entry.
   *
   * The only way an entry goes away. Removal is by slug, so deleting one of two
   * same-kind gateways leaves the other — and its encrypted key — alone.
   */
  deleteProvider: (tenantId: string, slug: ProviderSlug) =>
    api.delete<SuccessResponse>(
      `/api/tenants/${tenantId}/ai/providers/${encodeURIComponent(slug)}`
    ),

  /**
   * GET /api/tenants/{tenantId}/ai/models
   * Get available models (dynamically fetched from configured providers)
   *
   * @param tenantId - Tenant ID
   * @param options.provider - Filter by provider SLUG
   * @param options.refresh - If true, fetch models from provider APIs instead of cached
   */
  getAvailableModels: (
    tenantId: string,
    options?: { provider?: ProviderSlug; refresh?: boolean }
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
    api.get<{ providers: ProviderSummary[] }>(`/api/tenants/${tenantId}/ai/providers`),

  /**
   * POST /api/tenants/{tenantId}/ai/providers/{slug}/test
   * Test one entry's connection. Addressed by slug — two same-kind gateways
   * have different credentials and endpoints.
   */
  testProvider: (tenantId: string, slug: ProviderSlug) =>
    api.post<TestConnectionResponse>(
      `/api/tenants/${tenantId}/ai/providers/${encodeURIComponent(slug)}/test`,
      {}
    ),

  /**
   * GET /api/tenants/{tenantId}/ai/providers/{slug}/models/{model}/capabilities
   * Get capabilities for a specific model (including tool calling support)
   */
  getModelCapabilities: (tenantId: string, slug: ProviderSlug, modelId: string) =>
    api.get<ModelCapabilitiesResponse>(
      `/api/tenants/${tenantId}/ai/providers/${encodeURIComponent(slug)}/models/${encodeURIComponent(modelId)}/capabilities`
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
