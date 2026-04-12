import { nodesApi, type Node, type CreateNodeRequest, type UpdateNodeRequest } from './nodes'
import { sqlApi } from './sql'

const WORKSPACE = 'raisin:access_control'

export interface Permission {
  // Scope fields (default to "*" if empty/omitted)
  workspace?: string        // e.g., "content", "marketing", "content-*", "*"
  branch_pattern?: string   // e.g., "main", "features/*", "release-*", "*"

  // Path pattern within scope
  path: string
  node_types?: string[]
  operations: string[]
  fields?: string[]
  except_fields?: string[]
  condition?: string // REL expression for ABAC
}

// Legacy condition types for migration
type LegacyCondition = Record<string, unknown>

interface LegacyPermission {
  workspace?: string
  branch_pattern?: string
  path: string
  node_types?: string[]
  operations: string[]
  fields?: string[]
  except_fields?: string[]
  conditions?: LegacyCondition | LegacyCondition[]
  condition?: string
}

// Convert legacy structured conditions to REL expression string
function migrateLegacyConditionToRel(condition: LegacyCondition): string {
  // property_equals: { key: 'author', value: '$auth.user_id' }
  if ('property_equals' in condition) {
    const c = condition.property_equals as { key: string; value: unknown }
    const value = formatRelValue(c.value)
    return `resource.${c.key} == ${value}`
  }

  // property_in: { key: 'status', values: ['draft', 'review'] }
  if ('property_in' in condition) {
    const c = condition.property_in as { key: string; values: unknown[] }
    const values = c.values.map(formatRelValue).join(', ')
    return `[${values}].contains(resource.${c.key})`
  }

  // property_greater_than: { key: 'priority', value: 5 }
  if ('property_greater_than' in condition) {
    const c = condition.property_greater_than as { key: string; value: unknown }
    const value = formatRelValue(c.value)
    return `resource.${c.key} > ${value}`
  }

  // property_less_than: { key: 'priority', value: 10 }
  if ('property_less_than' in condition) {
    const c = condition.property_less_than as { key: string; value: unknown }
    const value = formatRelValue(c.value)
    return `resource.${c.key} < ${value}`
  }

  // user_has_role: 'admin'
  if ('user_has_role' in condition) {
    const role = condition.user_has_role as string
    return `auth.roles.contains('${role}')`
  }

  // user_in_group: 'group_engineers'
  if ('user_in_group' in condition) {
    const group = condition.user_in_group as string
    return `auth.groups.contains('${group}')`
  }

  // all: [...conditions] (AND)
  if ('all' in condition) {
    const conditions = condition.all as LegacyCondition[]
    const parts = conditions.map(migrateLegacyConditionToRel)
    return parts.length > 1 ? `(${parts.join(' && ')})` : parts[0] || 'true'
  }

  // any: [...conditions] (OR)
  if ('any' in condition) {
    const conditions = condition.any as LegacyCondition[]
    const parts = conditions.map(migrateLegacyConditionToRel)
    return parts.length > 1 ? `(${parts.join(' || ')})` : parts[0] || 'false'
  }

  // Unknown condition type - return as comment
  return `/* unknown: ${JSON.stringify(condition)} */`
}

function formatRelValue(value: unknown): string {
  if (typeof value === 'string') {
    // Handle auth variable references like '$auth.user_id'
    if (value.startsWith('$auth.')) {
      return value.slice(1) // Remove $ prefix -> 'auth.user_id'
    }
    return `'${value.replace(/'/g, "\\'")}'`
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value)
  }
  if (value === null) {
    return 'null'
  }
  return JSON.stringify(value)
}

function migratePermission(perm: LegacyPermission): Permission {
  const { conditions, ...rest } = perm

  // If already has REL condition string, use it
  if (rest.condition) {
    return rest as Permission
  }

  // If no legacy conditions, return as-is
  if (!conditions) {
    return rest as Permission
  }

  // Convert legacy conditions to REL string
  const conditionArray = Array.isArray(conditions) ? conditions : [conditions]
  if (conditionArray.length === 0) {
    return rest as Permission
  }

  const relParts = conditionArray.map(migrateLegacyConditionToRel)
  const condition = relParts.length > 1 ? relParts.join(' && ') : relParts[0]

  return { ...rest, condition }
}

