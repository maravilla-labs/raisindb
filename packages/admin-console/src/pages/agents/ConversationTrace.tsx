import { useEffect, useState } from 'react'
import { useParams, Link } from 'react-router-dom'
import { ArrowLeft, MessageSquare, Clock, AlertTriangle } from 'lucide-react'

import { agentConversationsApi, type ConversationTree } from '../../api/agent-conversations'
import MessageBubble from './components/MessageBubble'
import RawJsonViewer from './components/RawJsonViewer'

type ViewTab = 'timeline' | 'raw'

export default function ConversationTrace() {
  const { repo, branch, agentId, conversationPath: encodedPath } = useParams<{
    repo: string
    branch: string
    agentId: string
    conversationPath: string
  }>()
  const activeBranch = branch || 'main'

  const conversationPath = encodedPath ? decodeURIComponent(encodedPath) : ''

  const [tree, setTree] = useState<ConversationTree | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [activeTab, setActiveTab] = useState<ViewTab>('timeline')

  useEffect(() => {
    if (!repo || !conversationPath) return
    loadTree()
  }, [repo, conversationPath])

  async function loadTree() {
    setLoading(true)
    setError(null)
    try {
      const data = await agentConversationsApi.getConversationTree(repo!, conversationPath)
      setTree(data)
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load conversation')
    } finally {
      setLoading(false)
    }
  }

  if (loading) {
    return (
      <div className="p-8 animate-fade-in">
        <div className="animate-pulse">
          <div className="h-8 bg-white/10 rounded w-64 mb-4" />
          <div className="h-4 bg-white/5 rounded w-48 mb-8" />
          <div className="space-y-4">
            {[1, 2, 3].map(i => (
              <div key={i} className="h-24 bg-white/5 rounded-xl" />
            ))}
          </div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-8">
        <Link
          to={`/${repo}/${activeBranch}/agents/${agentId}`}
          className="inline-flex items-center gap-2 text-zinc-400 hover:text-white mb-6"
        >
          <ArrowLeft className="w-4 h-4" /> Back to agent
        </Link>
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4 text-red-300">
          <AlertTriangle className="w-5 h-5 inline mr-2" />
          {error}
        </div>
      </div>
    )
  }

  if (!tree) return null

  const { conversation, messages } = tree

  return (
    <div className="p-8 max-w-5xl mx-auto animate-fade-in">
      {/* Header */}
      <div className="mb-6">
        <Link
          to={`/${repo}/${activeBranch}/agents/${agentId}`}
          className="inline-flex items-center gap-2 text-zinc-400 hover:text-white mb-4"
        >
          <ArrowLeft className="w-4 h-4" /> Back to agent
        </Link>

        <div className="flex items-start justify-between">
          <div>
            <div className="flex items-center gap-3 mb-2">
              <MessageSquare className="w-7 h-7 text-primary-400" />
              <h1 className="text-2xl font-bold text-white">
                {conversation.subject || conversation.name}
              </h1>
              {conversation.hasErrors && (
                <span className="px-2 py-0.5 bg-red-500/20 text-red-300 text-xs rounded-full">Errors</span>
              )}
            </div>
            <div className="flex flex-wrap items-center gap-4 text-sm text-zinc-400">
              <code className="text-xs bg-white/5 px-2 py-0.5 rounded font-mono">{conversation.path}</code>
              {conversation.status && (
                <span className={`px-2 py-0.5 text-xs rounded-full ${
                  conversation.status === 'active' ? 'bg-green-500/20 text-green-300' : 'bg-zinc-500/20 text-zinc-300'
                }`}>{conversation.status}</span>
              )}
            </div>
            <div className="flex items-center gap-4 mt-2 text-xs text-zinc-500">
              {conversation.createdAt && (
                <span className="flex items-center gap-1">
                  <Clock className="w-3 h-3" />
                  Started: {new Date(conversation.createdAt).toLocaleString()}
                </span>
              )}
              {conversation.updatedAt && (
                <span>Last active: {new Date(conversation.updatedAt).toLocaleString()}</span>
              )}
              <span>{messages.length} message{messages.length !== 1 ? 's' : ''}</span>
            </div>
          </div>
        </div>
      </div>

      {/* Tab bar */}
      <div className="flex gap-1 mb-6 border-b border-white/10">
        <button
          onClick={() => setActiveTab('timeline')}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            activeTab === 'timeline'
              ? 'border-primary-400 text-primary-300'
              : 'border-transparent text-zinc-400 hover:text-zinc-300'
          }`}
        >
          Timeline
        </button>
        <button
          onClick={() => setActiveTab('raw')}
          className={`px-4 py-2 text-sm font-medium border-b-2 transition-colors ${
            activeTab === 'raw'
              ? 'border-primary-400 text-primary-300'
              : 'border-transparent text-zinc-400 hover:text-zinc-300'
          }`}
        >
          Raw JSON
        </button>
      </div>

      {/* Content */}
      {activeTab === 'timeline' ? (
        messages.length === 0 ? (
          <div className="text-center py-16">
            <MessageSquare className="w-12 h-12 text-zinc-600 mx-auto mb-3" />
            <p className="text-zinc-500">No messages in this conversation</p>
          </div>
        ) : (
          <div className="space-y-1">
            {messages.map(msg => (
              <MessageBubble key={msg.path} message={msg} />
            ))}
          </div>
        )
      ) : (
        <RawJsonViewer data={tree} title="Conversation Tree" />
      )}
    </div>
  )
}
