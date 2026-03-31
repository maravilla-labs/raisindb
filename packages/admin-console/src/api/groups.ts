import { nodesApi, type Node, type CreateNodeRequest, type UpdateNodeRequest } from './nodes'
import { sqlApi } from './sql'

const WORKSPACE = 'raisin:access_control'

export interface Group {
  id?: string
  group_id: string
  name: string
  description?: string
  roles?: string[]
  created_at?: string
  updated_at?: string
}

function nodeToGroup(node: Node): Group {
  return {
    id: node.id,
    group_id: node.properties?.group_id as string,
    name: node.properties?.name as string,
    description: node.properties?.description as string | undefined,
    roles: node.properties?.roles as string[] | undefined,
    created_at: node.created_at,
    updated_at: node.updated_at,
  }
}

function groupToProperties(group: Group): Record<string, unknown> {
  return {
    group_id: group.group_id,
    name: group.name,
    description: group.description,
    roles: group.roles || [],
  }
}

export const groupsApi = {
  list: async (repo: string, branch: string) => {
    const nodes = await nodesApi.listRootAtHead(repo, branch, WORKSPACE)
    return nodes.filter(n => n.node_type === 'raisin:Group').map(nodeToGroup)
  },

  /** List all groups from all folders using SQL search */
  listAll: async (repo: string, _branch: string): Promise<Group[]> => {
    const sql = `
      SELECT id, path, name,
             properties->>'group_id' as group_id,
             properties->>'name' as display_name,
             properties->>'description' as description
      FROM "raisin:access_control"
      WHERE node_type = 'raisin:Group'
      ORDER BY properties->>'group_id'
    `
    const response = await sqlApi.executeQuery(repo, sql, [])
    return response.rows.map(row => ({
      id: row.id,
      group_id: row.group_id || row.name,
      name: row.display_name || row.name,
      description: row.description,
    }))
  },

  get: async (repo: string, branch: string, groupId: string) => {
    const node = await nodesApi.getAtHead(repo, branch, WORKSPACE, `/${groupId}`)
    return nodeToGroup(node)
  },

  create: async (
    repo: string,
    branch: string,
    group: Group,
    commit?: { message: string; actor?: string },
    parentPath?: string
  ) => {
    const request: CreateNodeRequest = {
      name: group.group_id,
      node_type: 'raisin:Group',
      properties: groupToProperties(group),
      commit,
    }
    const node = parentPath
      ? await nodesApi.create(repo, branch, WORKSPACE, parentPath, request)
      : await nodesApi.createRoot(repo, branch, WORKSPACE, request)
    return nodeToGroup(node)
  },

  update: async (
    repo: string,
    branch: string,
    groupId: string,
    group: Group,
    commit?: { message: string; actor?: string }
  ) => {
    const request: UpdateNodeRequest = {
      properties: groupToProperties(group),
      commit,
    }
    const node = await nodesApi.update(repo, branch, WORKSPACE, `/${groupId}`, request)
    return nodeToGroup(node)
  },

  delete: (repo: string, branch: string, groupId: string) =>
    nodesApi.delete(repo, branch, WORKSPACE, `/${groupId}`),
}
