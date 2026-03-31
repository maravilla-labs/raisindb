import { api } from './client'

export interface Workspace {
  name: string
  description?: string
  allowed_node_types?: string[]
  allowed_root_node_types?: string[]
  depends_on?: string[]
  created_at?: string
  updated_at?: string
  config?: WorkspaceConfig
}

export type WorkspaceMode = 
  | { type: 'Versioned'; base_revision: number; auto_commit: boolean }
  | { type: 'Live'; keep_deltas: boolean; max_deltas: number }
  | { type: 'Ephemeral' }

export interface WorkspaceConfig {
  workspace_id: string
  mode: WorkspaceMode
  default_branch: string
  node_type_pins: Record<string, number | null> // null = latest, number = pinned revision
  // Legacy field for backward compatibility
  node_type_refs?: Record<string, number | null>
}

export interface PagedResponse<T> {
  items: T[]
  page: {
    total: number
    limit: number
    offset: number
    nextOffset: number | null
  }
}

export const workspacesApi = {
  list: async (repo: string) => {
    const response = await api.get<PagedResponse<Workspace>>(`/api/workspaces/${repo}`)
    return response.items
  },

  get: (repo: string, name: string) => api.get<Workspace>(`/api/workspaces/${repo}/${name}`),

  create: (repo: string, workspace: Workspace) =>
    api.put<Workspace>(`/api/workspaces/${repo}/${workspace.name}`, workspace),

  update: (repo: string, name: string, workspace: Workspace) =>
    api.put<Workspace>(`/api/workspaces/${repo}/${name}`, workspace),

  /**
   * Get workspace configuration
   * GET /api/workspaces/{repo}/{name}/config
   */
  getConfig: (repo: string, name: string) => api.get<WorkspaceConfig>(`/api/workspaces/${repo}/${name}/config`),

  /**
   * Update workspace configuration
   * PUT /api/workspaces/{repo}/{name}/config
   */
  updateConfig: (repo: string, name: string, config: WorkspaceConfig) =>
    api.put<void>(`/api/workspaces/${repo}/${name}/config`, config),
}
