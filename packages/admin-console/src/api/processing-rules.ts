import { api } from './client'
import { ChunkingSettings } from './ai'

// =============================================================================
// Types
// =============================================================================

/** Rule matcher types for conditional processing */
export type RuleMatcher =
  | { type: 'all' }
  | { type: 'node_type'; node_type: string }
  | { type: 'path'; pattern: string }
  | { type: 'mime_type'; mime_type: string }
  | { type: 'workspace'; workspace: string }
  | { type: 'property'; name: string; value: string }
  | { type: 'combined'; matchers: RuleMatcher[] }

/** PDF processing strategy */
export type PdfStrategy = 'auto' | 'native_only' | 'ocr_only' | 'force_ocr'

/** Processing settings for a rule */
export interface ProcessingSettings {
  /** Chunking configuration override */
  chunking?: ChunkingSettings
  /** PDF processing strategy */
  pdf_strategy?: PdfStrategy
  /** Generate image embeddings (CLIP) */
  generate_image_embedding?: boolean
  /** Generate image captions (Moondream/BLIP) */
  generate_image_caption?: boolean
  /** Caption model override (default: Moondream) */
  caption_model?: string
  /** Custom prompt for alt-text generation (Moondream only) */
  alt_text_prompt?: string
  /** Custom prompt for description generation (Moondream only) */
  description_prompt?: string
  /** Generate image keywords (Moondream only) */
  generate_keywords?: boolean
  /** Custom prompt for keyword extraction (Moondream only) */
  keywords_prompt?: string
  /** Embedding model override */
  embedding_model?: string
  /** Trigger embedding generation after extraction */
  trigger_embedding?: boolean
  /** Store extracted text in node properties */
  store_extracted_text?: boolean
  /** Maximum length of stored text */
  max_stored_text_length?: number
}

/** Processing rule definition */
export interface ProcessingRule {
  id: string
  name: string
  /** Priority order - lower numbers match first */
  order: number
  enabled: boolean
  matcher: RuleMatcher
  settings: ProcessingSettings
}

/** Response containing all rules for a repository */
export interface RulesListResponse {
  repo_id: string
  rules: ProcessingRule[]
}

/** Request body for creating a new rule */
export interface CreateRuleRequest {
  id?: string
  name: string
  order?: number
  enabled?: boolean
  matcher: RuleMatcher
  settings?: ProcessingSettings
}

/** Request body for updating an existing rule */
export interface UpdateRuleRequest {
  name?: string
  order?: number
  enabled?: boolean
  matcher?: RuleMatcher
  settings?: ProcessingSettings
}

/** Request body for reordering rules */
export interface ReorderRulesRequest {
  rule_ids: string[]
}

/** Request body for testing rule matching */
export interface TestRuleMatchRequest {
  path?: string
  node_type?: string
  mime_type?: string
  workspace?: string
  properties?: Record<string, string>
}

/** Response for rule matching test */
export interface TestRuleMatchResponse {
  matched: boolean
  matched_rule?: ProcessingRule
  rules_evaluated: number
}

/** Generic success response */
export interface SuccessResponse {
  success: boolean
  message: string
}

// =============================================================================
// API Functions
// =============================================================================

export const processingRulesApi = {
  /**
   * GET /api/repository/{repo}/ai/rules
   * List all processing rules for a repository
   */
  listRules: (repo: string) =>
    api.get<RulesListResponse>(`/api/repository/${encodeURIComponent(repo)}/ai/rules`),

  /**
   * GET /api/repository/{repo}/ai/rules/{ruleId}
   * Get a single processing rule by ID
   */
  getRule: (repo: string, ruleId: string) =>
    api.get<ProcessingRule>(
      `/api/repository/${encodeURIComponent(repo)}/ai/rules/${encodeURIComponent(ruleId)}`
    ),

  /**
   * POST /api/repository/{repo}/ai/rules
   * Create a new processing rule
   */
  createRule: (repo: string, request: CreateRuleRequest) =>
    api.post<ProcessingRule>(
      `/api/repository/${encodeURIComponent(repo)}/ai/rules`,
      request
    ),

  /**
   * PUT /api/repository/{repo}/ai/rules/{ruleId}
   * Update an existing processing rule
   */
  updateRule: (repo: string, ruleId: string, request: UpdateRuleRequest) =>
    api.put<ProcessingRule>(
      `/api/repository/${encodeURIComponent(repo)}/ai/rules/${encodeURIComponent(ruleId)}`,
      request
    ),

  /**
   * DELETE /api/repository/{repo}/ai/rules/{ruleId}
   * Delete a processing rule
   */
  deleteRule: (repo: string, ruleId: string) =>
    api.delete<SuccessResponse>(
      `/api/repository/${encodeURIComponent(repo)}/ai/rules/${encodeURIComponent(ruleId)}`
    ),

  /**
   * PUT /api/repository/{repo}/ai/rules/reorder
   * Reorder processing rules
   */
  reorderRules: (repo: string, ruleIds: string[]) =>
    api.put<SuccessResponse>(
      `/api/repository/${encodeURIComponent(repo)}/ai/rules/reorder`,
      { rule_ids: ruleIds }
    ),

  /**
   * POST /api/repository/{repo}/ai/rules/test
   * Test rule matching against provided metadata
   */
  testRuleMatch: (repo: string, request: TestRuleMatchRequest) =>
    api.post<TestRuleMatchResponse>(
      `/api/repository/${encodeURIComponent(repo)}/ai/rules/test`,
      request
    ),
}
