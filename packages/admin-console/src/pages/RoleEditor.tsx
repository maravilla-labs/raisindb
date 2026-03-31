import { useEffect, useState } from 'react'
import { useNavigate, useParams, useSearchParams, Link } from 'react-router-dom'
import { ArrowLeft, Save, Shield } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import TagSelector from '../components/TagSelector'
import PermissionEditor from '../components/PermissionEditor'
import { useToast, ToastContainer } from '../components/Toast'
import { rolesApi, type Role } from '../api/roles'
import { nodeTypesApi } from '../api/nodetypes'
import { nodesApi } from '../api/nodes'

const WORKSPACE = 'raisin:access_control'

export default function RoleEditor() {
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()

  // Parse path from wildcard - e.g., "system/admin_role" for editing role in subfolder
  // Full workspace path becomes "/roles/system/admin_role"
  const pathParts = pathParam?.split('/').filter(Boolean) || []
  const isNew = !pathParam || pathParam === 'new'

  // For new roles, parentPath comes from searchParams
  // For editing, parentPath is derived from the URL path
  let rolePath: string | null = null
  let parentPath = '/roles'

  if (isNew) {
    // Creating new role - parentPath from query param
    const parentPathParam = searchParams.get('parentPath')
    if (parentPathParam) {
      const trimmed = parentPathParam.trim()
      if (trimmed) {
        parentPath = trimmed.startsWith('/roles') ? trimmed : `/roles${trimmed.startsWith('/') ? trimmed : `/${trimmed}`}`
      }
    }
  } else {
    // Editing existing role - extract from URL path
    pathParts.pop() // Last segment is the role name, remove it to get parent
    parentPath = pathParts.length > 0 ? `/roles/${pathParts.join('/')}` : '/roles'
    rolePath = `/roles/${pathParam}` // Full path for loading/updating
  }

  const [loading, setLoading] = useState(!isNew)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [formData, setFormData] = useState<
    Omit<Role, 'id' | 'created_at' | 'updated_at' | 'published_at' | 'published_by' | 'publishable' | 'previous_version'>
  >({
    role_id: '',
    name: '',
    description: '',
    inherits: [],
    permissions: [],
  })

  const [availableRoles, setAvailableRoles] = useState<string[]>([])
  const [availableNodeTypes, setAvailableNodeTypes] = useState<string[]>([])
  const { toasts, success: showSuccess, error: showError, closeToast } = useToast()
  const rolesListPath = repo ? `/${repo}/${activeBranch}${parentPath}` : '/'
  const permissionCount = formData.permissions?.length || 0
  const inheritsCount = formData.inherits?.length || 0

  useEffect(() => {
    if (!repo) return
    loadSuggestions()
    if (!isNew && rolePath) {
      loadRole()
    }
  }, [repo, activeBranch, rolePath, isNew])

  async function loadSuggestions() {
    if (!repo) return
    try {
      const [roles, nodeTypes] = await Promise.all([
        rolesApi.listAll(repo, activeBranch),
        nodeTypesApi.list(repo, activeBranch),
      ])
      // Filter out current role (if editing) - use form data role_id
      setAvailableRoles(roles.map((r) => r.role_id).filter((id) => id !== formData.role_id))
      const nodeTypeNames = Array.from(
        new Set(nodeTypes.map((type) => type.name).filter((name): name is string => Boolean(name)))
      ).sort((a, b) => a.localeCompare(b))
      setAvailableNodeTypes(nodeTypeNames)
    } catch (err) {
      console.error('Failed to load role suggestions:', err)
      setAvailableNodeTypes([])
    }
  }

  async function loadRole() {
    if (!repo || !rolePath) return
    setLoading(true)
    setError(null)

    try {
      // Use nodesApi directly with full workspace path
      const node = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, rolePath)
      setFormData({
        role_id: node.properties?.role_id as string || '',
        name: node.properties?.name as string || '',
        description: (node.properties?.description as string) || '',
        inherits: (node.properties?.inherits as string[]) || [],
        permissions: (node.properties?.permissions as any[]) || [],
      })
    } catch (err) {
      console.error('Failed to load role:', err)
      setError('Failed to load role')
    } finally {
      setLoading(false)
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!repo) return

    // Validate permissions
    const permissions = formData.permissions || []

    for (let idx = 0; idx < permissions.length; idx += 1) {
      const permission = permissions[idx]
      if (!permission.path || !permission.path.trim()) {
        setError('All permissions must have a path')
        return
      }
      if (!permission.operations || permission.operations.length === 0) {
        setError('All permissions must have at least one operation')
        return
      }
      if (permission.node_types && permission.node_types.length > 0) {
        const invalidTypes = permission.node_types.filter(
          (type) => !availableNodeTypes.includes(type)
        )
        if (invalidTypes.length > 0) {
          setError(
            `Permission ${idx + 1} references unknown node type(s): ${invalidTypes.join(', ')}`
          )
          return
        }
      }
    }

    setError(null)
    setSaving(true)

    try {
      const properties = {
        role_id: formData.role_id.trim(),
        name: formData.name.trim(),
        description: formData.description?.trim() || undefined,
        inherits: formData.inherits || [],
        permissions: formData.permissions || [],
      }

      const commit = {
        message: isNew ? `Create role ${properties.role_id}` : `Update role ${properties.role_id}`,
        actor: 'admin',
      }

      if (isNew) {
        // Create new role
        await nodesApi.create(repo, activeBranch, WORKSPACE, parentPath, {
          name: properties.role_id,
          node_type: 'raisin:Role',
          properties,
          commit,
        })
        showSuccess('Role Created', `Role "${properties.name}" was created successfully`)
      } else if (rolePath) {
        // Update existing role using full path
        await nodesApi.update(repo, activeBranch, WORKSPACE, rolePath, {
          properties,
          commit,
        })
        showSuccess('Role Updated', `Role "${properties.name}" was updated successfully`)
      }

      // Small delay to allow toast to be visible before navigation
      setTimeout(() => navigate(rolesListPath), 500)
    } catch (err: any) {
      console.error('Failed to save role:', err)
      const errorMessage = err.message || 'Failed to save role'
      setError(errorMessage)
      showError('Save Failed', errorMessage)
    } finally {
      setSaving(false)
    }
  }

  if (loading) {
    return (
      <div className="animate-fade-in">
        <div className="text-center text-zinc-400 py-12">Loading role...</div>
      </div>
    )
  }

  return (
    <div className="animate-fade-in space-y-6">
      <div className="flex items-center gap-4">
        <Link
          to={rolesListPath}
          className="p-2 hover:bg-white/10 rounded-lg transition-colors"
        >
          <ArrowLeft className="w-6 h-6 text-zinc-400" />
        </Link>
        <div>
          <h1 className="text-4xl font-bold text-white flex items-center gap-3">
            <Shield className="w-10 h-10 text-primary-400" />
            {isNew ? 'New Role' : `Edit Role: ${formData.role_id}`}
          </h1>
          <p className="text-zinc-400 mt-2">
            {isNew ? 'Create a new role with permissions' : 'Update role information and permissions'}
          </p>
        </div>
      </div>

      {error && (
        <div className="mb-6 p-4 bg-red-500/20 border border-red-500/30 rounded-lg text-red-300">
          {error}
        </div>
      )}

      <form onSubmit={handleSubmit} className="space-y-6">
        <div className="sticky top-16 z-30 flex flex-col gap-3 rounded-xl bg-zinc-900/70 p-3 shadow-lg backdrop-blur md:flex-row md:items-center md:justify-between xl:hidden">
          <div className="space-y-1">
            <div className="text-sm font-medium text-white">
              {isNew ? 'Create and publish this role' : 'Save updates'}
            </div>
            <p className="text-xs text-zinc-400">
              Actions stay within reach while you review permissions on smaller screens.
            </p>
          </div>
          <div className="flex flex-col gap-2 md:flex-row">
            <button
              type="submit"
              disabled={saving}
              className="flex items-center justify-center gap-2 rounded-lg bg-primary-500 px-6 py-2 text-sm text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:bg-primary-500/50"
            >
              <Save className="h-4 w-4" />
              {saving ? 'Saving...' : isNew ? 'Create Role' : 'Update Role'}
            </button>
            <Link
              to={rolesListPath}
              className="flex items-center justify-center rounded-lg bg-white/10 px-6 py-2 text-sm text-zinc-300 transition-colors hover:bg-white/20"
            >
              Cancel
            </Link>
          </div>
        </div>

        <div className="grid gap-6 xl:grid-cols-[minmax(0,2.4fr)_minmax(260px,1fr)]">
          <GlassCard className="space-y-10">
            <section className="space-y-6">
              <div>
                <h2 className="text-lg font-semibold text-white">Role Details</h2>
                <p className="text-sm text-zinc-400">
                  Capture the essentials so team members know exactly what this role governs.
                </p>
              </div>
              <div className="grid gap-4 md:grid-cols-2">
                <div className="space-y-2">
                  <label htmlFor="role_id" className="block text-sm font-medium text-zinc-300">
                    Role ID <span className="text-red-400">*</span>
                  </label>
                  <input
                    type="text"
                    id="role_id"
                    required
                    disabled={!isNew}
                    value={formData.role_id}
                    onChange={(e) => setFormData({ ...formData, role_id: e.target.value })}
                    className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none disabled:cursor-not-allowed disabled:opacity-50"
                    placeholder="role_editor"
                  />
                  <p className="text-xs text-zinc-500">Unique identifier visible in config files.</p>
                </div>
                <div className="space-y-2">
                  <label htmlFor="name" className="block text-sm font-medium text-zinc-300">
                    Name <span className="text-red-400">*</span>
                  </label>
                  <input
                    type="text"
                    id="name"
                    required
                    value={formData.name}
                    onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                    className="w-full rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
                    placeholder="Content Editor"
                  />
                </div>
                <div className="md:col-span-2 space-y-2">
                  <label htmlFor="description" className="block text-sm font-medium text-zinc-300">
                    Description
                  </label>
                  <textarea
                    id="description"
                    value={formData.description}
                    onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                    rows={3}
                    className="w-full resize-none rounded-lg border border-white/10 bg-white/5 px-4 py-2 text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
                    placeholder="Can read and edit content nodes"
                  />
                </div>
              </div>
            </section>

            <section className="space-y-4">
              <div className="flex flex-col gap-2 md:flex-row md:items-center md:justify-between">
                <div>
                  <h2 className="text-lg font-semibold text-white">Permissions</h2>
                  <p className="text-sm text-zinc-400">
                    Define which paths this role can touch and the operations they can take.
                  </p>
                </div>
                {permissionCount > 0 && (
                  <span className="inline-flex items-center rounded-full border border-white/10 bg-white/10 px-3 py-1 text-xs text-zinc-300">
                    {permissionCount} configured
                  </span>
                )}
              </div>
              <PermissionEditor
                permissions={formData.permissions || []}
                onChange={(permissions) => setFormData({ ...formData, permissions })}
                nodeTypeOptions={availableNodeTypes}
                repo={repo}
                branch={activeBranch}
              />
            </section>
          </GlassCard>

          <div className="space-y-6">
            <GlassCard className="space-y-4">
              <div>
                <h2 className="text-lg font-semibold text-white">Inheritance</h2>
                <p className="text-sm text-zinc-400">
                  Layer existing roles to reuse permission sets and reduce duplication.
                </p>
              </div>
              <TagSelector
                label="Inherits From"
                value={formData.inherits || []}
                onChange={(inherits) => setFormData({ ...formData, inherits })}
                placeholder="Add role to inherit from..."
                suggestions={availableRoles}
                helperText="Press Enter to add a role ID. Inherited permissions stack automatically."
              />
            </GlassCard>

            <GlassCard className="hidden space-y-5 xl:sticky xl:top-24 xl:block">
              <div className="space-y-1">
                <h2 className="text-lg font-semibold text-white">Review & Save</h2>
                <p className="text-sm text-zinc-400">
                  Double-check what&apos;s configured, then publish your changes.
                </p>
              </div>
              <dl className="space-y-2 text-sm text-zinc-300">
                <div className="flex items-center justify-between">
                  <dt className="text-zinc-400">Role ID</dt>
                  <dd className="truncate text-white">{formData.role_id || 'Not set'}</dd>
                </div>
                <div className="flex items-center justify-between">
                  <dt className="text-zinc-400">Inherited roles</dt>
                  <dd className="text-white">{inheritsCount}</dd>
                </div>
                <div className="flex items-center justify-between">
                  <dt className="text-zinc-400">Permissions</dt>
                  <dd className="text-white">{permissionCount}</dd>
                </div>
              </dl>
              <div className="flex flex-col gap-3">
                <button
                  type="submit"
                  disabled={saving}
                  className="flex items-center justify-center gap-2 rounded-lg bg-primary-500 px-6 py-2 text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:bg-primary-500/50"
                >
                  <Save className="h-5 w-5" />
                  {saving ? 'Saving...' : isNew ? 'Create Role' : 'Update Role'}
                </button>
                <Link
                  to={rolesListPath}
                  className="flex items-center justify-center rounded-lg bg-white/10 px-6 py-2 text-zinc-300 transition-colors hover:bg-white/20"
                >
                  Cancel
                </Link>
              </div>
            </GlassCard>
          </div>
        </div>
      </form>

      {/* Toast Notifications */}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
