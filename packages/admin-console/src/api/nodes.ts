// SPDX-License-Identifier: BSL-1.1

import { api, requestRaw } from './client'

export interface Node {
  id: string
  name: string
  path: string
  node_type: string
  archetype?: string         // Archetype name if node was created from an archetype
  properties?: Record<string, unknown>
  children?: Node[]
  has_children?: boolean     // Whether node has children (computed server-side)
  created_at?: string
  updated_at?: string
  published_at?: string
  version?: number           // Node version number
  revision?: number          // Repository revision
}

export interface CreateNodeRequest {
  name: string
  node_type: string
  archetype?: string           // Optional archetype to use for creation
  properties?: Record<string, unknown>
  commit?: {
    message: string
    actor?: string
  }
}

export interface UpdateNodeRequest {
  properties?: Record<string, unknown>
  translations?: Record<string, unknown>
  commit?: {
    message: string
    actor?: string
  }
}

export interface DeleteNodeRequest {
  commit?: {
    message: string
    actor?: string
  }
}

export interface MoveNodeRequest {
  destination: string
  commit?: {
    message: string
    actor?: string
  }
}

export interface RenameNodeRequest {
  newName: string
  commit?: {
    message: string
    actor?: string
  }
}

export interface CopyNodeRequest {
  destination: string
  name?: string
  commit?: {
    message: string
    actor?: string
  }
}

export interface ReorderNodeRequest {
  targetPath: string      // The sibling node to reorder relative to
  position?: 'before' | 'after'  // Position relative to target (default: 'after')
  commit?: {
    message: string
    actor?: string
  }
}

export interface UploadFileRequest {
  file: Blob
  fileName: string
  childName?: string
  propertyPath?: string
  nodeType?: string
  inline?: boolean
  overrideExisting?: boolean
  commitMessage?: string
  commitActor?: string
}

// ===== RELATIONSHIP TYPES =====

export interface RelationRef {
  target: string         // Target node ID
  workspace: string      // Target workspace
  relation_type: string  // Target node type
  weight?: number        // Optional weight for graph algorithms
}

export interface IncomingRelation {
  source_workspace: string  // Workspace containing the source node
  source_node_id: string    // ID of the source node
  target: string            // Target node ID (this node)
  workspace: string         // Target workspace (this workspace)
  relation_type: string     // Target node type
  weight?: number           // Optional weight
}

export interface NodeRelationships {
  outgoing: RelationRef[]      // Relationships FROM this node
  incoming: IncomingRelation[] // Relationships TO this node
}

export interface AddRelationRequest {
  targetWorkspace: string  // Workspace containing the target node
  targetPath: string       // Path to the target node
  weight?: number          // Optional weight for graph algorithms
  relationType?: string    // Optional custom relation type (defaults to target node type)
}

export interface RemoveRelationRequest {
  targetWorkspace: string  // Workspace containing the target node
  targetPath: string       // Path to the target node
}

