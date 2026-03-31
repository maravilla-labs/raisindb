import { nodesApi, type Node, type CreateNodeRequest, type UpdateNodeRequest } from './nodes'

const WORKSPACE = 'raisin:access_control'

export interface User {
  id?: string
  user_id: string
  email: string
  display_name: string
  groups?: string[]
  roles?: string[]
  metadata?: Record<string, unknown>
  created_at?: string
  updated_at?: string
}

function nodeToUser(node: Node): User {
  return {
    id: node.id,
    user_id: node.properties?.user_id as string,
    email: node.properties?.email as string,
    display_name: node.properties?.display_name as string,
    groups: node.properties?.groups as string[] | undefined,
    roles: node.properties?.roles as string[] | undefined,
    metadata: node.properties?.metadata as Record<string, unknown> | undefined,
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

function userToProperties(user: User): Record<string, unknown> {
  return {
    user_id: user.user_id,
    email: user.email,
    display_name: user.display_name,
    groups: user.groups || [],
    roles: user.roles || [],
    metadata: user.metadata || {},
  }
}

export const usersApi = {
  list: async (repo: string, branch: string) => {
    const nodes = await nodesApi.listRootAtHead(repo, branch, WORKSPACE)
    return nodes.filter(n => n.node_type === 'raisin:User').map(nodeToUser)
  },

  get: async (repo: string, branch: string, userId: string) => {
    const node = await nodesApi.getAtHead(repo, branch, WORKSPACE, `/${userId}`)
    return nodeToUser(node)
  },

  create: async (
    repo: string,
    branch: string,
    user: User,
    commit?: { message: string; actor?: string },
    parentPath?: string
  ) => {
    const request: CreateNodeRequest = {
      name: user.user_id,
      node_type: 'raisin:User',
      properties: userToProperties(user),
      commit,
    }
    const node = parentPath
      ? await nodesApi.create(repo, branch, WORKSPACE, parentPath, request)
      : await nodesApi.createRoot(repo, branch, WORKSPACE, request)
    return nodeToUser(node)
  },

  update: async (
    repo: string,
    branch: string,
    userId: string,
    user: User,
    commit?: { message: string; actor?: string }
  ) => {
    const request: UpdateNodeRequest = {
      properties: userToProperties(user),
      commit,
    }
    const node = await nodesApi.update(repo, branch, WORKSPACE, `/${userId}`, request)
    return nodeToUser(node)
  },

  delete: (repo: string, branch: string, userId: string) =>
    nodesApi.delete(repo, branch, WORKSPACE, `/${userId}`),
}
