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

/** Who performs a task. Mirrors `raisin_ai::rules::tasks::TaskProvider`. */
export type TaskProviderKind = 'native' | 'function' | 'plugin'

/** One entry in the task catalogue, with availability on THIS server. */
export interface TaskCatalogEntry {
  slug: string
  summary: string
  provider: TaskProviderKind
  /** For `plugin`: the method that must be loaded (e.g. media.doc.toMarkdown). */
  method?: string
  /** For `function`: one line naming what should do it instead. */
  how?: string
  /**
   * Whether this server can run it right now. Server-computed — the console
   * cannot see the plugin registry, and a second availability table here would
   * drift from the one the asset job plans with.
   */
  available: boolean
}

export interface TaskCatalogResponse {
  tasks: TaskCatalogEntry[]
  /** False = no probe installed at startup; every `available` is a default. */
  capability_probe_installed: boolean
}

/** Why a configured task will not run here. */
export type BlockedReason =
  | { reason: 'malformed_slug' }
  | { reason: 'handled_above'; how: string }
  | { reason: 'plugin_missing'; method: string }
  | { reason: 'unknown' }

export interface PlannedTask {
  slug: string
  /** null for a native task; the plugin method otherwise. */
  via?: string | null
}

export interface BlockedTask {
  slug: string
  blocked: BlockedReason
}

/** What a matched rule will ACTUALLY do on this server. */
export interface PipelinePlan {
  runnable: PlannedTask[]
  blocked: BlockedTask[]
}

/** Processing settings for a rule */
export interface ProcessingSettings {
  /**
   * The tasks to run, in order. THIS is the routing table's action half.
   *
   * `undefined` means "not configured" and is NOT the same as `[]`: an absent
   * list falls back to deriving tasks from the legacy booleans below plus the
   * mimetype defaults, while `[]` explicitly means "match these nodes and do
   * nothing" — a real configuration for an opt-out rule ordered ahead of a
   * broad one.
   */
  tasks?: string[]
  /** Chunking configuration override */
  chunking?: ChunkingSettings
  /** PDF processing strategy */
  pdf_strategy?: PdfStrategy
  /** Generate image embeddings (CLIP) */
  generate_image_embedding?: boolean
  // No captioning settings. The six that used to be here mirrored Rust fields
  // that nothing read, and the engine answered all of them with one warn line.
  // Captioning is a trigger function's job now; see the comment where the
  // controls used to be in ProcessingRulesManagement.tsx.
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
  /** What the rule ASKS for, before capabilities are consulted. */
  effective_tasks: string[]
  /** What will actually happen on this server. */
  plan: PipelinePlan
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
   * GET /api/repository/{repo}/ai/rules/tasks
   * The task vocabulary, with per-task availability on this server.
   */
  listTasks: (repo: string) =>
    api.get<TaskCatalogResponse>(
      `/api/repository/${encodeURIComponent(repo)}/ai/rules/tasks`
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
