import { useEffect, useState } from 'react'
import { Link, useParams } from 'react-router-dom'
import { FolderTree, Plus, Settings } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { workspacesApi, type Workspace } from '../api/workspaces'
import { useToast, ToastContainer } from '../components/Toast'

export default function Workspaces() {
  const { repo } = useParams<{ repo: string }>()
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [loading, setLoading] = useState(true)
  const [showCreate, setShowCreate] = useState(false)
  const [newWorkspace, setNewWorkspace] = useState({
    name: '',
    description: '',
    allowed_node_types: [] as string[],
    allowed_root_node_types: [] as string[],
  })
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    if (repo) {
      loadWorkspaces()
    }
  }, [repo])

  async function loadWorkspaces() {
    if (!repo) return
    try {
      const data = await workspacesApi.list(repo)
      setWorkspaces(data)
    } catch (error) {
      console.error('Failed to load workspaces:', error)
    } finally {
      setLoading(false)
    }
  }

  async function handleCreate(e: React.FormEvent) {
    e.preventDefault()
    if (!repo) return
    try {
      await workspacesApi.create(repo, {
        name: newWorkspace.name,
        description: newWorkspace.description || undefined,
        allowed_node_types: newWorkspace.allowed_node_types,
        allowed_root_node_types: newWorkspace.allowed_root_node_types,
      })
      setNewWorkspace({
        name: '',
        description: '',
        allowed_node_types: [],
        allowed_root_node_types: [],
      })
      setShowCreate(false)
      showSuccess('Success', `Workspace "${newWorkspace.name}" created successfully`)
      loadWorkspaces()
    } catch (error) {
      console.error('Failed to create workspace:', error)
      showError('Error', 'Failed to create workspace')
    }
  }

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex justify-between items-start">
        <div>
          <h1 className="text-4xl font-bold text-white mb-2">Workspaces</h1>
          <p className="text-zinc-400">Manage your content workspaces</p>
        </div>
        <button
          onClick={() => setShowCreate(!showCreate)}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" />
          New Workspace
        </button>
      </div>

      {showCreate && (
        <GlassCard className="mb-6 animate-slide-in">
          <h2 className="text-xl font-semibold text-white mb-4">Create Workspace</h2>
          <form onSubmit={handleCreate} className="space-y-4">
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                Name *
              </label>
              <input
                type="text"
                value={newWorkspace.name}
                onChange={(e) => setNewWorkspace({ ...newWorkspace, name: e.target.value })}
                className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                required
                pattern="[a-zA-Z0-9_-]+"
                title="Only alphanumeric characters, underscores, and hyphens"
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                Description
              </label>
              <textarea
                value={newWorkspace.description}
                onChange={(e) => setNewWorkspace({ ...newWorkspace, description: e.target.value })}
                className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                rows={3}
              />
            </div>
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                Allowed Node Types *
              </label>
              <input
                type="text"
                value={newWorkspace.allowed_node_types.join(', ')}
                onChange={(e) => setNewWorkspace({
                  ...newWorkspace,
                  allowed_node_types: e.target.value.split(',').map(s => s.trim()).filter(Boolean)
                })}
                className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                placeholder="raisin:Folder, raisin:Page, raisin:Asset"
                required
              />
              <p className="text-xs text-zinc-500 mt-1">
                Comma-separated list of node types allowed in this workspace
              </p>
            </div>
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                Allowed Root Node Types *
              </label>
              <input
                type="text"
                value={newWorkspace.allowed_root_node_types.join(', ')}
                onChange={(e) => setNewWorkspace({
                  ...newWorkspace,
                  allowed_root_node_types: e.target.value.split(',').map(s => s.trim()).filter(Boolean)
                })}
                className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
                placeholder="raisin:Folder, raisin:Page"
                required
              />
              <p className="text-xs text-zinc-500 mt-1">
                Comma-separated list of node types that can be at root level
              </p>
            </div>
            <div className="flex gap-2">
              <button
                type="submit"
                className="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
              >
                Create
              </button>
              <button
                type="button"
                onClick={() => setShowCreate(false)}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
              >
                Cancel
              </button>
            </div>
          </form>
        </GlassCard>
      )}

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading...</div>
      ) : workspaces.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <FolderTree className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No workspaces yet</h3>
            <p className="text-zinc-400">Create your first workspace to get started</p>
          </div>
        </GlassCard>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {workspaces.map((workspace) => (
            <Link key={workspace.name} to={`/${repo}/workspaces/${workspace.name}`}>
              <GlassCard hover>
                <div className="flex items-start justify-between mb-3">
                  <div className="flex items-center gap-3">
                    <FolderTree className="w-6 h-6 text-primary-400" />
                    <h3 className="text-lg font-semibold text-white">{workspace.name}</h3>
                  </div>
                  <Settings className="w-5 h-5 text-primary-400 opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
                {workspace.description && (
                  <p className="text-zinc-400 text-sm mb-3">{workspace.description}</p>
                )}
                {workspace.allowed_node_types && workspace.allowed_node_types.length > 0 && (
                  <div className="flex gap-2 flex-wrap mb-3">
                    {workspace.allowed_node_types.slice(0, 3).map((type) => (
                      <span key={type} className="px-2 py-1 bg-primary-500/20 text-primary-300 rounded text-xs">
                        {type}
                      </span>
                    ))}
                    {workspace.allowed_node_types.length > 3 && (
                      <span className="px-2 py-1 text-zinc-400 text-xs">
                        +{workspace.allowed_node_types.length - 3} more
                      </span>
                    )}
                  </div>
                )}
                {workspace.created_at && (
                  <p className="text-xs text-zinc-500">
                    Created: {new Date(workspace.created_at).toLocaleDateString()}
                  </p>
                )}
              </GlassCard>
            </Link>
          ))}
        </div>
      )}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
