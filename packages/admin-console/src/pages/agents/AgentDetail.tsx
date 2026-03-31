import { useEffect, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { ArrowLeft, Bot, Pencil, MessageSquare, AlertCircle, RefreshCw } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import { agentsApi, type Agent } from '../../api/agents'
import { agentConversationsApi, type AgentConversation } from '../../api/agent-conversations'

function formatTimeAgo(dateStr: string | undefined): string {
  if (!dateStr) return '-'
  const date = new Date(dateStr)
  const now = new Date()
  const diffMs = now.getTime() - date.getTime()
  const diffMins = Math.floor(diffMs / 60000)
  if (diffMins < 1) return 'just now'
  if (diffMins < 60) return `${diffMins}m ago`
  const diffHours = Math.floor(diffMins / 60)
  if (diffHours < 24) return `${diffHours}h ago`
  const diffDays = Math.floor(diffHours / 24)
  if (diffDays < 30) return `${diffDays}d ago`
  return date.toLocaleDateString()
}

function shortenPath(path: string): string {
  const parts = path.split('/')
  if (parts.length <= 3) return path
  return '.../' + parts.slice(-2).join('/')
}

export default function AgentDetail() {
  const navigate = useNavigate()
  const { repo, branch, agentId } = useParams<{ repo: string; branch?: string; agentId: string }>()
  const activeBranch = branch || 'main'

  const [agent, setAgent] = useState<Agent | null>(null)
  const [conversations, setConversations] = useState<AgentConversation[]>([])
  const [loadingAgent, setLoadingAgent] = useState(true)
  const [loadingConversations, setLoadingConversations] = useState(true)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (repo && agentId) {
      loadAgent()
      loadConversations()
    }
  }, [repo, activeBranch, agentId])

  async function loadAgent() {
    if (!repo || !agentId) return
    setLoadingAgent(true)
    setError(null)
    try {
      const data = await agentsApi.get(repo, agentId, activeBranch)
      setAgent(data)
    } catch (err) {
      console.error('Failed to load agent:', err)
      setError('Failed to load agent')
    } finally {
      setLoadingAgent(false)
    }
  }

  async function loadConversations() {
    if (!repo || !agentId) return
    setLoadingConversations(true)
    try {
      const data = await agentConversationsApi.listConversations(repo, agentId)
      setConversations(data)
    } catch (err) {
      console.error('Failed to load conversations:', err)
    } finally {
      setLoadingConversations(false)
    }
  }

  function handleRetry() {
    loadAgent()
    loadConversations()
  }

  if (loadingAgent) {
    return (
      <div className="animate-fade-in">
        <div className="animate-pulse">
          <div className="h-8 bg-white/10 rounded w-64 mb-4" />
          <div className="h-4 bg-white/10 rounded w-48 mb-8" />
          <div className="h-48 bg-white/5 rounded-xl" />
        </div>
      </div>
    )
  }

  if (error || !agent) {
    return (
      <div className="animate-fade-in">
        <div className="mb-6">
          <Link
            to={`/${repo}/${activeBranch}/agents`}
            className="inline-flex items-center gap-2 text-zinc-400 hover:text-white transition-colors"
          >
            <ArrowLeft className="w-4 h-4" />
            Back to Agents
          </Link>
        </div>
        <GlassCard>
          <div className="text-center py-12">
            <AlertCircle className="w-16 h-16 text-red-400 mx-auto mb-4" />
            <h3 className="text-xl font-semibold text-white mb-2">Failed to load agent</h3>
            <p className="text-zinc-400 mb-4">{error || 'Agent not found'}</p>
            <button
              onClick={handleRetry}
              className="inline-flex items-center gap-2 px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors"
            >
              <RefreshCw className="w-4 h-4" />
              Retry
            </button>
          </div>
        </GlassCard>
      </div>
    )
  }

  const toolsCount = agent.properties.tools?.length || 0

  return (
    <div className="animate-fade-in">
      {/* Back link */}
      <div className="mb-6">
        <Link
          to={`/${repo}/${activeBranch}/agents`}
          className="inline-flex items-center gap-2 text-zinc-400 hover:text-white transition-colors"
        >
          <ArrowLeft className="w-4 h-4" />
          Back to Agents
        </Link>
      </div>

      {/* Header */}
      <div className="mb-8 flex flex-col md:flex-row justify-between items-start gap-4">
        <div>
          <div className="flex items-center gap-3 mb-2">
            <Bot className="w-10 h-10 text-primary-400" />
            <h1 className="text-4xl font-bold text-white">{agent.name}</h1>
          </div>
          <div className="flex flex-wrap items-center gap-2 mt-3">
            <span className="px-2.5 py-1 bg-zinc-700/60 text-zinc-300 text-xs rounded-full capitalize">
              {agent.properties.provider}
            </span>
            <span className="px-2.5 py-1 bg-zinc-700/60 text-zinc-300 text-xs rounded-full" title={agent.properties.model}>
              {agent.properties.model}
            </span>
            <span className="px-2.5 py-1 bg-primary-500/20 text-primary-300 text-xs rounded-full">
              {toolsCount} {toolsCount === 1 ? 'tool' : 'tools'}
            </span>
            {agent.properties.task_creation_enabled && (
              <span className="px-2.5 py-1 bg-green-500/20 text-green-300 text-xs rounded-full">
                Task Creation
              </span>
            )}
            {agent.properties.thinking_enabled && (
              <span className="px-2.5 py-1 bg-purple-500/20 text-purple-300 text-xs rounded-full">
                Thinking
              </span>
            )}
          </div>
        </div>
        <Link
          to={`/${repo}/${activeBranch}/agents/${agentId}/edit`}
          className="flex items-center gap-2 px-4 py-2 bg-white/10 hover:bg-white/20 border border-white/10 text-white rounded-lg transition-colors"
        >
          <Pencil className="w-4 h-4" />
          Edit Agent
        </Link>
      </div>

      {/* Conversations Section */}
      <div>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-semibold text-white flex items-center gap-2">
            <MessageSquare className="w-5 h-5 text-zinc-400" />
            Conversations
          </h2>
          <button
            onClick={loadConversations}
            className="p-2 text-zinc-400 hover:text-white hover:bg-white/10 rounded-lg transition-colors"
            title="Refresh conversations"
          >
            <RefreshCw className={`w-4 h-4 ${loadingConversations ? 'animate-spin' : ''}`} />
          </button>
        </div>

        {loadingConversations ? (
          <div className="space-y-3">
            {[1, 2, 3].map(i => (
              <div key={i} className="h-16 bg-white/5 rounded-xl animate-pulse" />
            ))}
          </div>
        ) : conversations.length === 0 ? (
          <GlassCard>
            <div className="text-center py-12">
              <MessageSquare className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
              <h3 className="text-xl font-semibold text-white mb-2">No conversations yet</h3>
              <p className="text-zinc-400">
                This agent hasn't had any conversations. Start a chat to see traces here.
              </p>
            </div>
          </GlassCard>
        ) : (
          <div className="space-y-2">
            {conversations.map(conv => (
              <div
                key={conv.id}
                onClick={() =>
                  navigate(
                    `/${repo}/${activeBranch}/agents/${agentId}/conversations/${encodeURIComponent(conv.path)}`
                  )
                }
                className="flex items-center gap-4 p-4 bg-white/5 border border-white/10 rounded-xl cursor-pointer hover:bg-white/[0.08] transition-colors"
              >
                <MessageSquare className="w-5 h-5 text-zinc-500 shrink-0" />
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-white font-medium truncate">
                      {conv.subject || shortenPath(conv.path)}
                    </span>
                    {conv.hasErrors && (
                      <span className="px-1.5 py-0.5 bg-red-500/20 text-red-300 text-xs rounded">
                        errors
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-zinc-500 mt-0.5 truncate" title={conv.path}>
                    {conv.path}
                  </div>
                </div>
                <div className="text-right shrink-0">
                  <div className="text-xs text-zinc-400">
                    {conv.status && (
                      <span className={`px-2 py-0.5 rounded text-xs ${
                        conv.status === 'completed'
                          ? 'bg-green-500/20 text-green-300'
                          : conv.status === 'active'
                          ? 'bg-blue-500/20 text-blue-300'
                          : 'bg-zinc-500/20 text-zinc-300'
                      }`}>
                        {conv.status}
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-zinc-500 mt-1">
                    {formatTimeAgo(conv.updatedAt || conv.createdAt)}
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
