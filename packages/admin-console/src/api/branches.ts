import { api } from './client'

const DEFAULT_TENANT = 'default'

export interface Branch {
  name: string
  head: string  // HLC format: "timestamp-counter"
  created_at: string
  created_by: string
  protected: boolean
  parent_revision?: string
  upstream_branch?: string  // Branch to compare against for divergence
}

export interface BranchDivergence {
  ahead: number  // Number of commits ahead
  behind: number  // Number of commits behind
  common_ancestor: string  // HLC format
}

export type MergeStrategy = 'FastForward' | 'ThreeWay'

export type ConflictType =
  | 'BothModified'
  | 'DeletedBySourceModifiedByTarget'
  | 'ModifiedBySourceDeletedByTarget'
  | 'BothAdded'

export interface MergeConflict {
  node_id: string
  path: string
  conflict_type: ConflictType
  base_properties: any | null
  target_properties: any | null
  source_properties: any | null
  /** Translation locale if this is a translation conflict (undefined = base node conflict) */
  translation_locale?: string
}

export interface MergeResult {
  success: boolean
  revision: string | null  // HLC format
  conflicts: MergeConflict[]
  fast_forward: boolean
  nodes_changed: number
}

export interface MergeBranchRequest {
  source_branch: string
  strategy: MergeStrategy
  message: string
  actor: string
}

export interface ConflictResolution {
  node_id: string
  resolution_type: 'keep-ours' | 'keep-theirs' | 'manual'
  resolved_properties: any
  /** Translation locale being resolved (undefined = base node conflict) */
  translation_locale?: string
}

export interface ResolveMergeRequest {
  source_branch: string
  resolutions: ConflictResolution[]
  message: string
  actor: string
}

export interface Tag {
  name: string
  revision: string  // HLC format: "timestamp-counter"
  created_at: string
  created_by: string
  message?: string
  protected: boolean
}

export interface HeadResponse {
  revision: string  // HLC format
}

export interface CreateBranchRequest {
  name: string
  from_revision?: string  // HLC format
  upstream_branch?: string  // Branch to compare against for divergence
  created_by?: string
  protected?: boolean
  include_revision_history?: boolean  // Default true - copy revision history from source branch
}

export interface CreateTagRequest {
  name: string
  revision: string  // HLC format
  created_by?: string
  message?: string
  protected?: boolean
}

export interface UpdateBranchHeadRequest {
  revision: string  // HLC format
}

export const branchesApi = {
  /**
   * List all branches in a repository
   * GET /api/management/repositories/{tenant}/{repo}/branches
   */
  list: (repoId: string) =>
    api.get<Branch[]>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches`
    ),

  /**
   * Get a specific branch
   * GET /api/management/repositories/{tenant}/{repo}/branches/{name}
   */
  get: (repoId: string, name: string) =>
    api.get<Branch>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${name}`
    ),

  /**
   * Create a new branch
   * POST /api/management/repositories/{tenant}/{repo}/branches
   */
  create: (repoId: string, data: CreateBranchRequest) =>
    api.post<Branch>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches`,
      data
    ),

  /**
   * Delete a branch
   * DELETE /api/management/repositories/{tenant}/{repo}/branches/{name}
   */
  delete: (repoId: string, name: string) =>
    api.delete<void>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${name}`
    ),

  /**
   * Get branch HEAD revision (structured response)
   * GET /api/management/repositories/{tenant}/{repo}/branches/{name}/head
   */
  getHead: (repoId: string, name: string) =>
    api.get<HeadResponse>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${name}/head`
    ),
  
  /**
   * Get branch HEAD revision (convenience method)
   */
  getHeadRevision: async (repoId: string, name: string): Promise<string> => {
    const response = await branchesApi.getHead(repoId, name)
    return response.revision
  },

  /**
   * Update branch HEAD pointer
   * PUT /api/management/repositories/{tenant}/{repo}/branches/{name}/head
   */
  updateHead: (repoId: string, name: string, revision: number) =>
    api.put<void>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${name}/head`,
      { revision }
    ),

  /**
   * Compare two branches to calculate divergence (commits ahead/behind)
   * GET /api/management/repositories/{tenant}/{repo}/branches/{branch}/compare/{baseBranch}
   */
  compare: (repoId: string, branch: string, baseBranch: string) =>
    api.get<BranchDivergence>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${branch}/compare/${baseBranch}`
    ),

  /**
   * Merge a source branch into a target branch
   * POST /api/management/repositories/{tenant}/{repo}/branches/{targetBranch}/merge
   */
  merge: (repoId: string, targetBranch: string, data: MergeBranchRequest) =>
    api.post<MergeResult>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${targetBranch}/merge`,
      data
    ),

  /**
   * Resolve merge conflicts and complete the merge
   * POST /api/management/repositories/{tenant}/{repo}/branches/{targetBranch}/resolve-merge
   */
  resolveMerge: (repoId: string, targetBranch: string, data: ResolveMergeRequest) =>
    api.post<MergeResult>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${targetBranch}/resolve-merge`,
      data
    ),

  /**
   * Set the upstream branch for divergence comparison
   * PATCH /api/management/repositories/{tenant}/{repo}/branches/{name}/upstream
   */
  setUpstream: (repoId: string, branchName: string, upstreamBranch: string | null) =>
    api.patch<Branch>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/branches/${branchName}/upstream`,
      { upstream_branch: upstreamBranch }
    ),
}

export const tagsApi = {
  /**
   * List all tags in a repository
   * GET /api/management/repositories/{tenant}/{repo}/tags
   */
  list: (repoId: string) =>
    api.get<Tag[]>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/tags`
    ),

  /**
   * Get a specific tag
   * GET /api/management/repositories/{tenant}/{repo}/tags/{name}
   */
  get: (repoId: string, name: string) =>
    api.get<Tag>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/tags/${name}`
    ),

  /**
   * Create a new tag
   * POST /api/management/repositories/{tenant}/{repo}/tags
   */
  create: (repoId: string, data: CreateTagRequest) =>
    api.post<Tag>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/tags`,
      data
    ),

  /**
   * Delete a tag
   * DELETE /api/management/repositories/{tenant}/{repo}/tags/{name}
   */
  delete: (repoId: string, name: string) =>
    api.delete<void>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/tags/${name}`
    ),
}