export const nodesApi = {
  // ===== HEAD OPERATIONS (current/mutable state) =====

  /**
   * List root nodes at HEAD
   * GET /api/repository/{repo}/{branch}/head/{ws}/
   */
  listRootAtHead: (repo: string, branch: string, workspace: string, locale?: string) => {
    const url = `/api/repository/${repo}/${branch}/head/${workspace}/`
    return api.get<Node[]>(locale ? `${url}?lang=${locale}` : url)
  },

  /**
   * Get node by path at HEAD
   * GET /api/repository/{repo}/{branch}/head/{ws}/{path}
   */
  getAtHead: (repo: string, branch: string, workspace: string, path: string, locale?: string) => {
    const url = `/api/repository/${repo}/${branch}/head/${workspace}${path}`
    return api.get<Node>(locale ? `${url}?lang=${locale}` : url)
  },

  /**
   * Get node by ID at HEAD
   * GET /api/repository/{repo}/{branch}/head/{ws}/$ref/{id}
   */
  getByIdAtHead: (repo: string, branch: string, workspace: string, id: string, locale?: string) => {
    const url = `/api/repository/${repo}/${branch}/head/${workspace}/$ref/${id}`
    return api.get<Node>(locale ? `${url}?lang=${locale}` : url)
  },

  /**
   * List children at HEAD (with trailing slash)
   * GET /api/repository/{repo}/{branch}/head/{ws}/{path}/
   */
  listChildrenAtHead: (repo: string, branch: string, workspace: string, parentPath: string, locale?: string) => {
    // Normalize path to avoid double slashes (e.g., when parentPath is '/')
    const normalizedPath = parentPath === '/' ? '' : parentPath
    const url = `/api/repository/${repo}/${branch}/head/${workspace}${normalizedPath}/`
    return api.get<Node[]>(locale ? `${url}?lang=${locale}` : url)
  },

  // ===== REVISION OPERATIONS (immutable/time-travel) =====

  /**
   * List root nodes at specific revision
   * GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/
   */
  listRootAtRevision: (repo: string, branch: string, workspace: string, revision: string, locale?: string) => {
    const url = `/api/repository/${repo}/${branch}/rev/${revision}/${workspace}/`
    return api.get<Node[]>(locale ? `${url}?lang=${locale}` : url)
  },

  /**
   * Get node by path at specific revision
   * GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/{path}
   */
  getAtRevision: (repo: string, branch: string, workspace: string, path: string, revision: string, locale?: string) => {
    const url = `/api/repository/${repo}/${branch}/rev/${revision}/${workspace}${path}`
    return api.get<Node>(locale ? `${url}?lang=${locale}` : url)
  },

  /**
   * Get node by ID at specific revision
   * GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/$ref/{id}
   */
  getByIdAtRevision: (repo: string, branch: string, workspace: string, id: string, revision: string, locale?: string) => {
    const url = `/api/repository/${repo}/${branch}/rev/${revision}/${workspace}/$ref/${id}`
    return api.get<Node>(locale ? `${url}?lang=${locale}` : url)
  },

  /**
   * List children at specific revision
   * GET /api/repository/{repo}/{branch}/rev/{revision}/{ws}/{path}/
   */
  listChildrenAtRevision: (repo: string, branch: string, workspace: string, parentPath: string, revision: string, locale?: string) => {
    // Normalize path to avoid double slashes (e.g., when parentPath is '/')
    const normalizedPath = parentPath === '/' ? '' : parentPath
    const url = `/api/repository/${repo}/${branch}/rev/${revision}/${workspace}${normalizedPath}/`
    return api.get<Node[]>(locale ? `${url}?lang=${locale}` : url)
  },

  // ===== MUTATION OPERATIONS (create, update, delete - HEAD only) =====

  /**
   * Create root node
   * POST /api/repository/{repo}/{branch}/head/{ws}/
   */
  createRoot: (repo: string, branch: string, workspace: string, request: CreateNodeRequest) =>
    api.post<Node>(`/api/repository/${repo}/${branch}/head/${workspace}/`, request),

  /**
   * Create child node
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}
   */
  create: (repo: string, branch: string, workspace: string, parentPath: string, request: CreateNodeRequest) =>
    api.post<Node>(`/api/repository/${repo}/${branch}/head/${workspace}${parentPath}`, request),

  /**
   * Update node
   * PUT /api/repository/{repo}/{branch}/head/{ws}/{path}
   */
  update: (repo: string, branch: string, workspace: string, path: string, request: UpdateNodeRequest) =>
    api.put<Node>(`/api/repository/${repo}/${branch}/head/${workspace}${path}`, request),

  /**
   * Delete node
   * DELETE /api/repository/{repo}/{branch}/head/{ws}/{path}
   */
  delete: (repo: string, branch: string, workspace: string, path: string, request?: DeleteNodeRequest) =>
    api.delete(`/api/repository/${repo}/${branch}/head/${workspace}${path}`, request),

  // ===== NODE COMMANDS (HEAD only) =====

  /**
   * Move node to new location
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/move
   */
  move: (repo: string, branch: string, workspace: string, path: string, request: MoveNodeRequest) => {
    const body: Record<string, string> = {
      targetPath: request.destination
    }

    // Only add commit fields if they're defined
    if (request.commit?.message) {
      body.message = request.commit.message
    }
    if (request.commit?.actor) {
      body.actor = request.commit.actor
    }

    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/move`,
      body
    )
  },

  /**
   * Rename node
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/rename
   */
  rename: (repo: string, branch: string, workspace: string, path: string, request: RenameNodeRequest) => {
    const body: Record<string, string> = {
      newName: request.newName,
    }

    if (request.commit?.message) {
      body.message = request.commit.message
    }
    if (request.commit?.actor) {
      body.actor = request.commit.actor
    }

    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/rename`,
      body
    )
  },

  /**
   * Copy node to new location
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/copy
   */
  copy: (repo: string, branch: string, workspace: string, path: string, request: CopyNodeRequest) => {
    const body: Record<string, string> = {
      targetPath: request.destination
    }

    // Only add optional fields if they're defined
    if (request.name) {
      body.newName = request.name
    }
    if (request.commit?.message) {
      body.message = request.commit.message
    }
    if (request.commit?.actor) {
      body.actor = request.commit.actor
    }

    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/copy`,
      body
    )
  },

  /**
   * Copy node tree (recursive) to new location
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/copy_tree
   */
  copyTree: (repo: string, branch: string, workspace: string, path: string, request: CopyNodeRequest) => {
    const body: Record<string, string> = {
      targetPath: request.destination
    }

    // Only add optional fields if they're defined
    if (request.name) {
      body.newName = request.name
    }
    if (request.commit?.message) {
      body.message = request.commit.message
    }
    if (request.commit?.actor) {
      body.actor = request.commit.actor
    }

    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/copy_tree`,
      body
    )
  },

  /**
   * Reorder node relative to a sibling
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/reorder
   */
  reorder: (repo: string, branch: string, workspace: string, path: string, request: ReorderNodeRequest) => {
    const body: Record<string, string> = {
      targetPath: request.targetPath,
      movePosition: request.position || 'after'
    }

    // Only add commit fields if they're defined
    if (request.commit?.message) {
      body.message = request.commit.message
    }
    if (request.commit?.actor) {
      body.actor = request.commit.actor
    }

    return api.post(
      `/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/reorder`,
      body
    )
  },

  /**
   * Upload a file to a node property (Resource or inline string).
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}
   *
   * `path` should point to the target node (e.g., `/function/file.js` for Assets).
   * If the node doesn't exist and a commit message is provided, the backend
   * auto-creates a node (default: raisin:Asset or `nodeType` hint) and writes
   * the file to `propertyPath` (default: "file").
   */
  uploadFile: (repo: string, branch: string, workspace: string, path: string, request: UploadFileRequest) => {
    const formData = new FormData()
    formData.append('file', request.file, request.fileName)

    const params = new URLSearchParams()
    if (request.overrideExisting) params.set('override_existing', 'true')
    if (request.childName) params.set('new_name', request.childName)
    if (request.inline) params.set('inline', 'true')
    if (request.commitMessage) params.set('commit_message', request.commitMessage)
    if (request.commitActor) params.set('commit_actor', request.commitActor)
    if (request.propertyPath) params.set('property_path', request.propertyPath)
    if (request.nodeType) params.set('node_type', request.nodeType)

    const normalizedPath = path.endsWith('/') ? path.slice(0, -1) : path
    const baseUrl = `/api/repository/${repo}/${branch}/head/${workspace}${normalizedPath}`
    const url = params.toString() ? `${baseUrl}?${params.toString()}` : baseUrl

    return api.post(url, formData)
  },

  /**
   * Download file content from a node property (Resource)
   * GET /api/repository/{repo}/{branch}/head/{ws}/{path}@{propertyPath}
   *
   * Uses auto-detect streaming: accessing a Resource property via @propertyPath
   * automatically streams the content with appropriate Content-Type header.
   *
   * Returns the raw file content as text or blob
   */
  downloadFile: async (
    repo: string,
    branch: string,
    workspace: string,
    path: string,
    propertyPath = 'file',
    asText = true
  ): Promise<string | Blob> => {
    // Use @propertyPath syntax - auto-detect streaming returns Resource content directly
    // Use requestRaw to include auth + impersonation headers
    const url = `/api/repository/${repo}/${branch}/head/${workspace}${path}@${propertyPath}`
    const response = await requestRaw(url, { method: 'GET' })

    if (asText) {
      return response.text()
    }
    return response.blob()
  },

  /**
   * Publish node
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/publish
   */
  publish: (repo: string, branch: string, workspace: string, path: string) =>
    api.post(`/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/publish`, {}),

  /**
   * Unpublish node
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/unpublish
   */
  unpublish: (repo: string, branch: string, workspace: string, path: string) =>
    api.post(`/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/unpublish`, {}),

  // ===== RELATIONSHIP OPERATIONS =====

  /**
   * Get all relationships for a node (incoming and outgoing)
   * GET /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/relations
   */
  getRelationships: (repo: string, branch: string, workspace: string, path: string) =>
    api.get<NodeRelationships>(`/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/relations`),

  /**
   * Add a relationship from this node to a target node
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/add-relation
   */
  addRelation: (repo: string, branch: string, workspace: string, path: string, request: AddRelationRequest) =>
    api.post(`/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/add-relation`, request),

  /**
   * Remove a relationship from this node to a target node
   * POST /api/repository/{repo}/{branch}/head/{ws}/{path}/raisin:cmd/remove-relation
   */
  removeRelation: (repo: string, branch: string, workspace: string, path: string, request: RemoveRelationRequest) =>
    api.post<boolean>(`/api/repository/${repo}/${branch}/head/${workspace}${path}/raisin:cmd/remove-relation`, request),

  // ===== QUERY OPERATIONS =====

  /**
   * Execute a query using JSON DSL
   * POST /api/repository/{repo}/{branch}/head/{ws}/query/dsl
   * Returns a Page object with items and page metadata
   */
  queryDsl: async (repo: string, branch: string, workspace: string, query: any): Promise<Node[]> => {
    const response = await api.post<{ items: Node[], page: any }>(`/api/repository/${repo}/${branch}/head/${workspace}/query/dsl`, query)
    return response.items
  },

  // ===== BACKWARD COMPATIBILITY (deprecated) =====

  /**
   * @deprecated Use listRootAtHead instead
   */
  listRoot: (repo: string, branch: string, workspace: string) => {
    console.warn('nodesApi.listRoot is deprecated, use listRootAtHead instead')
    return api.get<Node[]>(`/api/repository/${repo}/${branch}/head/${workspace}/`)
  },

  /**
   * @deprecated Use getAtHead instead
   */
  get: (repo: string, branch: string, workspace: string, path: string) => {
    console.warn('nodesApi.get is deprecated, use getAtHead instead')
    return api.get<Node>(`/api/repository/${repo}/${branch}/head/${workspace}${path}`)
  },

  /**
   * @deprecated Use listChildrenAtHead instead
   */
  listChildren: (repo: string, branch: string, workspace: string, parentPath: string) => {
    console.warn('nodesApi.listChildren is deprecated, use listChildrenAtHead instead')
    const normalizedPath = parentPath === '/' ? '' : parentPath
    return api.get<Node[]>(`/api/repository/${repo}/${branch}/head/${workspace}${normalizedPath}/`)
  },

  /**
   * @deprecated Use getByIdAtHead instead
   */
  getById: (repo: string, branch: string, workspace: string, id: string) => {
    console.warn('nodesApi.getById is deprecated, use getByIdAtHead instead')
    return api.get<Node>(`/api/repository/${repo}/${branch}/head/${workspace}/$ref/${id}`)
  },
}
