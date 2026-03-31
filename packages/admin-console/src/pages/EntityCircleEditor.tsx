import { useEffect, useState } from 'react'
import { useNavigate, useParams, useSearchParams, Link } from 'react-router-dom'
import { ArrowLeft, Save, Users, Plus, Trash2, ChevronDown, ChevronRight } from 'lucide-react'
import { createPortal } from 'react-dom'
import GlassCard from '../components/GlassCard'
import { useToast, ToastContainer } from '../components/Toast'
import { nodesApi, type NodeRelationships, type IncomingRelation } from '../api/nodes'

const WORKSPACE = 'raisin:access_control'

interface EntityCircle {
  name: string
  circle_type: string
  primary_contact_id?: string
  address?: {
    street?: string
    city?: string
    state?: string
    postal_code?: string
    country?: string
  }
  metadata?: Record<string, unknown>
}

interface Member {
  id: string
  path: string
  display_name: string
  email?: string
}

const CIRCLE_TYPES = [
  { value: 'family', label: 'Family' },
  { value: 'team', label: 'Team' },
  { value: 'org_unit', label: 'Organizational Unit' },
  { value: 'department', label: 'Department' },
  { value: 'project', label: 'Project' },
  { value: 'custom', label: 'Custom' },
]

// User Selector Dialog Component
interface UserSelectorDialogProps {
  isOpen: boolean
  onClose: () => void
  onSelect: (userId: string, userName: string) => void
  repo: string
  branch: string
  excludeIds?: string[]
}

