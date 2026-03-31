import { api } from './client'

export interface FullTextSearchRequest {
  query: string
  workspace?: string
  language?: string
  limit?: number
}

export interface SearchResultItem {
  node_id: string
  workspace_id: string
  name: string
  path: string
  node_type: string
  score: number
  fulltext_rank?: number
  vector_distance?: number
  revision?: number
}

export interface HybridSearchRequest {
  q: string
  strategy?: 'fulltext' | 'vector' | 'hybrid'
  limit?: number
  workspace?: string
  branch?: string
}

export interface HybridSearchResponse {
  results: SearchResultItem[]
  count: number
  strategy: string
  fulltext_count: number
  vector_count: number
}

export const searchApi = {
  /**
   * Full-text search across repository
   * POST /api/repository/{repo}/{branch}/fulltext/search
   */
  search: (
    repo: string,
    branch: string,
    request: FullTextSearchRequest
  ) =>
    api.post<SearchResultItem[]>(
      `/api/repository/${repo}/${branch}/fulltext/search`,
      request
    ),

  /**
   * Hybrid search combining fulltext and vector search
   * GET /api/search/{repo}?q=...&strategy=...
   */
  hybridSearch: (repo: string, request: HybridSearchRequest) => {
    const params = new URLSearchParams()
    params.append('q', request.q)
    if (request.strategy) params.append('strategy', request.strategy)
    if (request.limit) params.append('limit', request.limit.toString())
    if (request.workspace) params.append('workspace', request.workspace)
    if (request.branch) params.append('branch', request.branch)

    return api.get<HybridSearchResponse>(`/api/search/${repo}?${params.toString()}`)
  },
}
