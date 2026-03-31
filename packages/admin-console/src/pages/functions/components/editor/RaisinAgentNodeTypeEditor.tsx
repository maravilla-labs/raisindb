/**
 * Raisin Agent Node Type Editor Component
 *
 * Form-based editor for raisin:AIAgent node properties.
 * Opens as a tab (like files) with save, undo/redo functionality.
 */

import { useEffect, useCallback, useState, useRef } from 'react'
import { Save, Undo2, Redo2, Loader2, Bot, Plus, Trash2, MessageSquare, PanelRightClose, ArrowUp, ArrowDown, Pencil, Check, X } from 'lucide-react'
import { Allotment } from 'allotment'
import { useFunctionsContext, useUndoRedo } from '../../hooks'
import { nodesApi } from '../../../../api/nodes'
import { aiApi, type AIConfig, type ProviderConfigResponse, type AIProvider } from '../../../../api/ai'
import CommitDialog from '../../../../components/CommitDialog'
import { InlineFunctionPicker } from './InlineFunctionPicker'
import { AgentTestChat } from './AgentTestChat'
import type { EditorTab } from '../../types'

// Use "default" as tenant ID for single-tenant mode
const TENANT_ID = 'default'

// Helper to convert provider array to map
function providersArrayToMap(providers: ProviderConfigResponse[]): Record<AIProvider, ProviderConfigResponse | undefined> {
  const map: Record<AIProvider, ProviderConfigResponse | undefined> = {
    openai: undefined,
    anthropic: undefined,
    google: undefined,
    azure_openai: undefined,
    ollama: undefined,
    groq: undefined,
    openrouter: undefined,
    bedrock: undefined,
    local: undefined,
    custom: undefined,
  }
  for (const p of providers) {
    map[p.provider] = p
  }
  return map
}

// Helper function to get display name for providers
function getProviderDisplayName(provider: AIProvider): string {
  const names: Record<AIProvider, string> = {
    openai: 'OpenAI',
    anthropic: 'Anthropic (Claude)',
    google: 'Google Gemini',
    azure_openai: 'Azure OpenAI',
    ollama: 'Ollama (Local)',
    groq: 'Groq',
    openrouter: 'OpenRouter',
    bedrock: 'AWS Bedrock',
    local: 'Local (Candle)',
    custom: 'Custom',
  }
  return names[provider] || provider
}

interface RaisinAgentNodeTypeEditorProps {
  tab: EditorTab
}

type Provider = 'openai' | 'anthropic' | 'google' | 'azure_openai' | 'ollama' | 'groq' | 'openrouter' | 'bedrock' | 'local' | 'custom'

/** Parse tools from DB — extracts path strings from any format (string, RaisinReference, etc.) */
function parseToolPaths(raw: unknown): string[] {
  if (!Array.isArray(raw)) return []
  return raw
    .map((t): string | null => {
      if (typeof t === 'string') return t
      if (t && typeof t === 'object' && 'raisin:path' in t) return (t as Record<string, string>)['raisin:path']
      return null
    })
    .filter((t): t is string => !!t)
}

type ExecutionContext = 'user' | 'system'
type ExecutionMode = 'automatic' | 'approve_then_auto' | 'step_by_step' | 'manual'

interface AgentProperties {
  system_prompt: string
  provider: Provider
  model: string
  temperature: number
  max_tokens: number
  thinking_enabled: boolean
  task_creation_enabled: boolean
  execution_mode: ExecutionMode
  execution_context: ExecutionContext
  tools: string[]
  rules: string[]
  compaction_enabled: boolean
  compaction_token_threshold: number
  compaction_keep_recent: number
  compaction_provider: string
  compaction_model: string
  compaction_prompt: string
}

interface AgentNode {
  id: string
  name: string
  path: string
  node_type: string
  properties?: Record<string, unknown>
  created_at?: string
  updated_at?: string
}

