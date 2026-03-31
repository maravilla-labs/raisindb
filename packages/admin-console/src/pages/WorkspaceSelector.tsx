import { useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { FolderTree, ArrowRight } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { workspacesApi, type Workspace } from '../api/workspaces'

export default function WorkspaceSelector() {
  const { repo } = useParams<{ repo: string }>()
  const navigate = useNavigate()
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [loading, setLoading] = useState(true)

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
      
      // Auto-navigate to first workspace if only one exists
      if (data.length === 1) {
        handleSelectWorkspace(data[0].name)
      }
    } catch (error) {
      console.error('Failed to load workspaces:', error)
    } finally {
      setLoading(false)
    }
  }

  function handleSelectWorkspace(workspaceName: string) {
    navigate(`/${repo}/content/main/${workspaceName}`)
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-white text-xl">Loading workspaces...</div>
      </div>
    )
  }

  return (
    <div className="animate-fade-in">
      <div className="max-w-4xl mx-auto">
        <div className="mb-8 text-center">
          <h1 className="text-4xl font-bold text-white mb-2">Select a Workspace</h1>
          <p className="text-zinc-400">Choose a workspace to browse its content</p>
        </div>

        {workspaces.length === 0 ? (
          <GlassCard className="text-center py-16">
            <FolderTree className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">No workspaces found</h3>
            <p className="text-zinc-400 mb-6">Create a workspace first to browse content</p>
            <button
              onClick={() => navigate(`/${repo}/workspaces`)}
              className="px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg font-semibold transition-colors"
            >
              Go to Workspaces
            </button>
          </GlassCard>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {workspaces.map((workspace) => (
              <GlassCard
                key={workspace.name}
                className="cursor-pointer hover:bg-white/10 transition-all group"
                onClick={() => handleSelectWorkspace(workspace.name)}
              >
                <div className="flex items-start justify-between mb-3">
                  <div className="flex items-center gap-3">
                    <FolderTree className="w-6 h-6 text-primary-400" />
                    <h3 className="text-lg font-semibold text-white">{workspace.name}</h3>
                  </div>
                  <ArrowRight className="w-5 h-5 text-primary-400 opacity-0 group-hover:opacity-100 transition-opacity" />
                </div>
                {workspace.description && (
                  <p className="text-zinc-400 text-sm mb-3">{workspace.description}</p>
                )}
                {workspace.allowed_node_types && workspace.allowed_node_types.length > 0 && (
                  <div className="flex gap-2 flex-wrap">
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
              </GlassCard>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