export interface Role {
  id?: string
  path?: string
  role_id: string
  name: string
  description?: string
  inherits?: string[]
  permissions?: Permission[]
  created_at?: string
  updated_at?: string
  published_at?: string
  published_by?: string
  publishable?: boolean
  previous_version?: string
}

function nodeToRole(node: Node): Role {
  // Migrate legacy permissions on load
  const rawPermissions = node.properties?.permissions as LegacyPermission[] | undefined
  const permissions = rawPermissions?.map(migratePermission)

  return {
    id: node.id,
    path: node.path,
    role_id: node.properties?.role_id as string,
    name: node.properties?.name as string,
    description: node.properties?.description as string | undefined,
    inherits: node.properties?.inherits as string[] | undefined,
    permissions,
    created_at: node.created_at,
    updated_at: node.updated_at,
    published_at: node.published_at,
    publishable: !!node.published_at,
  }
}

function roleToProperties(role: Role): Record<string, unknown> {
  return {
    role_id: role.role_id,
    name: role.name,
    description: role.description,
    inherits: role.inherits || [],
    permissions: role.permissions || [],
  }
}

export const rolesApi = {
  list: async (repo: string, branch: string) => {
    const nodes = await nodesApi.listRootAtHead(repo, branch, WORKSPACE)
    return nodes.filter(n => n.node_type === 'raisin:Role').map(nodeToRole)
  },

  /** List all roles from all folders using SQL search */
  listAll: async (repo: string, _branch: string): Promise<Role[]> => {
    const sql = `
      SELECT id, path, name,
             properties->>'role_id' as role_id,
             properties->>'name' as display_name,
             properties->>'description' as description
      FROM "raisin:access_control"
      WHERE node_type = 'raisin:Role'
      ORDER BY properties->>'role_id'
    `
    const response = await sqlApi.executeQuery(repo, sql, [])
    return response.rows.map(row => ({
      id: row.id,
      path: row.path,
      role_id: row.role_id || row.name,
      name: row.display_name || row.name,
      description: row.description,
    }))
  },

  listPublished: async (repo: string, branch: string) => {
    const nodes = await nodesApi.listRootAtHead(repo, branch, WORKSPACE)
    return nodes
      .filter(n => n.node_type === 'raisin:Role' && n.published_at)
      .map(nodeToRole)
  },

  get: async (repo: string, branch: string, rolePath: string) => {
    // Support both bare ID (legacy) and full path
    const path = rolePath.startsWith('/') ? rolePath : `/roles/${rolePath}`
    const node = await nodesApi.getAtHead(repo, branch, WORKSPACE, path)
    return nodeToRole(node)
  },

  create: async (
    repo: string,
    branch: string,
    role: Role,
    commit?: { message: string; actor?: string },
    parentPath?: string
  ) => {
    const request: CreateNodeRequest = {
      name: role.role_id,
      node_type: 'raisin:Role',
      properties: roleToProperties(role),
      commit,
    }
    const node = parentPath
      ? await nodesApi.create(repo, branch, WORKSPACE, parentPath, request)
      : await nodesApi.createRoot(repo, branch, WORKSPACE, request)
    return nodeToRole(node)
  },

  update: async (
    repo: string,
    branch: string,
    rolePath: string,
    role: Role,
    commit?: { message: string; actor?: string }
  ) => {
    const path = rolePath.startsWith('/') ? rolePath : `/roles/${rolePath}`
    const request: UpdateNodeRequest = {
      properties: roleToProperties(role),
      commit,
    }
    const node = await nodesApi.update(repo, branch, WORKSPACE, path, request)
    return nodeToRole(node)
  },

  delete: (repo: string, branch: string, rolePath: string) => {
    const path = rolePath.startsWith('/') ? rolePath : `/roles/${rolePath}`
    return nodesApi.delete(repo, branch, WORKSPACE, path)
  },

  publish: async (repo: string, branch: string, rolePath: string) => {
    const path = rolePath.startsWith('/') ? rolePath : `/roles/${rolePath}`
    await nodesApi.publish(repo, branch, WORKSPACE, path)
    return rolesApi.get(repo, branch, path)
  },

  unpublish: async (repo: string, branch: string, rolePath: string) => {
    const path = rolePath.startsWith('/') ? rolePath : `/roles/${rolePath}`
    await nodesApi.unpublish(repo, branch, WORKSPACE, path)
    return rolesApi.get(repo, branch, path)
  },
}
