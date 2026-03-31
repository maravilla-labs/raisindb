import { useEffect, useState } from 'react'
import { useNavigate, useParams, useSearchParams, Link } from 'react-router-dom'
import { ArrowLeft, Save, Users } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import TagSelector from '../components/TagSelector'
import { useToast, ToastContainer } from '../components/Toast'
import type { Group } from '../api/groups'
import { rolesApi } from '../api/roles'
import { nodesApi } from '../api/nodes'

const WORKSPACE = 'raisin:access_control'

export default function GroupEditor() {
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()

  // Parse path from wildcard - e.g., "subfolder/group_name" for editing group in subfolder
  // Full workspace path becomes "/groups/subfolder/group_name"
  const pathParts = pathParam?.split('/').filter(Boolean) || []
  const isNew = !pathParam || pathParam === 'new'

  // For new groups, parentPath comes from searchParams
  // For editing, parentPath is derived from the URL path
  let groupPath: string | null = null
  let parentPath = '/groups'

  if (isNew) {
    // Creating new group - parentPath from query param
    const parentPathParam = searchParams.get('parentPath')
    if (parentPathParam) {
      const trimmed = parentPathParam.trim()
      if (trimmed) {
        parentPath = trimmed.startsWith('/groups') ? trimmed : `/groups${trimmed.startsWith('/') ? trimmed : `/${trimmed}`}`
      }
    }
  } else {
    // Editing existing group - extract from URL path
    pathParts.pop() // Last segment is the group name, remove it to get parent
    parentPath = pathParts.length > 0 ? `/groups/${pathParts.join('/')}` : '/groups'
    groupPath = `/groups/${pathParam}` // Full path for loading/updating
  }

  const listRoute = repo ? `/${repo}/${activeBranch}${parentPath}` : '/'

  const [loading, setLoading] = useState(!isNew)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const [formData, setFormData] = useState<Omit<Group, 'id' | 'created_at' | 'updated_at'>>({
    group_id: '',
    name: '',
    description: '',
    roles: [],
  })

  const [availableRoles, setAvailableRoles] = useState<string[]>([])
  const { toasts, success: showSuccess, error: showError, closeToast } = useToast()

  useEffect(() => {
    if (!repo) return
    loadSuggestions()
    if (!isNew && groupPath) {
      loadGroup()
    }
  }, [repo, activeBranch, groupPath, isNew])

  async function loadSuggestions() {
    if (!repo) return
    try {
      const roles = await rolesApi.listAll(repo, activeBranch)
      setAvailableRoles(roles.map((r) => r.role_id))
    } catch (err) {
      console.error('Failed to load roles:', err)
    }
  }

  async function loadGroup() {
    if (!repo || !groupPath) return
    setLoading(true)
    setError(null)

    try {
      // Use nodesApi directly with full workspace path
      const node = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, groupPath)
      setFormData({
        group_id: node.properties?.group_id as string || '',
        name: node.properties?.name as string || '',
        description: (node.properties?.description as string) || '',
        roles: (node.properties?.roles as string[]) || [],
      })
    } catch (err) {
      console.error('Failed to load group:', err)
      setError('Failed to load group')
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
        group_id: formData.group_id.trim(),
        name: formData.name.trim(),
        description: formData.description?.trim() || undefined,
        roles: formData.roles || [],
      }

      const commit = {
        message: isNew ? `Create group ${properties.group_id}` : `Update group ${properties.group_id}`,
        actor: 'admin',
      }

      if (isNew) {
        // Create new group
        await nodesApi.create(repo, activeBranch, WORKSPACE, parentPath, {
          name: properties.group_id,
          node_type: 'raisin:Group',
          properties,
          commit,
        })
        showSuccess('Group Created', `Group "${properties.name}" was created successfully`)
      } else if (groupPath) {
        // Update existing group using full path
        await nodesApi.update(repo, activeBranch, WORKSPACE, groupPath, {
          properties,
          commit,
        })
        showSuccess('Group Updated', `Group "${properties.name}" was updated successfully`)
      }

      // Small delay to allow toast to be visible before navigation
      setTimeout(() => navigate(listRoute), 500)
    } catch (err: any) {
      console.error('Failed to save group:', err)
      const errorMessage = err.message || 'Failed to save group'
      setError(errorMessage)
      showError('Save Failed', errorMessage)
    } finally {
      setSaving(false)
    }
  }

  if (loading) {
    return (
      <div className="animate-fade-in">
        <div className="text-center text-zinc-400 py-12">Loading group...</div>
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
            <Users className="w-10 h-10 text-primary-400" />
            {isNew ? 'New Group' : `Edit Group: ${formData.group_id}`}
          </h1>
          <p className="text-zinc-400 mt-2">
            {isNew ? 'Create a new user group' : 'Update group information'}
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
            <label htmlFor="group_id" className="block text-sm font-medium text-zinc-300 mb-2">
              Group ID <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              id="group_id"
              required
              disabled={!isNew}
              value={formData.group_id}
              onChange={(e) => setFormData({ ...formData, group_id: e.target.value })}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none disabled:opacity-50 disabled:cursor-not-allowed"
              placeholder="group_engineers"
            />
            <p className="text-xs text-zinc-500 mt-1">Unique identifier for the group</p>
          </div>

          <div>
            <label htmlFor="name" className="block text-sm font-medium text-zinc-300 mb-2">
              Name <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              id="name"
              required
              value={formData.name}
              onChange={(e) => setFormData({ ...formData, name: e.target.value })}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
              placeholder="Engineering Team"
            />
          </div>

          <div>
            <label htmlFor="description" className="block text-sm font-medium text-zinc-300 mb-2">
              Description
            </label>
            <textarea
              id="description"
              value={formData.description}
              onChange={(e) => setFormData({ ...formData, description: e.target.value })}
              rows={3}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none resize-none"
              placeholder="All engineers across projects"
            />
          </div>

          <TagSelector
            label="Roles"
            value={formData.roles || []}
            onChange={(roles) => setFormData({ ...formData, roles })}
            placeholder="Add role..."
            suggestions={availableRoles}
          />

          <div className="flex items-center gap-4 pt-4">
            <button
              type="submit"
              disabled={saving}
              className="flex items-center gap-2 px-6 py-2 bg-primary-500 hover:bg-primary-600 disabled:bg-primary-500/50 text-white rounded-lg transition-colors"
            >
              <Save className="w-5 h-5" />
              {saving ? 'Saving...' : isNew ? 'Create Group' : 'Update Group'}
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
