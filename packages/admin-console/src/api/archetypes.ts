import { api } from './client'

export interface FieldSchema {
  // Placeholder – actual field schema typing can be added later
  [key: string]: unknown
}

export interface Archetype {
  id?: string
  name: string
  extends?: string
  strict?: boolean              // Disallow undefined properties
  icon?: string
  title?: string
  description?: string
  base_node_type?: string
  fields?: FieldSchema[]
  initial_content?: Record<string, unknown>
  view?: Record<string, unknown>
  version?: number
  created_at?: string
  updated_at?: string
  published_at?: string
  published_by?: string
  publishable?: boolean
  previous_version?: string
}

export interface ArchetypeCommitPayload {
  message: string
  actor?: string
  is_system?: boolean
}

interface ArchetypeWriteRequest {
  archetype: Archetype
  commit?: ArchetypeCommitPayload
}

export interface ResolvedArchetype {
  archetype: Archetype
  resolved_fields: FieldSchema[]
  resolved_layout: unknown[] | null
  inheritance_chain: string[]
  resolved_strict: boolean
}

export const archetypesApi = {
  list: (repo: string, branch: string) =>
    api.get<Archetype[]>(`/api/management/${repo}/${branch}/archetypes`),

  listPublished: (repo: string, branch: string) =>
    api.get<Archetype[]>(`/api/management/${repo}/${branch}/archetypes/published`),

  get: (repo: string, branch: string, name: string) =>
    api.get<Archetype>(`/api/management/${repo}/${branch}/archetypes/${name}`),

  create: (
    repo: string,
    branch: string,
    archetype: Archetype,
    commit?: ArchetypeCommitPayload,
  ) =>
    api.post<Archetype>(`/api/management/${repo}/${branch}/archetypes`, {
      archetype,
      commit,
    } as ArchetypeWriteRequest),

  update: (
    repo: string,
    branch: string,
    name: string,
    archetype: Archetype,
    commit?: ArchetypeCommitPayload,
  ) =>
    api.put<Archetype>(`/api/management/${repo}/${branch}/archetypes/${name}`, {
      archetype,
      commit,
    } as ArchetypeWriteRequest),

  delete: (repo: string, branch: string, name: string) =>
    api.delete<void>(`/api/management/${repo}/${branch}/archetypes/${name}`),

  publish: (
    repo: string,
    branch: string,
    name: string,
    commit?: ArchetypeCommitPayload,
  ) => api.post<Archetype>(`/api/management/${repo}/${branch}/archetypes/${name}/publish`, commit),

  getResolved: (repo: string, branch: string, name: string) =>
    api.get<ResolvedArchetype>(
      `/api/management/${repo}/${branch}/archetypes/${name}/resolved`
    ),

  unpublish: (
    repo: string,
    branch: string,
    name: string,
    commit?: ArchetypeCommitPayload,
  ) => api.post<Archetype>(`/api/management/${repo}/${branch}/archetypes/${name}/unpublish`, commit),
}
