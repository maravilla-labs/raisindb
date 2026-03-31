import { api } from './client'

export interface NodeType {
  id?: string
  name: string
  version?: number
  extends?: string
  mixins?: string[]
  strict?: boolean
  overrides?: any
  description?: string
  icon?: string
  properties?: any
  allowed_children?: string[]
  required_nodes?: string[]
  initial_structure?: any
  versionable?: boolean
  publishable?: boolean
  auditable?: boolean
  indexable?: boolean
  index_types?: any
  published?: boolean
  published_at?: string
  created_at?: string
  updated_at?: string
  previous_version?: string
}

export interface NodeTypeCommitPayload {
  message: string
  actor?: string
  is_system?: boolean
}

interface NodeTypeWriteRequest {
  node_type: NodeType
  commit?: NodeTypeCommitPayload
}

export interface PropertyDefinition {
  name: string
  type: string
  required?: boolean
  default?: any
  description?: string
  label?: string
  enum?: string[]
  minimum?: number
  maximum?: number
  minLength?: number
  maxLength?: number
  pattern?: string
  multiline?: boolean
  placeholder?: string
}

export interface ResolvedNodeType {
  node_type: NodeType
  resolved_properties: PropertyDefinition[]
  resolved_allowed_children: string[]
  inheritance_chain: string[]
}

export const nodeTypesApi = {
  list: (repo: string, branch: string) =>
    api.get<NodeType[]>(`/api/management/${repo}/${branch}/nodetypes`),

  listPublished: (repo: string, branch: string) =>
    api.get<NodeType[]>(`/api/management/${repo}/${branch}/nodetypes/published`),

  get: (repo: string, branch: string, name: string) =>
    api.get<NodeType>(`/api/management/${repo}/${branch}/nodetypes/${name}`),

  getResolved: (
    repo: string,
    branch: string,
    name: string,
    workspace?: string
  ) => {
    const params = workspace ? `?workspace=${encodeURIComponent(workspace)}` : ''
    return api.get<ResolvedNodeType>(
      `/api/management/${repo}/${branch}/nodetypes/${name}/resolved${params}`
    )
  },

  create: (
    repo: string,
    branch: string,
    nodeType: NodeType,
    commit?: NodeTypeCommitPayload
  ) =>
    api.post<NodeType>(`/api/management/${repo}/${branch}/nodetypes`, {
      node_type: nodeType,
      commit,
    } as NodeTypeWriteRequest),

  update: (
    repo: string,
    branch: string,
    name: string,
    nodeType: NodeType,
    commit?: NodeTypeCommitPayload
  ) =>
    api.put<NodeType>(`/api/management/${repo}/${branch}/nodetypes/${name}`, {
      node_type: nodeType,
      commit,
    } as NodeTypeWriteRequest),

  delete: (repo: string, branch: string, name: string) =>
    api.delete(`/api/management/${repo}/${branch}/nodetypes/${name}`),

  publish: (repo: string, branch: string, name: string) =>
    api.post(`/api/management/${repo}/${branch}/nodetypes/${name}/publish`),

  unpublish: (repo: string, branch: string, name: string) =>
    api.post(`/api/management/${repo}/${branch}/nodetypes/${name}/unpublish`),

  validate: (repo: string, branch: string, workspace: string, node: unknown) =>
    api.post(`/api/management/${repo}/${branch}/nodetypes/validate`, { workspace, node }),
}
