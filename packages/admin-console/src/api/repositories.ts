import { api } from './client'

export interface Repository {
  repo_id: string
  tenant_id: string
  config: {
    default_branch: string
    description?: string
    tags: Record<string, string>
    default_language: string
    supported_languages: string[]
    locale_fallback_chains?: Record<string, string[]>
  }
  created_at: string
  updated_at?: string
}

export interface CreateRepositoryRequest {
  repo_id: string
  description?: string
  default_branch?: string
  default_language?: string
  supported_languages?: string[]
}

export interface UpdateRepositoryRequest {
  description?: string
  default_branch?: string
  supported_languages?: string[]
  locale_fallback_chains?: Record<string, string[]>
}

export const repositoriesApi = {
  /**
   * List all repositories for the default tenant
   * GET /api/repositories
   */
  list: () => api.get<Repository[]>('/api/repositories'),

  /**
   * Get repository information
   * GET /api/repositories/{repo_id}
   */
  get: (repoId: string) =>
    api.get<Repository>(`/api/repositories/${repoId}`),

  /**
   * Create a new repository
   * POST /api/repositories
   */
  create: (data: CreateRepositoryRequest) =>
    api.post<Repository>('/api/repositories', data),

  /**
   * Update repository configuration
   * PUT /api/repositories/{repo_id}
   */
  update: (repoId: string, data: UpdateRepositoryRequest) =>
    api.put<void>(`/api/repositories/${repoId}`, data),

  /**
   * Delete a repository
   * DELETE /api/repositories/{repo_id}
   */
  delete: (repoId: string) =>
    api.delete<void>(`/api/repositories/${repoId}`),
}
