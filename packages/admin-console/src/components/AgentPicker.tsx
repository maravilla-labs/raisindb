import { useState, useEffect } from 'react'
import { createPortal } from 'react-dom'
import { X, Bot, Search, CheckCircle, Sparkles, Wrench, Brain } from 'lucide-react'
import { agentsApi, type Agent } from '../api/agents'

interface AgentRef {
  path: string
  workspace: string
}

interface AgentPickerProps {
  repo: string
  branch?: string
  currentAgentRef?: AgentRef | null
  onSelect: (agentRef: AgentRef) => void
  onClose: () => void
}

export default function AgentPicker({
  repo,
  branch = 'main',
  currentAgentRef,
  onSelect,
  onClose,
}: AgentPickerProps) {
  const [agents, setAgents] = useState<Agent[]>([])
  const [loading, setLoading] = useState(true)
  const [searchQuery, setSearchQuery] = useState('')
  const [selectedAgent, setSelectedAgent] = useState<Agent | null>(null)

  useEffect(() => {
    loadAgents()
  }, [repo, branch])

  async function loadAgents() {
    setLoading(true)
    try {
      const data = await agentsApi.list(repo, branch)
      setAgents(data)

      // Pre-select current agent if one is set
      if (currentAgentRef) {
        const current = data.find(
          (a) => a.path === currentAgentRef.path || a.id === currentAgentRef.path
        )
        if (current) {
          setSelectedAgent(current)
        }
      }
    } catch (error) {
      console.error('Failed to load agents:', error)
    } finally {
      setLoading(false)
    }
  }

  function handleConfirm() {
    if (selectedAgent) {
      const agentRef: AgentRef = {
        path: selectedAgent.path,
        workspace: 'functions',
      }
      onSelect(agentRef)
    }
  }

  // Filter agents based on search query
  const filteredAgents = agents.filter(
    (agent) =>
      agent.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      agent.properties.system_prompt.toLowerCase().includes(searchQuery.toLowerCase()) ||
      agent.properties.model.toLowerCase().includes(searchQuery.toLowerCase())
  )

  return createPortal(
    <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
      <div className="glass-dark rounded-xl max-w-3xl w-full max-h-[90vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex justify-between items-start p-6 border-b border-white/10">
          <div>
            <h2 className="text-2xl font-bold text-white flex items-center gap-2">
              <Bot className="w-6 h-6 text-purple-400" />
              Select AI Agent
            </h2>
            <p className="text-sm text-gray-400 mt-1">
              Choose an agent to handle this flow step
            </p>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-white/10 rounded-lg transition-colors"
          >
            <X className="w-6 h-6 text-gray-400" />
          </button>
        </div>

        {/* Search */}
        <div className="p-4 border-b border-white/10">
          <div className="relative">
            <Search className="absolute left-3 top-1/2 transform -translate-y-1/2 w-5 h-5 text-zinc-400" />
            <input
              type="text"
              placeholder="Search agents by name, prompt, or model..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full pl-10 pr-4 py-3 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
              autoFocus
            />
          </div>
        </div>

        {/* Agent List */}
        <div className="flex-1 overflow-y-auto p-4">
          {loading ? (
            <div className="text-center text-zinc-400 py-12">Loading agents...</div>
          ) : filteredAgents.length === 0 ? (
            <div className="text-center py-12">
              <Bot className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
              <h3 className="text-xl font-semibold text-white mb-2">
                {searchQuery ? 'No agents found' : 'No agents available'}
              </h3>
              <p className="text-zinc-400">
                {searchQuery
                  ? 'Try adjusting your search query'
                  : 'Create agents in the Functions workspace first'}
              </p>
            </div>
          ) : (
            <div className="space-y-2">
              {filteredAgents.map((agent) => {
                const isSelected = selectedAgent?.id === agent.id
                const toolsCount = agent.properties.tools?.length || 0

                return (
                  <div
                    key={agent.id}
                    onClick={() => setSelectedAgent(agent)}
                    className={`p-4 rounded-lg cursor-pointer transition-all ${
                      isSelected
                        ? 'bg-purple-500/30 border-2 border-purple-500'
                        : 'bg-white/5 border-2 border-transparent hover:bg-white/10 hover:border-white/20'
                    }`}
                  >
                    <div className="flex items-start justify-between">
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-3 mb-2">
                          <Bot className={`w-5 h-5 ${isSelected ? 'text-purple-300' : 'text-purple-400'}`} />
                          <h4 className="text-lg font-semibold text-white truncate">
                            {agent.name}
                          </h4>
                          {isSelected && (
                            <CheckCircle className="w-5 h-5 text-green-400 flex-shrink-0" />
                          )}
                        </div>

                        <p className="text-sm text-zinc-300 mb-3 line-clamp-2">
                          {agent.properties.system_prompt}
                        </p>

                        <div className="flex flex-wrap gap-2">
                          <span className="inline-flex items-center gap-1 px-2 py-1 bg-zinc-700/50 text-zinc-300 text-xs rounded-full">
                            <Sparkles className="w-3 h-3" />
                            {agent.properties.provider} / {agent.properties.model}
                          </span>
                          {toolsCount > 0 && (
                            <span className="inline-flex items-center gap-1 px-2 py-1 bg-primary-500/20 text-primary-300 text-xs rounded-full">
                              <Wrench className="w-3 h-3" />
                              {toolsCount} {toolsCount === 1 ? 'tool' : 'tools'}
                            </span>
                          )}
                          {agent.properties.thinking_enabled && (
                            <span className="inline-flex items-center gap-1 px-2 py-1 bg-purple-500/20 text-purple-300 text-xs rounded-full">
                              <Brain className="w-3 h-3" />
                              Thinking
                            </span>
                          )}
                        </div>
                      </div>
                    </div>
                  </div>
                )
              })}
            </div>
          )}
        </div>

        {/* Footer */}
        <div className="flex gap-3 justify-end p-4 border-t border-white/10 bg-white/5">
          <button
            onClick={onClose}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleConfirm}
            disabled={!selectedAgent}
            className="flex items-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Bot className="w-4 h-4" />
            Select Agent
          </button>
        </div>
      </div>
    </div>,
    document.body
  )
}
