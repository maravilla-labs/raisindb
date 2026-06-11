import { api } from './client'
import type { NodeType, NodeTypeCommitPayload } from './nodetypes'

// Mixins are NodeTypes with is_mixin=true, served by dedicated /mixins endpoints.
export type Mixin = NodeType

interface MixinWriteRequest {
  node_type: Mixin
  commit?: NodeTypeCommitPayload
}

export const mixinsApi = {
  list: (repo: string, branch: string) =>
    api.get<Mixin[]>(`/api/management/${repo}/${branch}/mixins`),

  listPublished: (repo: string, branch: string) =>
    api.get<Mixin[]>(`/api/management/${repo}/${branch}/mixins/published`),

  get: (repo: string, branch: string, name: string) =>
    api.get<Mixin>(`/api/management/${repo}/${branch}/mixins/${name}`),

  create: (
    repo: string,
    branch: string,
    mixin: Mixin,
    commit?: NodeTypeCommitPayload
  ) =>
    api.post<Mixin>(`/api/management/${repo}/${branch}/mixins`, {
      // Always flag as a mixin; the server enforces this too.
      node_type: { ...mixin, is_mixin: true },
      commit,
    } as MixinWriteRequest),

  update: (
    repo: string,
    branch: string,
    name: string,
    mixin: Mixin,
    commit?: NodeTypeCommitPayload
  ) =>
    api.put<Mixin>(`/api/management/${repo}/${branch}/mixins/${name}`, {
      node_type: { ...mixin, is_mixin: true },
      commit,
    } as MixinWriteRequest),

  delete: (repo: string, branch: string, name: string) =>
    api.delete(`/api/management/${repo}/${branch}/mixins/${name}`),

  publish: (repo: string, branch: string, name: string) =>
    api.post(`/api/management/${repo}/${branch}/mixins/${name}/publish`),

  unpublish: (repo: string, branch: string, name: string) =>
    api.post(`/api/management/${repo}/${branch}/mixins/${name}/unpublish`),
}
