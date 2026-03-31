import { useEffect, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { Bot, Plus, Trash2, Pencil, Search } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import ConfirmDialog from '../../components/ConfirmDialog'
import { useToast, ToastContainer } from '../../components/Toast'
import { agentsApi, type Agent } from '../../api/agents'

export default function AgentsList() {
  const navigate = useNavigate()
  const { repo, branch } = useParams<{ repo: string; branch?: string }>()
  const activeBranch = branch || 'main'

  const [agents, setAgents] = useState<Agent[]>([])
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [deleteConfirm, setDeleteConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  useEffect(() => {
    loadAgents()
  }, [repo, activeBranch])

  async function loadAgents() {
    if (!repo) return
    setLoading(true)
    try {
      const data = await agentsApi.list(repo, activeBranch)
      setAgents(data)
    } catch (error) {
      console.error('Failed to load agents:', error)
      showError('Load Failed', 'Failed to load agents')
    } finally {
      setLoading(false)
    }
  }

  async function handleDelete(agentName: string) {
    if (!repo) return
    setDeleteConfirm({
      message: `Are you sure you want to delete agent "${agentName}"?`,
      onConfirm: async () => {
        try {
          await agentsApi.delete(repo, agentName, activeBranch)
          loadAgents()
          showSuccess('Deleted', 'Agent deleted successfully')
        } catch (error) {
          console.error('Failed to delete agent:', error)
          showError('Delete Failed', 'Failed to delete agent')
        }
      }
    })
  }

  // Filter agents based on search query
  const filteredAgents = agents.filter(agent =>
    agent.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
    agent.properties.system_prompt.toLowerCase().includes(searchQuery.toLowerCase())
  )

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex flex-col md:flex-row justify-between items-start gap-4">
        <div>
          <div className="flex items-center gap-3 mb-2">
            <Bot className="w-10 h-10 text-primary-400" />
            <h1 className="text-4xl font-bold text-white">AI Agents</h1>
          </div>
          <p className="text-zinc-400">Create and manage AI agents with custom prompts and tools</p>
        </div>
        <Link
          to={`/${repo}/${activeBranch}/agents/new`}
          className="flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
        >
          <Plus className="w-5 h-5" />
          Create Agent
        </Link>
      </div>

      {/* Search Bar */}
      <div className="mb-6">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-zinc-400" />
          <input
            type="text"
            placeholder="Search agents..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
          />
        </div>
      </div>

      {loading ? (
        <div className="text-center text-zinc-400 py-12">Loading agents...</div>
      ) : filteredAgents.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <Bot className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">
              {searchQuery ? 'No agents found' : 'No agents yet'}
            </h3>
            <p className="text-zinc-400 mb-4">
              {searchQuery
                ? 'Try adjusting your search query'
                : 'Create your first AI agent to get started'}
            </p>
            {!searchQuery && (
              <Link
                to={`/${repo}/${activeBranch}/agents/new`}
                className="inline-flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
              >
                <Plus className="w-5 h-5" />
                Create Agent
              </Link>
            )}
          </div>
        </GlassCard>
      ) : (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
          {filteredAgents.map((agent) => {
            const toolsCount = agent.properties.tools?.length || 0
            const rulesCount = agent.properties.rules?.length || 0

            return (
              <GlassCard key={agent.id} hover className="cursor-pointer">
                <div className="flex items-start justify-between">
                  <div
                    className="flex-1 cursor-pointer"
                    onClick={() => navigate(`/${repo}/${activeBranch}/agents/${agent.name}`)}
                  >
                    <div className="flex items-center gap-3 mb-2">
                      <Bot className="w-6 h-6 text-primary-400" />
                      <h3 className="text-xl font-semibold text-white">{agent.name}</h3>
                    </div>

                    <p className="text-sm text-zinc-300 mb-3 line-clamp-2">
                      {agent.properties.system_prompt}
                    </p>

                    <div className="grid grid-cols-2 gap-3 mb-3">
                      <div className="flex flex-col">
                        <span className="text-xs text-zinc-500">Provider</span>
                        <span className="text-sm text-zinc-300 capitalize">
                          {agent.properties.provider}
                        </span>
                      </div>
                      <div className="flex flex-col">
                        <span className="text-xs text-zinc-500">Model</span>
                        <span className="text-sm text-zinc-300 truncate" title={agent.properties.model}>
                          {agent.properties.model}
                        </span>
                      </div>
                      <div className="flex flex-col">
                        <span className="text-xs text-zinc-500">Temperature</span>
                        <span className="text-sm text-zinc-300">{agent.properties.temperature}</span>
                      </div>
                      <div className="flex flex-col">
                        <span className="text-xs text-zinc-500">Max Tokens</span>
                        <span className="text-sm text-zinc-300">{agent.properties.max_tokens}</span>
                      </div>
                    </div>

                    <div className="flex flex-wrap gap-2 mb-3">
                      <span className="px-2 py-1 bg-primary-500/20 text-primary-300 text-xs rounded-full">
                        {toolsCount} {toolsCount === 1 ? 'tool' : 'tools'}
                      </span>
                      <span className="px-2 py-1 bg-indigo-500/20 text-indigo-300 text-xs rounded-full">
                        {rulesCount} {rulesCount === 1 ? 'rule' : 'rules'}
                      </span>
                      {agent.properties.thinking_enabled && (
                        <span className="px-2 py-1 bg-purple-500/20 text-purple-300 text-xs rounded-full">
                          Thinking
                        </span>
                      )}
                      {agent.properties.task_creation_enabled && (
                        <span className="px-2 py-1 bg-green-500/20 text-green-300 text-xs rounded-full">
                          Task Creation
                        </span>
                      )}
                    </div>

                    <div className="text-xs text-zinc-500">
                      {agent.updated_at && (
                        <p>Updated: {new Date(agent.updated_at).toLocaleString()}</p>
                      )}
                    </div>
                  </div>

                  <div className="flex items-center gap-2 ml-4">
                    <Link
                      to={`/${repo}/${activeBranch}/agents/${agent.name}/edit`}
                      className="p-2 bg-white/10 hover:bg-white/20 text-primary-300 rounded-lg transition-colors"
                      title="Edit agent"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <Pencil className="w-5 h-5" />
                    </Link>
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        handleDelete(agent.name)
                      }}
                      className="p-2 bg-red-500/20 hover:bg-red-500/30 text-red-400 rounded-lg transition-colors"
                      title="Delete agent"
                    >
                      <Trash2 className="w-5 h-5" />
                    </button>
                  </div>
                </div>
              </GlassCard>
            )
          })}
        </div>
      )}

      <ConfirmDialog
        open={deleteConfirm !== null}
        title="Confirm Delete"
        message={deleteConfirm?.message || ''}
        variant="danger"
        confirmText="Delete"
        onConfirm={() => {
          deleteConfirm?.onConfirm()
          setDeleteConfirm(null)
        }}
        onCancel={() => setDeleteConfirm(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
