import { useEffect, useState, useRef } from 'react'
import { useNavigate, useParams, useSearchParams, Link } from 'react-router-dom'
import { ArrowLeft, Save, User as UserIcon, Search, Link as LinkIcon } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import TagSelector from '../components/TagSelector'
import { useToast, ToastContainer } from '../components/Toast'
import { useAuth } from '../contexts/AuthContext'
import type { User } from '../api/users'
import { rolesApi } from '../api/roles'
import { groupsApi } from '../api/groups'
import { nodesApi } from '../api/nodes'
import { identityUsersApi, IdentityUser } from '../api/identity-users'

const WORKSPACE = 'raisin:access_control'

export default function UserEditor() {
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const { tenantId } = useAuth()

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

  // Identity user linking
  const [linkMode, setLinkMode] = useState<'identity' | 'manual'>('identity')
  const [identitySearch, setIdentitySearch] = useState('')
  const [identityResults, setIdentityResults] = useState<IdentityUser[]>([])
  const [selectedIdentity, setSelectedIdentity] = useState<IdentityUser | null>(null)
  const [searchingIdentity, setSearchingIdentity] = useState(false)
  const [showIdentityDropdown, setShowIdentityDropdown] = useState(false)
  const searchTimeoutRef = useRef<ReturnType<typeof setTimeout>>()
  const dropdownRef = useRef<HTMLDivElement>(null)

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

  // Close dropdown on outside click
  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setShowIdentityDropdown(false)
      }
    }
    document.addEventListener('mousedown', handleClick)
    return () => document.removeEventListener('mousedown', handleClick)
  }, [])

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

  async function searchIdentityUsers(query: string) {
    if (!query.trim()) {
      setIdentityResults([])
      return
    }
    setSearchingIdentity(true)
    try {
      const results = await identityUsersApi.list(tenantId, { email: query })
      setIdentityResults(results)
      setShowIdentityDropdown(true)
    } catch (err) {
      console.error('Failed to search identity users:', err)
    } finally {
      setSearchingIdentity(false)
    }
  }

  function handleIdentitySearchChange(value: string) {
    setIdentitySearch(value)
    setSelectedIdentity(null)
    if (searchTimeoutRef.current) clearTimeout(searchTimeoutRef.current)
    searchTimeoutRef.current = setTimeout(() => searchIdentityUsers(value), 300)
  }

  function selectIdentity(identity: IdentityUser) {
    setSelectedIdentity(identity)
    setIdentitySearch(identity.email)
    setShowIdentityDropdown(false)
    setFormData({
      ...formData,
      user_id: identity.id,
      email: identity.email,
      display_name: identity.display_name || formData.display_name,
    })
  }

  async function loadUser() {
    if (!repo || !userPath) return
    setLoading(true)
    setError(null)

    try {
      // Use nodesApi directly with full workspace path
      const node = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, userPath)
      const userId = node.properties?.user_id as string || ''
      setFormData({
        user_id: userId,
        email: node.properties?.email as string || '',
        display_name: node.properties?.display_name as string || '',
        groups: (node.properties?.groups as string[]) || [],
        roles: (node.properties?.roles as string[]) || [],
        metadata: (node.properties?.metadata as Record<string, unknown>) || {},
      })
      // If there's a user_id, this user is linked to an identity
      if (userId) {
        setLinkMode('identity')
        setIdentitySearch(node.properties?.email as string || '')
      } else {
        setLinkMode('manual')
      }
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
        message: isNew ? `Create user ${properties.display_name || properties.email}` : `Update user ${properties.display_name || properties.email}`,
        actor: 'admin',
      }

      if (isNew) {
        // Use email-derived name for the node name (same convention as ensure_user_node)
        const nodeName = properties.email
          .toLowerCase()
          .replace(/@/g, '-at-')
          .replace(/\./g, '-')
          .replace(/[^a-z0-9-]/g, '')

        await nodesApi.create(repo, activeBranch, WORKSPACE, parentPath, {
          name: nodeName,
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
            {isNew ? 'New User' : `Edit User: ${formData.display_name || formData.email}`}
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
          {/* Identity linking toggle (only for new users) */}
          {isNew && (
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">User Type</label>
              <div className="flex gap-3">
                <button
                  type="button"
                  onClick={() => {
                    setLinkMode('identity')
                    setFormData({ ...formData, user_id: '' })
                  }}
                  className={`flex items-center gap-2 px-4 py-2 rounded-lg border transition-all ${
                    linkMode === 'identity'
                      ? 'bg-primary-500/20 border-primary-500/40 text-primary-300'
                      : 'bg-white/5 border-white/10 text-zinc-400 hover:bg-white/10'
                  }`}
                >
                  <LinkIcon className="w-4 h-4" />
                  Link to Identity User
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setLinkMode('manual')
                    setSelectedIdentity(null)
                    setIdentitySearch('')
                  }}
                  className={`flex items-center gap-2 px-4 py-2 rounded-lg border transition-all ${
                    linkMode === 'manual'
                      ? 'bg-primary-500/20 border-primary-500/40 text-primary-300'
                      : 'bg-white/5 border-white/10 text-zinc-400 hover:bg-white/10'
                  }`}
                >
                  <UserIcon className="w-4 h-4" />
                  Manual Entry
                </button>
              </div>
              <p className="text-xs text-zinc-500 mt-1">
                {linkMode === 'identity'
                  ? 'Search and select an existing identity user to link this repository user to'
                  : 'Create a repository-only user without linking to a tenant identity'}
              </p>
            </div>
          )}

          {/* Identity user search (when in identity mode) */}
          {linkMode === 'identity' && (
            <div ref={dropdownRef} className="relative">
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                Identity User <span className="text-red-400">*</span>
              </label>
              <div className="relative">
                <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-zinc-500" />
                <input
                  type="text"
                  value={identitySearch}
                  onChange={(e) => handleIdentitySearchChange(e.target.value)}
                  onFocus={() => identityResults.length > 0 && setShowIdentityDropdown(true)}
                  className="w-full pl-10 pr-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
                  placeholder="Search by email..."
                />
                {searchingIdentity && (
                  <div className="absolute right-3 top-1/2 -translate-y-1/2">
                    <div className="w-4 h-4 border-2 border-primary-500 border-t-transparent rounded-full animate-spin" />
                  </div>
                )}
              </div>
              {selectedIdentity && (
                <div className="mt-2 p-3 bg-primary-500/10 border border-primary-500/20 rounded-lg">
                  <div className="flex items-center gap-2 text-sm">
                    <LinkIcon className="w-4 h-4 text-primary-400" />
                    <span className="text-primary-300">Linked to:</span>
                    <span className="text-white font-medium">{selectedIdentity.email}</span>
                    {selectedIdentity.display_name && (
                      <span className="text-zinc-400">({selectedIdentity.display_name})</span>
                    )}
                  </div>
                </div>
              )}
              {showIdentityDropdown && identityResults.length > 0 && (
                <div className="absolute z-10 w-full mt-1 bg-zinc-800 border border-white/10 rounded-lg shadow-xl max-h-48 overflow-y-auto">
                  {identityResults.map((identity) => (
                    <button
                      key={identity.id}
                      type="button"
                      onClick={() => selectIdentity(identity)}
                      className="w-full text-left px-4 py-3 hover:bg-white/10 transition-colors border-b border-white/5 last:border-0"
                    >
                      <div className="text-sm text-white font-medium">{identity.email}</div>
                      <div className="text-xs text-zinc-400 flex items-center gap-2">
                        {identity.display_name && <span>{identity.display_name}</span>}
                        {identity.is_active ? (
                          <span className="text-green-400">Active</span>
                        ) : (
                          <span className="text-red-400">Inactive</span>
                        )}
                      </div>
                    </button>
                  ))}
                </div>
              )}
              {showIdentityDropdown && identityResults.length === 0 && identitySearch.trim() && !searchingIdentity && (
                <div className="absolute z-10 w-full mt-1 bg-zinc-800 border border-white/10 rounded-lg shadow-xl p-4 text-center text-sm text-zinc-400">
                  No identity users found for "{identitySearch}"
                </div>
              )}
            </div>
          )}

          {/* Manual user_id (when in manual mode) */}
          {linkMode === 'manual' && (
            <div>
              <label htmlFor="user_id" className="block text-sm font-medium text-zinc-300 mb-2">
                User ID
              </label>
              <input
                type="text"
                id="user_id"
                value={formData.user_id}
                onChange={(e) => setFormData({ ...formData, user_id: e.target.value })}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none"
                placeholder="Optional identifier"
              />
              <p className="text-xs text-zinc-500 mt-1">Optional identifier for this repository-only user</p>
            </div>
          )}

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
              disabled={linkMode === 'identity' && !!selectedIdentity}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none disabled:opacity-50 disabled:cursor-not-allowed"
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
              disabled={saving || (isNew && linkMode === 'identity' && !selectedIdentity)}
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