const DEFAULT_PROPERTIES: AgentProperties = {
  system_prompt: '',
  provider: 'openai',
  model: '',
  temperature: 0.7,
  max_tokens: 4096,
  thinking_enabled: false,
  task_creation_enabled: false,
  execution_mode: 'automatic',
  execution_context: 'user',
  tools: [],
  rules: [],
  compaction_enabled: true,
  compaction_token_threshold: 8000,
  compaction_keep_recent: 10,
  compaction_provider: '',
  compaction_model: '',
  compaction_prompt: '',
}

export function RaisinAgentNodeTypeEditor({ tab }: RaisinAgentNodeTypeEditorProps) {
  const {
    repo,
    branch,
    workspace,
    nodes,
    markTabDirty,
    loadRootNodes,
    addLog,
  } = useFunctionsContext()

  // State
  const [isLoading, setIsLoading] = useState(true)
  const [pendingCommit, setPendingCommit] = useState<{ properties: AgentProperties } | null>(null)
  const [agentNode, setAgentNode] = useState<AgentNode | null>(null)
  const [aiConfig, setAiConfig] = useState<AIConfig | null>(null)
  const [availableModels, setAvailableModels] = useState<string[]>([])
  const [newRule, setNewRule] = useState('')
  const [showTestChat, setShowTestChat] = useState(false)
  const [editingRuleIndex, setEditingRuleIndex] = useState<number | null>(null)
  const [editingRuleText, setEditingRuleText] = useState('')

  // Undo/redo for properties
  const {
    value: properties,
    setValue: setProperties,
    undo,
    redo,
    canUndo,
    canRedo,
    reset: resetProperties,
    isDirty,
  } = useUndoRedo<AgentProperties>(DEFAULT_PROPERTIES)

  // Ref for keyboard shortcuts
  const containerRef = useRef<HTMLDivElement>(null)

  // Find agent node from tree
  const findAgentNode = useCallback((nodeList: typeof nodes, path: string): AgentNode | null => {
    for (const node of nodeList) {
      if (node.path === path && node.node_type === 'raisin:AIAgent') {
        return node as unknown as AgentNode
      }
      if (node.children && Array.isArray(node.children)) {
        const found = findAgentNode(node.children as typeof nodes, path)
        if (found) return found
      }
    }
    return null
  }, [])

  // Load AI config
  useEffect(() => {
    async function loadConfig() {
      try {
        const config = await aiApi.getConfig(TENANT_ID)
        setAiConfig(config)
      } catch (error) {
        console.error('Failed to load AI config:', error)
      }
    }

    loadConfig()
  }, [])

  // Update available models when provider changes
  useEffect(() => {
    if (aiConfig && properties.provider) {
      const providerMap = providersArrayToMap(aiConfig.providers)
      const providerConfig = providerMap[properties.provider]
      if (providerConfig?.models) {
        const models = providerConfig.models.map((m) => m.model_id)
        setAvailableModels(models)

        // If current model is not available in new provider, clear it
        if (properties.model && !models.includes(properties.model)) {
          setProperties({ ...properties, model: models[0] || '' })
        }
      } else {
        setAvailableModels([])
      }
    }
  }, [properties.provider, aiConfig])

  // Load agent node data
  useEffect(() => {
    const loadNode = async () => {
      setIsLoading(true)
      try {
        // First try to find in tree
        let node = findAgentNode(nodes, tab.path)

        // If not found, fetch from server
        if (!node) {
          const fetchedNode = await nodesApi.getAtHead(repo, branch, workspace, tab.path)
          if (fetchedNode.node_type === 'raisin:AIAgent') {
            node = fetchedNode as unknown as AgentNode
          }
        }

        if (node) {
          setAgentNode(node)
          const props: AgentProperties = {
            system_prompt: (node.properties?.system_prompt as string) || '',
            provider: (node.properties?.provider as Provider) || 'openai',
            model: (node.properties?.model as string) || '',
            temperature: (node.properties?.temperature as number) ?? 0.7,
            max_tokens: (node.properties?.max_tokens as number) ?? 4096,
            thinking_enabled: (node.properties?.thinking_enabled as boolean) ?? false,
            task_creation_enabled: (node.properties?.task_creation_enabled as boolean) ?? false,
            execution_mode: (node.properties?.execution_mode as ExecutionMode) || 'automatic',
            execution_context: (node.properties?.execution_context as ExecutionContext) || 'user',
            tools: parseToolPaths(node.properties?.tools),
            rules: (node.properties?.rules as string[]) || [],
            compaction_enabled: (node.properties?.compaction_enabled as boolean) ?? true,
            compaction_token_threshold: (node.properties?.compaction_token_threshold as number) ?? 8000,
            compaction_keep_recent: (node.properties?.compaction_keep_recent as number) ?? 10,
            compaction_provider: (node.properties?.compaction_provider as string) || '',
            compaction_model: (node.properties?.compaction_model as string) || '',
            compaction_prompt: (node.properties?.compaction_prompt as string) || '',
          }
          resetProperties(props)
        }
      } catch (error) {
        console.error('Failed to load agent node:', error)
      } finally {
        setIsLoading(false)
      }
    }

    loadNode()
  }, [tab.path, repo, branch, nodes, findAgentNode, resetProperties])

  // Sync dirty state with tab
  useEffect(() => {
    markTabDirty(tab.id, isDirty)
  }, [isDirty, tab.id, markTabDirty])

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const isMac = navigator.platform.toUpperCase().indexOf('MAC') >= 0
      const cmdOrCtrl = isMac ? e.metaKey : e.ctrlKey

      if (cmdOrCtrl && e.key === 's') {
        e.preventDefault()
        if (isDirty) {
          handleSave()
        }
      } else if (cmdOrCtrl && e.key === 'z' && !e.shiftKey) {
        e.preventDefault()
        if (canUndo) {
          undo()
        }
      } else if (cmdOrCtrl && e.shiftKey && e.key === 'z') {
        e.preventDefault()
        if (canRedo) {
          redo()
        }
      } else if (cmdOrCtrl && e.key === 'y') {
        e.preventDefault()
        if (canRedo) {
          redo()
        }
      }
    }

    const container = containerRef.current
    if (container) {
      container.addEventListener('keydown', handleKeyDown)
      return () => container.removeEventListener('keydown', handleKeyDown)
    }
  }, [isDirty, canUndo, canRedo, undo, redo])

  // Handle property changes
  const handlePropertiesChange = useCallback((newProps: Partial<AgentProperties>) => {
    setProperties({ ...properties, ...newProps })
  }, [setProperties, properties])

  // Handle save
  const handleSave = useCallback(() => {
    if (!isDirty) return
    setPendingCommit({ properties })
  }, [isDirty, properties])

  // Execute save with commit message
  const executeCommit = useCallback(async (message: string, actor: string) => {
    if (!pendingCommit || !agentNode) return

    try {
      await nodesApi.update(repo, branch, workspace, tab.path, {
        properties: {
          ...agentNode.properties,
          ...pendingCommit.properties,
        },
        commit: { message, actor },
      })

      // Reset undo history with new saved state
      resetProperties(pendingCommit.properties)

      // Refresh the tree
      await loadRootNodes()

      addLog({
        level: 'info',
        message: `Agent "${agentNode.name}" properties saved`,
        timestamp: new Date().toISOString(),
      })
    } catch (error) {
      console.error('Failed to save agent properties:', error)
      addLog({
        level: 'error',
        message: `Failed to save: ${error instanceof Error ? error.message : String(error)}`,
        timestamp: new Date().toISOString(),
      })
    } finally {
      setPendingCommit(null)
    }
  }, [pendingCommit, agentNode, repo, branch, tab.path, resetProperties, loadRootNodes, addLog])

  // Handle rule management
  const handleAddRule = useCallback(() => {
    if (newRule.trim()) {
      setProperties({
        ...properties,
        rules: [...properties.rules, newRule.trim()]
      })
      setNewRule('')
    }
  }, [newRule, properties, setProperties])

  const handleRemoveRule = useCallback((index: number) => {
    setProperties({
      ...properties,
      rules: properties.rules.filter((_, i) => i !== index)
    })
    if (editingRuleIndex === index) {
      setEditingRuleIndex(null)
      setEditingRuleText('')
    }
  }, [properties, setProperties, editingRuleIndex])

  const handleEditRule = useCallback((index: number) => {
    setEditingRuleIndex(index)
    setEditingRuleText(properties.rules[index])
  }, [properties.rules])

  const handleSaveEditedRule = useCallback(() => {
    if (editingRuleIndex === null) return
    const trimmed = editingRuleText.trim()
    if (trimmed) {
      const newRules = [...properties.rules]
      newRules[editingRuleIndex] = trimmed
      setProperties({ ...properties, rules: newRules })
    }
    setEditingRuleIndex(null)
    setEditingRuleText('')
  }, [editingRuleIndex, editingRuleText, properties, setProperties])

  const handleCancelEditRule = useCallback(() => {
    setEditingRuleIndex(null)
    setEditingRuleText('')
  }, [])

  const handleMoveRule = useCallback((index: number, direction: 'up' | 'down') => {
    const newIndex = direction === 'up' ? index - 1 : index + 1
    if (newIndex < 0 || newIndex >= properties.rules.length) return
    const newRules = [...properties.rules]
    const temp = newRules[index]
    newRules[index] = newRules[newIndex]
    newRules[newIndex] = temp
    setProperties({ ...properties, rules: newRules })
  }, [properties, setProperties])

  // Handle tools change
  const handleToolsChange = useCallback((selectedPaths: string[]) => {
    setProperties({ ...properties, tools: selectedPaths })
  }, [properties, setProperties])

  // Loading state
  if (isLoading) {
    return (
      <div className="h-full flex items-center justify-center text-gray-400">
        <Loader2 className="w-6 h-6 animate-spin mr-2" />
        Loading agent properties...
      </div>
    )
  }

  // Error state - agent not found
  if (!agentNode) {
    return (
      <div className="h-full flex flex-col items-center justify-center text-gray-400">
        <Bot className="w-16 h-16 mb-4 opacity-50" />
        <p className="text-lg">Agent not found</p>
      </div>
    )
  }

  const selectedToolPaths = properties.tools

  return (
    <div
      ref={containerRef}
      className="h-full flex flex-col focus:outline-none"
      tabIndex={0}
    >
      {/* Toolbar */}
      <div className="flex-shrink-0 flex items-center gap-2 px-3 py-1.5 bg-black/20 border-b border-white/10">
        {/* Save Button */}
        <button
          onClick={handleSave}
          disabled={!isDirty}
          className={`flex items-center gap-1.5 px-2 py-1 rounded text-sm
            ${isDirty
              ? 'bg-purple-500/20 text-purple-300 hover:bg-purple-500/30'
              : 'text-gray-500 cursor-not-allowed'
            }
          `}
          title="Save (Ctrl+S)"
        >
          <Save className="w-4 h-4" />
          Save
        </button>

        {/* Undo Button */}
        <button
          onClick={undo}
          disabled={!canUndo}
          className={`p-1.5 rounded text-sm
            ${canUndo
              ? 'text-gray-300 hover:bg-white/10'
              : 'text-gray-600 cursor-not-allowed'
            }
          `}
          title="Undo (Ctrl+Z)"
        >
          <Undo2 className="w-4 h-4" />
        </button>

        {/* Redo Button */}
        <button
          onClick={redo}
          disabled={!canRedo}
          className={`p-1.5 rounded text-sm
            ${canRedo
              ? 'text-gray-300 hover:bg-white/10'
              : 'text-gray-600 cursor-not-allowed'
            }
          `}
          title="Redo (Ctrl+Shift+Z)"
        >
          <Redo2 className="w-4 h-4" />
        </button>

        <div className="flex-1" />

        {/* Test Chat Toggle Button */}
        <button
          onClick={() => setShowTestChat(!showTestChat)}
          className={`flex items-center gap-1.5 px-2 py-1 rounded text-sm transition-colors ${
            showTestChat
              ? 'bg-purple-500/30 text-purple-300'
              : 'text-gray-400 hover:bg-white/10 hover:text-gray-200'
          }`}
          title={showTestChat ? 'Close Test Chat' : 'Open Test Chat'}
        >
          {showTestChat ? (
            <PanelRightClose className="w-4 h-4" />
          ) : (
            <MessageSquare className="w-4 h-4" />
          )}
          {showTestChat ? 'Hide Chat' : 'Test Chat'}
        </button>

        <span className="text-xs text-gray-500 ml-2">AI Agent</span>
      </div>

      {/* Content - Split Panel Layout using Allotment */}
      <div className="flex-1 overflow-hidden">
        <Allotment>
          {/* Editor Panel */}
          <Allotment.Pane minSize={350}>
            <div className="h-full overflow-auto">
              <div className="max-w-2xl mx-auto p-6 space-y-6">
            {/* Header */}
            <div className="mb-6">
              <div className="flex items-center gap-3 mb-2">
                <Bot className="w-8 h-8 text-purple-400" />
                <h2 className="text-lg font-semibold text-white">{agentNode.name}</h2>
              </div>
              <p className="text-sm text-gray-400">{tab.path}</p>
            </div>

            {/* System Prompt */}
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                System Prompt <span className="text-red-400">*</span>
              </label>
              <textarea
                rows={8}
                value={properties.system_prompt}
                onChange={(e) => handlePropertiesChange({ system_prompt: e.target.value })}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20 resize-y"
                placeholder="You are a helpful AI assistant that..."
              />
              <p className="text-xs text-zinc-500 mt-1">Define the agent's behavior and capabilities</p>
            </div>

            {/* Provider and Model */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-zinc-300 mb-2">
                  Provider
                </label>
                <select
                  value={properties.provider}
                  onChange={(e) => handlePropertiesChange({ provider: e.target.value as Provider })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                >
                  {aiConfig?.providers
                    .filter(p => p.enabled && p.has_api_key)
                    .map(p => (
                      <option key={p.provider} value={p.provider}>
                        {getProviderDisplayName(p.provider)}
                      </option>
                    ))}
                  {/* Fallback if no providers configured */}
                  {(!aiConfig?.providers || aiConfig.providers.filter(p => p.enabled && p.has_api_key).length === 0) && (
                    <option value="" disabled>No providers configured</option>
                  )}
                </select>
              </div>

              <div>
                <label className="block text-sm font-medium text-zinc-300 mb-2">
                  Model
                </label>
                {availableModels.length > 0 ? (
                  <select
                    value={properties.model}
                    onChange={(e) => handlePropertiesChange({ model: e.target.value })}
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                  >
                    <option value="">Select a model</option>
                    {availableModels.map(model => (
                      <option key={model} value={model}>{model}</option>
                    ))}
                  </select>
                ) : (
                  <input
                    type="text"
                    value={properties.model}
                    onChange={(e) => handlePropertiesChange({ model: e.target.value })}
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                    placeholder="gpt-4"
                  />
                )}
              </div>
            </div>

            {/* Temperature and Max Tokens */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-zinc-300 mb-2">
                  Temperature: {properties.temperature}
                </label>
                <input
                  type="range"
                  min="0"
                  max="2"
                  step="0.1"
                  value={properties.temperature}
                  onChange={(e) => handlePropertiesChange({ temperature: parseFloat(e.target.value) })}
                  className="w-full accent-purple-500"
                />
                <p className="text-xs text-zinc-500 mt-1">Controls randomness (0 = deterministic, 2 = very creative)</p>
              </div>

              <div>
                <label className="block text-sm font-medium text-zinc-300 mb-2">
                  Max Tokens
                </label>
                <input
                  type="number"
                  min="1"
                  max="32000"
                  value={properties.max_tokens}
                  onChange={(e) => handlePropertiesChange({ max_tokens: parseInt(e.target.value) || 4096 })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                />
                <p className="text-xs text-zinc-500 mt-1">Maximum tokens in the response</p>
              </div>
            </div>

            {/* Feature Toggles */}
            <div className="space-y-4">
              <div className="flex items-center gap-3">
                <input
                  type="checkbox"
                  id="thinking_enabled"
                  checked={properties.thinking_enabled}
                  onChange={(e) => handlePropertiesChange({ thinking_enabled: e.target.checked })}
                  className="w-5 h-5 bg-white/5 border border-white/10 rounded text-purple-500 focus:ring-2 focus:ring-purple-500/20"
                />
                <label htmlFor="thinking_enabled" className="text-sm font-medium text-zinc-300">
                  Enable Extended Thinking
                </label>
              </div>
              <p className="text-xs text-zinc-500 ml-8">Allow the agent to use extended thinking for complex reasoning</p>

              <div className="flex items-center gap-3">
                <input
                  type="checkbox"
                  id="task_creation_enabled"
                  checked={properties.task_creation_enabled}
                  onChange={(e) => handlePropertiesChange({ task_creation_enabled: e.target.checked })}
                  className="w-5 h-5 bg-white/5 border border-white/10 rounded text-purple-500 focus:ring-2 focus:ring-purple-500/20"
                />
                <label htmlFor="task_creation_enabled" className="text-sm font-medium text-zinc-300">
                  Enable Task Creation
                </label>
              </div>
              <p className="text-xs text-zinc-500 ml-8">Allow the agent to create and manage tasks</p>

              {properties.task_creation_enabled && (
                <div className="ml-8">
                  <label className="block text-sm font-medium text-zinc-300 mb-2">
                    Execution Mode
                  </label>
                  <select
                    value={properties.execution_mode}
                    onChange={(e) => handlePropertiesChange({ execution_mode: e.target.value as ExecutionMode })}
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                  >
                    <option value="automatic">Automatic — execute tasks immediately</option>
                    <option value="approve_then_auto">Approve Then Auto — approve plan, then run all tasks</option>
                    <option value="step_by_step">Step-by-Step — approve each task before execution</option>
                    <option value="manual">Manual — no automatic task execution</option>
                  </select>
                  <p className="text-xs text-zinc-500 mt-1">Controls whether user approval is required before task execution</p>
                </div>
              )}
            </div>

            {/* Execution Context */}
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                Function Execution Context
              </label>
              <select
                value={properties.execution_context}
                onChange={(e) => handlePropertiesChange({ execution_context: e.target.value as ExecutionContext })}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
              >
                <option value="user">User Context (Recommended)</option>
                <option value="system">System Context (Elevated Privileges)</option>
              </select>
              <p className="text-xs text-zinc-500 mt-1">
                Who functions execute as when called by this agent
              </p>
              {properties.execution_context === 'system' && (
                <p className="text-xs text-amber-500 mt-1">
                  Warning: System context bypasses user permissions. Use with caution.
                </p>
              )}
            </div>

            {/* Tools Selection */}
            <div>
              <InlineFunctionPicker
                label="Tools"
                value={selectedToolPaths}
                onChange={handleToolsChange}
                placeholder="Search functions..."
                helperText="Select functions this agent can use as tools"
              />
              <p className="text-xs text-amber-500/80 mt-1">
                Note: Make sure your selected model supports tool calling
              </p>
            </div>

            {/* History Compaction */}
            <div className="space-y-4">
              <h3 className="text-base font-medium text-zinc-200 border-b border-white/10 pb-2">History Compaction</h3>
              <div className="flex items-center gap-3">
                <input
                  type="checkbox"
                  id="compaction_enabled"
                  checked={properties.compaction_enabled}
                  onChange={(e) => handlePropertiesChange({ compaction_enabled: e.target.checked })}
                  className="w-5 h-5 bg-white/5 border border-white/10 rounded text-purple-500 focus:ring-2 focus:ring-purple-500/20"
                />
                <label htmlFor="compaction_enabled" className="text-sm font-medium text-zinc-300">
                  Enable History Compaction
                </label>
              </div>
              <p className="text-xs text-zinc-500 ml-8">Automatically summarize older messages to reduce token usage in long conversations</p>

              {properties.compaction_enabled && (
                <>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-2">
                        Token Threshold
                      </label>
                      <input
                        type="number"
                        min="1000"
                        max="100000"
                        value={properties.compaction_token_threshold}
                        onChange={(e) => handlePropertiesChange({ compaction_token_threshold: parseInt(e.target.value) || 8000 })}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                      />
                      <p className="text-xs text-zinc-500 mt-1">Trigger compaction when history exceeds this token count</p>
                    </div>

                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-2">
                        Keep Recent Messages
                      </label>
                      <input
                        type="number"
                        min="1"
                        max="100"
                        value={properties.compaction_keep_recent}
                        onChange={(e) => handlePropertiesChange({ compaction_keep_recent: parseInt(e.target.value) || 10 })}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                      />
                      <p className="text-xs text-zinc-500 mt-1">Number of recent messages to keep uncompacted</p>
                    </div>
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-2">
                        Compaction Provider <span className="text-zinc-500">(optional)</span>
                      </label>
                      <select
                        value={properties.compaction_provider}
                        onChange={(e) => handlePropertiesChange({ compaction_provider: e.target.value })}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                      >
                        <option value="">Use agent's provider</option>
                        {aiConfig?.providers
                          .filter(p => p.enabled && p.has_api_key)
                          .map(p => (
                            <option key={p.provider} value={p.provider}>
                              {getProviderDisplayName(p.provider)}
                            </option>
                          ))}
                      </select>
                      <p className="text-xs text-zinc-500 mt-1">Override provider for summarization</p>
                    </div>

                    <div>
                      <label className="block text-sm font-medium text-zinc-300 mb-2">
                        Compaction Model <span className="text-zinc-500">(optional)</span>
                      </label>
                      <input
                        type="text"
                        value={properties.compaction_model}
                        onChange={(e) => handlePropertiesChange({ compaction_model: e.target.value })}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                        placeholder="Use agent's model"
                      />
                      <p className="text-xs text-zinc-500 mt-1">Override model for summarization</p>
                    </div>
                  </div>

                  <div>
                    <label className="block text-sm font-medium text-zinc-300 mb-2">
                      Compaction Prompt <span className="text-zinc-500">(optional)</span>
                    </label>
                    <textarea
                      rows={4}
                      value={properties.compaction_prompt}
                      onChange={(e) => handlePropertiesChange({ compaction_prompt: e.target.value })}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20 resize-y"
                      placeholder="Custom summarization prompt (uses system default if empty)"
                    />
                    <p className="text-xs text-zinc-500 mt-1">Custom prompt template for summarizing older messages</p>
                  </div>
                </>
              )}
            </div>

            {/* Rules */}
            <div>
              <label className="block text-sm font-medium text-zinc-300 mb-2">
                Rules
              </label>
              <div className="space-y-2">
                {properties.rules.map((rule, index) => (
                  <div key={index} className="flex items-start gap-1 p-3 bg-white/5 border border-white/10 rounded-lg group">
                    {editingRuleIndex === index ? (
                      <>
                        <textarea
                          rows={2}
                          value={editingRuleText}
                          onChange={(e) => setEditingRuleText(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter' && !e.shiftKey) {
                              e.preventDefault()
                              handleSaveEditedRule()
                            }
                            if (e.key === 'Escape') {
                              handleCancelEditRule()
                            }
                          }}
                          className="flex-1 px-2 py-1 bg-white/5 border border-purple-500/50 rounded text-sm text-white placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-purple-500/30 resize-y"
                          autoFocus
                        />
                        <button
                          type="button"
                          onClick={handleSaveEditedRule}
                          className="p-1 hover:bg-green-500/20 text-green-400 rounded transition-colors"
                          title="Save (Enter)"
                        >
                          <Check className="w-4 h-4" />
                        </button>
                        <button
                          type="button"
                          onClick={handleCancelEditRule}
                          className="p-1 hover:bg-zinc-500/20 text-zinc-400 rounded transition-colors"
                          title="Cancel (Esc)"
                        >
                          <X className="w-4 h-4" />
                        </button>
                      </>
                    ) : (
                      <>
                        <p className="flex-1 text-sm text-zinc-300">{rule}</p>
                        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                          <button
                            type="button"
                            onClick={() => handleMoveRule(index, 'up')}
                            disabled={index === 0}
                            className="p-1 hover:bg-white/10 text-zinc-400 disabled:text-zinc-600 disabled:cursor-not-allowed rounded transition-colors"
                            title="Move up"
                          >
                            <ArrowUp className="w-3.5 h-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={() => handleMoveRule(index, 'down')}
                            disabled={index === properties.rules.length - 1}
                            className="p-1 hover:bg-white/10 text-zinc-400 disabled:text-zinc-600 disabled:cursor-not-allowed rounded transition-colors"
                            title="Move down"
                          >
                            <ArrowDown className="w-3.5 h-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={() => handleEditRule(index)}
                            className="p-1 hover:bg-purple-500/20 text-purple-400 rounded transition-colors"
                            title="Edit rule"
                          >
                            <Pencil className="w-3.5 h-3.5" />
                          </button>
                          <button
                            type="button"
                            onClick={() => handleRemoveRule(index)}
                            className="p-1 hover:bg-red-500/20 text-red-400 rounded transition-colors"
                            title="Remove rule"
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                ))}
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={newRule}
                    onChange={(e) => setNewRule(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault()
                        handleAddRule()
                      }
                    }}
                    className="flex-1 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-purple-500 focus:outline-none focus:ring-2 focus:ring-purple-500/20"
                    placeholder="Add a rule..."
                  />
                  <button
                    type="button"
                    onClick={handleAddRule}
                    className="px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors flex items-center gap-2"
                  >
                    <Plus className="w-4 h-4" />
                    Add
                  </button>
                </div>
              </div>
              <p className="text-xs text-zinc-500 mt-1">Define rules and constraints for the agent</p>
            </div>
          </div>
        </div>
          </Allotment.Pane>

          {/* Test Chat Panel */}
          {showTestChat && (
            <Allotment.Pane preferredSize={400} minSize={300} maxSize={600}>
              <div className="h-full border-l border-white/10">
                <AgentTestChat
                  repo={repo}
                  branch={branch}
                  agentPath={tab.path}
                  agentName={agentNode.name}
                  agentId={agentNode.id}
                />
              </div>
            </Allotment.Pane>
          )}
        </Allotment>
      </div>

      {/* Commit Dialog */}
      {pendingCommit && (
        <CommitDialog
          title="Save Agent Properties"
          action={`Update properties for "${agentNode.name}"`}
          onCommit={executeCommit}
          onClose={() => setPendingCommit(null)}
        />
      )}
    </div>
  )
}