function UserSelectorDialog({ isOpen, onClose, onSelect, repo, branch, excludeIds = [] }: UserSelectorDialogProps) {
  const [users, setUsers] = useState<Member[]>([])
  const [loading, setLoading] = useState(false)
  const [searchTerm, setSearchTerm] = useState('')

  useEffect(() => {
    if (isOpen) {
      loadUsers()
    }
  }, [isOpen])

  async function loadUsers() {
    setLoading(true)
    try {
      const nodes = await nodesApi.listChildrenAtHead(repo, branch, WORKSPACE, '/users')
      const userList = nodes
        .filter(n => n.node_type === 'raisin:User')
        .map(n => ({
          id: n.properties?.user_id as string,
          path: n.path,
          display_name: n.properties?.display_name as string || n.name,
          email: n.properties?.email as string | undefined,
        }))
        .filter(u => !excludeIds.includes(u.id))
      setUsers(userList)
    } catch (error) {
      console.error('Failed to load users:', error)
    } finally {
      setLoading(false)
    }
  }

  const filteredUsers = users.filter(u =>
    u.display_name.toLowerCase().includes(searchTerm.toLowerCase()) ||
    u.email?.toLowerCase().includes(searchTerm.toLowerCase()) ||
    u.id.toLowerCase().includes(searchTerm.toLowerCase())
  )

  if (!isOpen) return null

  return createPortal(
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-labelledby="user-selector-title"
    >
      <div
        className="bg-white/90 backdrop-blur-xl border border-white/20 shadow-2xl rounded-2xl max-w-2xl w-full max-h-[80vh] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="p-6 border-b border-white/10">
          <h2 id="user-selector-title" className="text-2xl font-bold text-gray-900">Select User</h2>
          <input
            type="text"
            placeholder="Search users..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="mt-4 w-full px-4 py-2 bg-white/50 border border-gray-300 rounded-lg text-gray-900 placeholder-gray-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
            autoFocus
          />
        </div>
        <div className="overflow-y-auto max-h-96 p-4">
          {loading ? (
            <div className="text-center py-8 text-gray-600">Loading users...</div>
          ) : filteredUsers.length === 0 ? (
            <div className="text-center py-8 text-gray-600">No users found</div>
          ) : (
            <div className="space-y-2">
              {filteredUsers.map((user) => (
                <button
                  key={user.id}
                  onClick={() => {
                    onSelect(user.id, user.display_name)
                    onClose()
                  }}
                  className="w-full text-left px-4 py-3 bg-white/50 hover:bg-white/80 border border-gray-200 rounded-lg transition-colors"
                >
                  <div className="font-medium text-gray-900">{user.display_name}</div>
                  {user.email && (
                    <div className="text-sm text-gray-600">{user.email}</div>
                  )}
                  <div className="text-xs text-gray-500">ID: {user.id}</div>
                </button>
              ))}
            </div>
          )}
        </div>
        <div className="p-4 border-t border-white/10 flex justify-end">
          <button
            onClick={onClose}
            className="px-6 py-2 bg-gray-200 hover:bg-gray-300 text-gray-900 rounded-lg transition-colors"
          >
            Cancel
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}

export default function EntityCircleEditor() {
  const { repo, branch, '*': pathParam } = useParams<{ repo: string; branch?: string; '*': string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()

  // Parse path from wildcard
  const pathParts = pathParam?.split('/').filter(Boolean) || []
  const isNew = !pathParam || pathParam === 'new'

  let circlePath: string | null = null
  let parentPath = '/circles'

  if (isNew) {
    // Creating new circle - parentPath from query param
    const parentPathParam = searchParams.get('parentPath')
    if (parentPathParam) {
      const trimmed = parentPathParam.trim()
      if (trimmed) {
        parentPath = trimmed.startsWith('/circles') ? trimmed : `/circles${trimmed.startsWith('/') ? trimmed : `/${trimmed}`}`
      }
    }
  } else {
    // Editing existing circle - extract from URL path
    pathParts.pop() // Last segment is the circle name, remove it to get parent
    parentPath = pathParts.length > 0 ? `/circles/${pathParts.join('/')}` : '/circles'
    circlePath = `/circles/${pathParam}` // Full path for loading/updating
  }

  const listRoute = repo ? `/${repo}/${activeBranch}${parentPath}` : '/'

  const [loading, setLoading] = useState(!isNew)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [showAddressSection, setShowAddressSection] = useState(false)
  const [members, setMembers] = useState<Member[]>([])
  const [showUserSelector, setShowUserSelector] = useState(false)
  const [selectorMode, setSelectorMode] = useState<'contact' | 'member'>('member')

  const [formData, setFormData] = useState<EntityCircle>({
    name: '',
    circle_type: 'team',
    primary_contact_id: undefined,
    address: undefined,
    metadata: undefined,
  })

  const [primaryContactName, setPrimaryContactName] = useState<string>('')

  const { toasts, success: showSuccess, error: showError, closeToast } = useToast()

  useEffect(() => {
    if (!repo) return
    if (!isNew && circlePath) {
      loadCircle()
    }
  }, [repo, activeBranch, circlePath, isNew])

  async function loadCircle() {
    if (!repo || !circlePath) return
    setLoading(true)
    setError(null)

    try {
      // Load circle node
      const node = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, circlePath)
      const circleData: EntityCircle = {
        name: node.properties?.name as string || '',
        circle_type: node.properties?.circle_type as string || 'custom',
        primary_contact_id: node.properties?.primary_contact_id as string | undefined,
        address: node.properties?.address as EntityCircle['address'] | undefined,
        metadata: node.properties?.metadata as Record<string, unknown> | undefined,
      }
      setFormData(circleData)
      setShowAddressSection(!!circleData.address)

      // Load primary contact name
      if (circleData.primary_contact_id) {
        try {
          const contactNode = await nodesApi.getAtHead(repo, activeBranch, WORKSPACE, `/users/${circleData.primary_contact_id}`)
          setPrimaryContactName(contactNode.properties?.display_name as string || circleData.primary_contact_id)
        } catch {
          setPrimaryContactName(circleData.primary_contact_id)
        }
      }

      // Load members (incoming MEMBER_OF relationships)
      try {
        const relationships: NodeRelationships = await nodesApi.getRelationships(repo, activeBranch, WORKSPACE, circlePath)
        const memberRelations = relationships.incoming.filter(rel => rel.relation_type === 'MEMBER_OF')

        // Load full user data for each member
        const memberData = await Promise.all(
          memberRelations.map(async (rel: IncomingRelation) => {
            try {
              const userNode = await nodesApi.getByIdAtHead(repo, activeBranch, WORKSPACE, rel.source_node_id)
              return {
                id: userNode.properties?.user_id as string,
                path: userNode.path,
                display_name: userNode.properties?.display_name as string || userNode.name,
                email: userNode.properties?.email as string | undefined,
              }
            } catch {
              return null
            }
          })
        )

        setMembers(memberData.filter(m => m !== null) as Member[])
      } catch (error) {
        console.error('Failed to load members:', error)
      }
    } catch (err) {
      console.error('Failed to load circle:', err)
      setError('Failed to load entity circle')
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
      const properties: any = {
        name: formData.name.trim(),
        circle_type: formData.circle_type,
      }

      if (formData.primary_contact_id) {
        properties.primary_contact_id = formData.primary_contact_id
      }

      if (showAddressSection && formData.address) {
        properties.address = formData.address
      }

      if (formData.metadata) {
        properties.metadata = formData.metadata
      }

      const commit = {
        message: isNew ? `Create entity circle ${formData.name}` : `Update entity circle ${formData.name}`,
        actor: 'admin',
      }

      if (isNew) {
        // Create new circle
        const nodeName = formData.name.toLowerCase().replace(/\s+/g, '_')
        await nodesApi.create(repo, activeBranch, WORKSPACE, parentPath, {
          name: nodeName,
          node_type: 'raisin:EntityCircle',
          properties,
          commit,
        })
        showSuccess('Circle Created', `Entity circle "${properties.name}" was created successfully`)
      } else if (circlePath) {
        // Update existing circle
        await nodesApi.update(repo, activeBranch, WORKSPACE, circlePath, {
          properties,
          commit,
        })
        showSuccess('Circle Updated', `Entity circle "${properties.name}" was updated successfully`)
      }

      // Small delay to allow toast to be visible before navigation
      setTimeout(() => navigate(listRoute), 500)
    } catch (err: any) {
      console.error('Failed to save circle:', err)
      const errorMessage = err.message || 'Failed to save entity circle'
      setError(errorMessage)
      showError('Save Failed', errorMessage)
    } finally {
      setSaving(false)
    }
  }

  async function handleAddMember(userId: string, userName: string) {
    if (!repo || !circlePath) return

    try {
      // Add MEMBER_OF relationship from user to circle
      await nodesApi.addRelation(repo, activeBranch, WORKSPACE, `/users/${userId}`, {
        targetWorkspace: WORKSPACE,
        targetPath: circlePath,
        relationType: 'MEMBER_OF',
      })

      // Add to local state
      setMembers([...members, { id: userId, path: `/users/${userId}`, display_name: userName }])
      showSuccess('Member Added', `${userName} was added to the circle`)
    } catch (error: any) {
      console.error('Failed to add member:', error)
      showError('Failed to Add Member', error.message || 'Could not add member to circle')
    }
  }

  async function handleRemoveMember(member: Member) {
    if (!repo || !circlePath) return

    try {
      // Remove MEMBER_OF relationship
      await nodesApi.removeRelation(repo, activeBranch, WORKSPACE, member.path, {
        targetWorkspace: WORKSPACE,
        targetPath: circlePath,
      })

      // Remove from local state
      setMembers(members.filter(m => m.id !== member.id))
      showSuccess('Member Removed', `${member.display_name} was removed from the circle`)
    } catch (error: any) {
      console.error('Failed to remove member:', error)
      showError('Failed to Remove Member', error.message || 'Could not remove member from circle')
    }
  }

  function handleSelectPrimaryContact(userId: string, userName: string) {
    setFormData({ ...formData, primary_contact_id: userId })
    setPrimaryContactName(userName)
  }

  if (loading) {
    return (
      <div className="animate-fade-in">
        <div className="text-center text-zinc-400 py-12">Loading entity circle...</div>
      </div>
    )
  }

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex items-center gap-4">
        <Link
          to={listRoute}
          className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          aria-label="Back to circles list"
        >
          <ArrowLeft className="w-6 h-6 text-zinc-400" />
        </Link>
        <div>
          <h1 className="text-4xl font-bold text-white flex items-center gap-3">
            <Users className="w-10 h-10 text-primary-400" />
            {isNew ? 'New Entity Circle' : `Edit Circle: ${formData.name}`}
          </h1>
          <p className="text-zinc-400 mt-2">
            {isNew ? 'Create a new entity circle' : 'Update circle information and manage members'}
          </p>
        </div>
      </div>

      {error && (
        <div className="mb-6 p-4 bg-red-500/20 border border-red-500/30 rounded-lg text-red-300" role="alert">
          {error}
        </div>
      )}

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Main Form */}
        <div className="lg:col-span-2">
          <GlassCard>
            <form onSubmit={handleSubmit} className="space-y-6">
              <div>
                <label htmlFor="name" className="block text-sm font-medium text-zinc-300 mb-2">
                  Circle Name <span className="text-red-400">*</span>
                </label>
                <input
                  type="text"
                  id="name"
                  required
                  value={formData.name}
                  onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                  placeholder="Engineering Team"
                />
              </div>

              <div>
                <label htmlFor="circle_type" className="block text-sm font-medium text-zinc-300 mb-2">
                  Circle Type <span className="text-red-400">*</span>
                </label>
                <select
                  id="circle_type"
                  required
                  value={formData.circle_type}
                  onChange={(e) => setFormData({ ...formData, circle_type: e.target.value })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                >
                  {CIRCLE_TYPES.map((type) => (
                    <option key={type.value} value={type.value}>
                      {type.label}
                    </option>
                  ))}
                </select>
              </div>

              <div>
                <label className="block text-sm font-medium text-zinc-300 mb-2">
                  Primary Contact
                </label>
                <div className="flex gap-2">
                  <input
                    type="text"
                    readOnly
                    value={primaryContactName || formData.primary_contact_id || 'No contact selected'}
                    className="flex-1 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 cursor-not-allowed"
                  />
                  <button
                    type="button"
                    onClick={() => {
                      setSelectorMode('contact')
                      setShowUserSelector(true)
                    }}
                    className="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
                  >
                    Select
                  </button>
                  {formData.primary_contact_id && (
                    <button
                      type="button"
                      onClick={() => {
                        setFormData({ ...formData, primary_contact_id: undefined })
                        setPrimaryContactName('')
                      }}
                      className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 text-red-300 rounded-lg transition-colors"
                    >
                      Clear
                    </button>
                  )}
                </div>
              </div>

              {/* Address Section - Collapsible */}
              <div className="border border-white/10 rounded-lg">
                <button
                  type="button"
                  onClick={() => setShowAddressSection(!showAddressSection)}
                  className="w-full flex items-center justify-between px-4 py-3 hover:bg-white/5 transition-colors"
                >
                  <span className="text-sm font-medium text-zinc-300">Address (Optional)</span>
                  {showAddressSection ? (
                    <ChevronDown className="w-5 h-5 text-zinc-400" />
                  ) : (
                    <ChevronRight className="w-5 h-5 text-zinc-400" />
                  )}
                </button>
                {showAddressSection && (
                  <div className="px-4 pb-4 space-y-4 border-t border-white/10">
                    <div className="mt-4">
                      <label htmlFor="street" className="block text-sm font-medium text-zinc-300 mb-2">
                        Street
                      </label>
                      <input
                        type="text"
                        id="street"
                        value={formData.address?.street || ''}
                        onChange={(e) => setFormData({
                          ...formData,
                          address: { ...formData.address, street: e.target.value }
                        })}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                        placeholder="123 Main St"
                      />
                    </div>
                    <div className="grid grid-cols-2 gap-4">
                      <div>
                        <label htmlFor="city" className="block text-sm font-medium text-zinc-300 mb-2">
                          City
                        </label>
                        <input
                          type="text"
                          id="city"
                          value={formData.address?.city || ''}
                          onChange={(e) => setFormData({
                            ...formData,
                            address: { ...formData.address, city: e.target.value }
                          })}
                          className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                          placeholder="San Francisco"
                        />
                      </div>
                      <div>
                        <label htmlFor="state" className="block text-sm font-medium text-zinc-300 mb-2">
                          State/Province
                        </label>
                        <input
                          type="text"
                          id="state"
                          value={formData.address?.state || ''}
                          onChange={(e) => setFormData({
                            ...formData,
                            address: { ...formData.address, state: e.target.value }
                          })}
                          className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                          placeholder="CA"
                        />
                      </div>
                    </div>
                    <div className="grid grid-cols-2 gap-4">
                      <div>
                        <label htmlFor="postal_code" className="block text-sm font-medium text-zinc-300 mb-2">
                          Postal Code
                        </label>
                        <input
                          type="text"
                          id="postal_code"
                          value={formData.address?.postal_code || ''}
                          onChange={(e) => setFormData({
                            ...formData,
                            address: { ...formData.address, postal_code: e.target.value }
                          })}
                          className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                          placeholder="94102"
                        />
                      </div>
                      <div>
                        <label htmlFor="country" className="block text-sm font-medium text-zinc-300 mb-2">
                          Country
                        </label>
                        <input
                          type="text"
                          id="country"
                          value={formData.address?.country || ''}
                          onChange={(e) => setFormData({
                            ...formData,
                            address: { ...formData.address, country: e.target.value }
                          })}
                          className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                          placeholder="USA"
                        />
                      </div>
                    </div>
                  </div>
                )}
              </div>

              <div className="flex items-center gap-4 pt-4">
                <button
                  type="submit"
                  disabled={saving}
                  className="flex items-center gap-2 px-6 py-2 bg-primary-500 hover:bg-primary-600 disabled:bg-primary-500/50 text-white rounded-lg transition-colors focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2"
                >
                  <Save className="w-5 h-5" />
                  {saving ? 'Saving...' : isNew ? 'Create Circle' : 'Update Circle'}
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
        </div>

        {/* Members Panel - Only show when editing existing circle */}
        {!isNew && (
          <div className="lg:col-span-1">
            <GlassCard>
              <div className="flex items-center justify-between mb-4">
                <h3 className="text-lg font-semibold text-white">Members ({members.length})</h3>
                <button
                  onClick={() => {
                    setSelectorMode('member')
                    setShowUserSelector(true)
                  }}
                  className="p-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
                  title="Add member"
                  aria-label="Add member to circle"
                >
                  <Plus className="w-4 h-4" />
                </button>
              </div>

              {members.length === 0 ? (
                <div className="text-center py-8 text-zinc-400">
                  <Users className="w-12 h-12 mx-auto mb-2 text-zinc-500" />
                  <p className="text-sm">No members yet</p>
                  <p className="text-xs mt-1">Click + to add members</p>
                </div>
              ) : (
                <div className="space-y-2 max-h-96 overflow-y-auto">
                  {members.map((member) => (
                    <div
                      key={member.id}
                      className="flex items-center justify-between p-3 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 transition-colors"
                    >
                      <div className="flex-1 min-w-0">
                        <div className="font-medium text-white truncate">{member.display_name}</div>
                        {member.email && (
                          <div className="text-xs text-zinc-400 truncate">{member.email}</div>
                        )}
                      </div>
                      <button
                        onClick={() => handleRemoveMember(member)}
                        className="ml-2 p-1.5 hover:bg-red-500/20 text-zinc-400 hover:text-red-400 rounded transition-colors"
                        title="Remove member"
                        aria-label={`Remove ${member.display_name} from circle`}
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </GlassCard>
          </div>
        )}
      </div>

      {/* User Selector Dialog */}
      {repo && (
        <UserSelectorDialog
          isOpen={showUserSelector}
          onClose={() => setShowUserSelector(false)}
          onSelect={selectorMode === 'contact' ? handleSelectPrimaryContact : handleAddMember}
          repo={repo}
          branch={activeBranch}
          excludeIds={selectorMode === 'member' ? members.map(m => m.id) : []}
        />
      )}

      {/* Toast Notifications */}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
