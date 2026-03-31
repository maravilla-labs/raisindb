import { api } from './client'

export interface ElementTypeFieldSchema {
  [key: string]: unknown
}

export interface ElementType {
  id?: string
  name: string
  title?: string
  icon?: string
  description?: string
  extends?: string              // Parent element type for inheritance
  strict?: boolean              // Disallow undefined properties
  fields: ElementTypeFieldSchema[]
  initial_content?: Record<string, unknown>
  layout?: unknown
  meta?: Record<string, unknown>
  version?: number
  created_at?: string
  updated_at?: string
  published_at?: string
  published_by?: string
  publishable?: boolean
  previous_version?: string
}

export interface ElementTypeCommitPayload {
  message: string
  actor?: string
  is_system?: boolean
}

interface ElementTypeWriteRequest {
  element_type: ElementType
  commit?: ElementTypeCommitPayload
}

export interface ResolvedElementType {
  element_type: ElementType
  resolved_fields: ElementTypeFieldSchema[]
  resolved_layout: unknown[] | null
  inheritance_chain: string[]
  resolved_strict: boolean
}

export const elementTypesApi = {
  list: (repo: string, branch: string) =>
    api.get<ElementType[]>(`/api/management/${repo}/${branch}/elementtypes`),

  listPublished: (repo: string, branch: string) =>
    api.get<ElementType[]>(`/api/management/${repo}/${branch}/elementtypes/published`),

  get: (repo: string, branch: string, name: string) =>
    api.get<ElementType>(`/api/management/${repo}/${branch}/elementtypes/${name}`),

  create: (
    repo: string,
    branch: string,
    elementType: ElementType,
    commit?: ElementTypeCommitPayload,
  ) =>
    api.post<ElementType>(`/api/management/${repo}/${branch}/elementtypes`, {
      element_type: elementType,
      commit,
    } as ElementTypeWriteRequest),

  update: (
    repo: string,
    branch: string,
    name: string,
    elementType: ElementType,
    commit?: ElementTypeCommitPayload,
  ) =>
    api.put<ElementType>(`/api/management/${repo}/${branch}/elementtypes/${name}`, {
      element_type: elementType,
      commit,
    } as ElementTypeWriteRequest),

  delete: (repo: string, branch: string, name: string) =>
    api.delete<void>(`/api/management/${repo}/${branch}/elementtypes/${name}`),

  publish: (repo: string, branch: string, name: string, commit?: ElementTypeCommitPayload) =>
    api.post<ElementType>(
      `/api/management/${repo}/${branch}/elementtypes/${name}/publish`,
      commit
    ),

  getResolved: (repo: string, branch: string, name: string) =>
    api.get<ResolvedElementType>(
      `/api/management/${repo}/${branch}/elementtypes/${name}/resolved`
    ),

  unpublish: (
    repo: string,
    branch: string,
    name: string,
    commit?: ElementTypeCommitPayload
  ) =>
    api.post<ElementType>(
      `/api/management/${repo}/${branch}/elementtypes/${name}/unpublish`,
      commit
    ),
}
