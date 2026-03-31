import { api } from './client'

const DEFAULT_TENANT = 'default'

export interface Revision {
  number: string  // HLC format: "timestamp-counter" (e.g., "1762780281515-0")
  timestamp: string
  actor: string
  message: string
  is_system: boolean
  changed_nodes: string[]
  parent?: string
}

export interface OperationType {
  type: 'move' | 'copy' | 'rename' | 'reorder'
  from_path?: string
  from_parent_id?: string
  to_path?: string
  to_parent_id?: string
  source_id?: string
  source_path?: string
  destination_path?: string
  old_name?: string
  new_name?: string
  old_index?: string
  new_index?: string
}

export interface OperationMeta {
  operation: OperationType
  revision: string  // HLC format
  parent_revision?: string
  timestamp: string
  actor: string
  message: string
  is_system: boolean
  node_id: string
}

export interface RevisionMeta {
  revision: string  // HLC format: "timestamp-counter"
  branch: string
  timestamp: string
  actor: string
  message: string
  is_system: boolean
  changed_node_types: Array<{ name: string; operation: string }>
  changed_archetypes: Array<{ name: string; operation: string }>
  changed_element_types: Array<{ name: string; operation: string }>
  operation?: OperationMeta
}

export interface ListRevisionsResponse {
  revisions: RevisionMeta[]
  total: number
  has_more: boolean
}

export interface NodeChange {
  node_id: string
  operation: 'added' | 'modified' | 'deleted'
  node_type?: string
  path?: string
  translation_locale?: string
}

export const revisionsApi = {
  /**
   * List revisions in a repository
   * GET /api/management/repositories/{tenant}/{repo}/revisions?branch={branch}
   */
  list: async (repoId: string, limit = 50, offset = 0, includeSystem = false, branch?: string) => {
    const params = new URLSearchParams({
      limit: limit.toString(),
      offset: offset.toString(),
      include_system: includeSystem.toString(),
    })
    
    // Add branch filter if provided (Git-like: only show commits from this branch)
    if (branch) {
      params.append('branch', branch)
    }
    
    const response = await api.get<ListRevisionsResponse>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/revisions?${params.toString()}`
    )
    // Convert to legacy format for compatibility
    return response.revisions.map(r => ({
      number: r.revision,
      timestamp: r.timestamp,
      actor: r.actor,
      message: r.message,
      is_system: r.is_system,
      changed_nodes: [] as string[],
    }))
  },
  
  /**
   * Get details of a specific revision
   * GET /api/management/repositories/{tenant}/{repo}/revisions/{revision}
   */
  get: (repoId: string, revisionNumber: string) =>
    api.get<RevisionMeta>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/revisions/${revisionNumber}`
    ),

  /**
   * Get nodes changed in a revision
   * GET /api/management/repositories/{tenant}/{repo}/revisions/{revision}/changes
   */
  getChanges: (repoId: string, revisionNumber: string) =>
    api.get<NodeChange[]>(
      `/api/management/repositories/${DEFAULT_TENANT}/${repoId}/revisions/${revisionNumber}/changes`
    ),

  /**
   * Compare two revisions
   * Returns aggregated changes between two revisions
   * Note: Comparing HLC strings directly is not supported, this returns changes from both revisions
   */
  compare: async (repoId: string, fromRevision: string, toRevision: string) => {
    // Since HLC strings can't be compared numerically, we fetch changes for both revisions
    const allChanges: NodeChange[] = []

    try {
      const changes1 = await revisionsApi.getChanges(repoId, fromRevision)
      const changes2 = await revisionsApi.getChanges(repoId, toRevision)
      allChanges.push(...changes1, ...changes2)
    } catch (err) {
      console.warn(`Failed to fetch changes:`, err)
    }

    return {
      fromRevision,
      toRevision,
      changes: allChanges,
    }
  },
  
  /**
   * Get revision history for a specific node (not yet implemented in backend)
   */
  getNodeHistory: (_repo: string, _branch: string, _workspace: string, _path: string) =>
    Promise.reject(new Error('Node history not yet implemented')),
}
