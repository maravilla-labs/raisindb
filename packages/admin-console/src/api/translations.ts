import { api } from './client'

export interface TranslationConfig {
  default_language: string
  supported_languages: string[]
  locale_fallback_chains: Record<string, string[]>
}

export interface UpdateTranslationRequest {
  translations: Record<string, any>
  message?: string
  actor?: string
}

export interface TranslationResponse {
  node_id: string
  locale: string
  revision: number
  timestamp: string
}

export interface ListTranslationsResponse {
  node_id: string
  locales: string[]
}

// Staleness detection types
export interface StaleFieldInfo {
  pointer: string
  original_hash_at_translation: string
  current_original_hash: string
  translated_at: string
}

export interface MissingFieldInfo {
  pointer: string
  current_original_hash: string
}

export interface StalenessReport {
  stale: StaleFieldInfo[]
  missing: MissingFieldInfo[]
  fresh: string[]
  unknown: string[]
}

export interface AcknowledgeStalenessResponse {
  acknowledged: boolean
  pointer: string
  locale: string
}

export const translationsApi = {
  /**
   * Get translation configuration for a repository
   * GET /api/repositories/{repo}/translation-config
   */
  getConfig: (repo: string) =>
    api.get<TranslationConfig>(`/api/repositories/${repo}/translation-config`),

  /**
   * Update or create a translation for a node in a specific locale
   * POST /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/translate
   */
  updateTranslation: async (
    repo: string,
    branch: string,
    workspace: string,
    nodePath: string,
    locale: string,
    request: UpdateTranslationRequest
  ): Promise<TranslationResponse> => {
    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${nodePath}/raisin:cmd/translate`,
      { ...request, locale }
    )
  },

  /**
   * List all translations for a node
   * GET /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/list-translations
   */
  listTranslations: async (
    repo: string,
    branch: string,
    workspace: string,
    nodePath: string
  ): Promise<ListTranslationsResponse> => {
    return api.get(
      `/api/repository/${repo}/${branch}/head/${workspace}${nodePath}/raisin:cmd/list-translations`
    )
  },

  /**
   * Delete a translation for a node in a specific locale
   * POST /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/delete-translation
   */
  deleteTranslation: async (
    repo: string,
    branch: string,
    workspace: string,
    nodePath: string,
    locale: string,
    message?: string,
    actor?: string
  ): Promise<void> => {
    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${nodePath}/raisin:cmd/delete-translation`,
      { locale, message, actor }
    )
  },

  /**
   * Hide a node in a specific locale
   * POST /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/hide-in-locale
   */
  hideNode: async (
    repo: string,
    branch: string,
    workspace: string,
    nodePath: string,
    locale: string,
    message?: string,
    actor?: string
  ): Promise<TranslationResponse> => {
    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${nodePath}/raisin:cmd/hide-in-locale`,
      { locale, message, actor }
    )
  },

  /**
   * Unhide a node in a specific locale (by deleting the translation)
   * POST /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/delete-translation
   */
  unhideNode: async (
    repo: string,
    branch: string,
    workspace: string,
    nodePath: string,
    locale: string,
    actor?: string
  ): Promise<void> => {
    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${nodePath}/raisin:cmd/delete-translation`,
      { locale, message: 'Unhide node', actor }
    )
  },

  /**
   * Check translation staleness for a node in a specific locale
   * POST /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/translation-staleness
   */
  checkStaleness: async (
    repo: string,
    branch: string,
    workspace: string,
    nodePath: string,
    locale: string
  ): Promise<StalenessReport> => {
    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${nodePath}/raisin:cmd/translation-staleness`,
      { locale }
    )
  },

  /**
   * Acknowledge a stale translation field without re-translating
   * POST /api/repository/{repo}/{branch}/head/{workspace}{path}/raisin:cmd/acknowledge-staleness
   */
  acknowledgeStaleness: async (
    repo: string,
    branch: string,
    workspace: string,
    nodePath: string,
    locale: string,
    pointer: string
  ): Promise<AcknowledgeStalenessResponse> => {
    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${nodePath}/raisin:cmd/acknowledge-staleness`,
      { locale, pointer }
    )
  },
}
