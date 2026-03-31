import { useEffect, useState } from 'react'
import { useNavigate, useParams, useSearchParams, Link } from 'react-router-dom'
import { ArrowLeft, Save, User as UserIcon } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import TagSelector from '../components/TagSelector'
import { useToast, ToastContainer } from '../components/Toast'
import type { User } from '../api/users'
import { rolesApi } from '../api/roles'
import { groupsApi } from '../api/groups'
import { nodesApi } from '../api/nodes'

const WORKSPACE = 'raisin:access_control'

export default function UserEditor() {
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()

  // Parse path from wildcard - e.g., "internal/senol" for editing user in subfolder
  // Full workspace path becomes "/users/internal/senol"
  const pathParts = pathParam?.split('/').filter(Boolean) || []
  const isNew = !pathParam || pathParam === 'new'

  // For new users, parentPath comes from searchParams
  // For editing, parentPath is derived from the URL path
  let userPath: string | null = null
  let parentPath = '/users'

  if (isNew) {
    // Creating new user - parentPath from query param
    const parentPathParam = searchParams.get('parentPath')
    if (parentPathParam) {
      const trimmed = parentPathParam.trim()
      if (trimmed) {
        parentPath = trimmed.startsWith('/users') ? trimmed : `/users${trimmed.startsWith('/') ? trimmed : `/${trimmed}`}`
      }
    }
  } else {
    // Editing existing user - extract from URL path
    pathParts.pop() // Last segment is the user name, remove it to get parent
    parentPath = pathParts.length > 0 ? `/users/${pathParts.join('/')}` : '/users'
    userPath = `/users/${pathParam}` // Full path for loading/updating
  }

  const listRoute = repo ? `/${repo}/${activeBranch}${parentPath}` : '/'

  const [loading, setLoading] = useState(!isNew)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [formData, setFormData] = useState<Omit<User, 'id' | 'created_at' | 'updated_at'>>({
    user_id: '',
    email: '',
    display_name: '',
    groups: [],
    roles: [],
    metadata: {},
  })

  const [availableRoles, setAvailableRoles] = useState<string[]>([])
  const [availableGroups, setAvailableGroups] = useState<string[]>([])
  const { toasts, success: showSuccess, error: showError, closeToast } = useToast()

  useEffect(() => {
    if (!repo) return
    loadSuggestions()
    if (!isNew && userPath) {
      loadUser()
    }
  }, [repo, activeBranch, userPath, isNew])

  async function loadSuggestions() {
    if (!repo) return
    try {
      const [roles, groups] = await Promise.all([
        rolesApi.listAll(repo, activeBranch),
        groupsApi.listAll(repo, activeBranch),
      ])
      setAvailableRoles(roles.map((r) => r.role_id))
      setAvailableGroups(groups.map((g) => g.group_id))
    } catch (err) {
      console.error('Failed to load suggestions:', err)
    }
  }

  async function loadUser() {
    if (!repo || !userPath) return
    setLoading(true)
    setError(null)

    try {
      // Use nodesApi directly with full workspace path
      const node = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, userPath)
      setFormData({
        user_id: node.properties?.user_id as string || '',
        email: node.properties?.email as string || '',
        display_name: node.properties?.display_name as string || '',
        groups: (node.properties?.groups as string[]) || [],
        roles: (node.properties?.roles as string[]) || [],
        metadata: (node.properties?.metadata as Record<string, unknown>) || {},
      })
    } catch (err) {
      console.error('Failed to load user:', err)
      setError('Failed to load user')
    } finally {
      setLoading(false)
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!repo) return

    setError(null)
    setSaving(true)

    try {
      const properties = {
        user_id: formData.user_id.trim(),
        email: formData.email.trim(),
        display_name: formData.display_name.trim(),
        groups: formData.groups || [],
        roles: formData.roles || [],
        metadata: formData.metadata || {},
      }

      const commit = {
        message: isNew ? `Create user ${properties.user_id}` : `Update user ${properties.user_id}`,
        actor: 'admin',
      }

      if (isNew) {
        // Create new user
        await nodesApi.create(repo, activeBranch, WORKSPACE, parentPath, {
          name: properties.user_id,
          node_type: 'raisin:User',
          properties,
          commit,
        })
        showSuccess('User Created', `User "${properties.display_name}" was created successfully`)
      } else if (userPath) {
        // Update existing user using full path
        await nodesApi.update(repo, activeBranch, WORKSPACE, userPath, {
          properties,
          commit,
        })
        showSuccess('User Updated', `User "${properties.display_name}" was updated successfully`)
      }

      // Small delay to allow toast to be visible before navigation
      setTimeout(() => navigate(listRoute), 500)
    } catch (err: any) {
      console.error('Failed to save user:', err)
      const errorMessage = err.message || 'Failed to save user'
      setError(errorMessage)
      showError('Save Failed', errorMessage)
    } finally {
      setSaving(false)
    }
  }

  if (loading) {
    return (
      <div className="animate-fade-in">
        <div className="text-center text-zinc-400 py-12">Loading user...</div>
      </div>
    )
  }

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex items-center gap-4">
        <Link
          to={listRoute}
          className="p-2 hover:bg-white/10 rounded-lg transition-colors"
        >
          <ArrowLeft className="w-6 h-6 text-zinc-400" />
        </Link>
        <div>
          <h1 className="text-4xl font-bold text-white flex items-center gap-3">
            <UserIcon className="w-10 h-10 text-primary-400" />
            {isNew ? 'New User' : `Edit User: ${formData.user_id}`}
          </h1>
          <p className="text-zinc-400 mt-2">
            {isNew ? 'Create a new user account' : 'Update user information'}
          </p>
        </div>
      </div>

      {error && (
        <div className="mb-6 p-4 bg-red-500/20 border border-red-500/30 rounded-lg text-red-300">
          {error}
        </div>
      )}

      <GlassCard>
        <form onSubmit={handleSubmit} className="space-y-6">
          <div>
            <label htmlFor="user_id" className="block text-sm font-medium text-zinc-300 mb-2">
              User ID <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              id="user_id"
              required
              disabled={!isNew}
              value={formData.user_id}
              onChange={(e) => setFormData({ ...formData, user_id: e.target.value })}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none disabled:opacity-50 disabled:cursor-not-allowed"
              placeholder="user_john"
            />
            <p className="text-xs text-zinc-500 mt-1">Unique identifier for the user</p>
          </div>

          <div>
            <label htmlFor="email" className="block text-sm font-medium text-zinc-300 mb-2">
              Email <span className="text-red-400">*</span>
            </label>
            <input
              type="email"
              id="email"
              required
              value={formData.email}
              onChange={(e) => setFormData({ ...formData, email: e.target.value })}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
              placeholder="john@example.com"
            />
          </div>

          <div>
            <label htmlFor="display_name" className="block text-sm font-medium text-zinc-300 mb-2">
              Display Name <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              id="display_name"
              required
              value={formData.display_name}
              onChange={(e) => setFormData({ ...formData, display_name: e.target.value })}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
              placeholder="John Doe"
            />
          </div>

          <TagSelector
            label="Roles"
            value={formData.roles || []}
            onChange={(roles) => setFormData({ ...formData, roles })}
            placeholder="Add role..."
            suggestions={availableRoles}
          />

          <TagSelector
            label="Groups"
            value={formData.groups || []}
            onChange={(groups) => setFormData({ ...formData, groups })}
            placeholder="Add group..."
            suggestions={availableGroups}
          />

          <div className="flex items-center gap-4 pt-4">
            <button
              type="submit"
              disabled={saving}
              className="flex items-center gap-2 px-6 py-2 bg-primary-500 hover:bg-primary-600 disabled:bg-primary-500/50 text-white rounded-lg transition-colors"
            >
              <Save className="w-5 h-5" />
              {saving ? 'Saving...' : isNew ? 'Create User' : 'Update User'}
            </button>
            <Link
              to={listRoute}
              className="px-6 py-2 bg-white/10 hover:bg-white/20 text-zinc-300 rounded-lg transition-colors"
            >
              Cancel
            </Link>
          </div>
        </form>
      </GlassCard>

      {/* Toast Notifications */}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
