import { useEffect, useState } from 'react'
import { useNavigate, useParams, Link } from 'react-router-dom'
import { ArrowLeft, Save, Bot, Plus, Trash2, ArrowUp, ArrowDown, Pencil, Check, X } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import TagSelector from '../../components/TagSelector'
import { useToast, ToastContainer } from '../../components/Toast'
import { agentsApi, type CreateAgentRequest, type UpdateAgentRequest } from '../../api/agents'
import { aiApi, type AIConfig, type ProviderConfigResponse, type AIProvider } from '../../api/ai'

import { nodesApi } from '../../api/nodes'

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

type Provider = 'openai' | 'anthropic' | 'google' | 'azure_openai' | 'ollama' | 'groq' | 'openrouter' | 'bedrock' | 'local' | 'custom'

// Helper function to get display name for providers
function getProviderDisplayName(provider: AIProvider): string {
  const names: Record<AIProvider, string> = {
    openai: 'OpenAI',
    anthropic: 'Anthropic',
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

interface FormData {
  name: string
  system_prompt: string
  provider: Provider
  model: string
  temperature: number
  max_tokens: number
  thinking_enabled: boolean
  task_creation_enabled: boolean
  tools: string[]
  rules: string[]
  compaction_enabled: boolean
  compaction_token_threshold: number
  compaction_keep_recent: number
  compaction_provider: string
  compaction_model: string
  compaction_prompt: string
}

export default function AgentEditor() {
  const { repo, branch, agentId } = useParams<{ repo: string; branch?: string; agentId?: string }>()
  const activeBranch = branch || 'main'
  const navigate = useNavigate()
  const isNew = agentId === 'new'

  // When editing an existing agent, back goes to agent detail; when creating new, back goes to list
  const backPath = isNew
    ? `/${repo}/${activeBranch}/agents`
    : `/${repo}/${activeBranch}/agents/${agentId}`

  const [loading, setLoading] = useState(!isNew)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const { toasts, error: showError, success: showSuccess, closeToast } = useToast()

  const [formData, setFormData] = useState<FormData>({
    name: '',
    system_prompt: '',
    provider: 'openai',
    model: '',
    temperature: 0.7,
    max_tokens: 4096,
    thinking_enabled: false,
    task_creation_enabled: false,
    tools: [],
    rules: [],
    compaction_enabled: true,
    compaction_token_threshold: 8000,
    compaction_keep_recent: 10,
    compaction_provider: '',
    compaction_model: '',
    compaction_prompt: '',
  })

  const [aiConfig, setAiConfig] = useState<AIConfig | null>(null)
  const [availableModels, setAvailableModels] = useState<string[]>([])
  const [availableFunctions, setAvailableFunctions] = useState<string[]>([])
  const [newRule, setNewRule] = useState('')
  const [editingRuleIndex, setEditingRuleIndex] = useState<number | null>(null)
  const [editingRuleText, setEditingRuleText] = useState('')

  useEffect(() => {
    loadAIConfig()
    loadFunctions()
    if (!isNew && agentId) {
      loadAgent()
    }
  }, [repo, activeBranch, agentId, isNew])

  useEffect(() => {
    // Update available models when provider changes
    if (aiConfig && formData.provider) {
      const providerMap = providersArrayToMap(aiConfig.providers)
      const providerConfig = providerMap[formData.provider]
      if (providerConfig?.models) {
        const models = providerConfig.models.map((m) => m.model_id)
        setAvailableModels(models)

        // If current model is not available in new provider, clear it
        if (formData.model && !models.includes(formData.model)) {
          setFormData(prev => ({ ...prev, model: models[0] || '' }))
        }
      } else {
        setAvailableModels([])
      }
    }
  }, [formData.provider, aiConfig])

  async function loadAIConfig() {
    try {
      const config = await aiApi.getConfig(TENANT_ID)
      setAiConfig(config)
    } catch (error) {
      console.error('Failed to load AI config:', error)
    }
  }

  async function loadFunctions() {
    if (!repo) return
    try {
      // List all functions from the functions workspace
      const functions = await nodesApi.listChildrenAtHead(repo, activeBranch, 'functions', '/')
      const functionPaths = functions
        .filter(f => f.node_type === 'raisin:Function')
        .map(f => f.path)
      setAvailableFunctions(functionPaths)
    } catch (error) {
      console.error('Failed to load functions:', error)
    }
  }

  async function loadAgent() {
    if (!repo || !agentId) return
    setLoading(true)
    setError(null)

    try {
      const agent = await agentsApi.get(repo, agentId, activeBranch)
      setFormData({
        name: agent.name,
        system_prompt: agent.properties.system_prompt,
        provider: agent.properties.provider,
        model: agent.properties.model,
        temperature: agent.properties.temperature,
        max_tokens: agent.properties.max_tokens,
        thinking_enabled: agent.properties.thinking_enabled,
        task_creation_enabled: agent.properties.task_creation_enabled,
        tools: agent.properties.tools || [],
        rules: agent.properties.rules || [],
        compaction_enabled: agent.properties.compaction_enabled ?? true,
        compaction_token_threshold: agent.properties.compaction_token_threshold ?? 8000,
        compaction_keep_recent: agent.properties.compaction_keep_recent ?? 10,
        compaction_provider: agent.properties.compaction_provider || '',
        compaction_model: agent.properties.compaction_model || '',
        compaction_prompt: agent.properties.compaction_prompt || '',
      })
    } catch (err) {
      console.error('Failed to load agent:', err)
      setError('Failed to load agent')
      showError('Load Failed', 'Failed to load agent')
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
      if (isNew) {
        const request: CreateAgentRequest = {
          name: formData.name.trim(),
          system_prompt: formData.system_prompt.trim(),
          provider: formData.provider,
          model: formData.model,
          temperature: formData.temperature,
          max_tokens: formData.max_tokens,
          thinking_enabled: formData.thinking_enabled,
          task_creation_enabled: formData.task_creation_enabled,
          tools: formData.tools,
          rules: formData.rules,
          compaction_enabled: formData.compaction_enabled,
          compaction_token_threshold: formData.compaction_token_threshold,
          compaction_keep_recent: formData.compaction_keep_recent,
          compaction_provider: formData.compaction_provider || undefined,
          compaction_model: formData.compaction_model || undefined,
          compaction_prompt: formData.compaction_prompt || undefined,
        }
        await agentsApi.create(repo, request, activeBranch)
        showSuccess('Created', 'Agent created successfully')
      } else {
        const request: UpdateAgentRequest = {
          system_prompt: formData.system_prompt.trim(),
          provider: formData.provider,
          model: formData.model,
          temperature: formData.temperature,
          max_tokens: formData.max_tokens,
          thinking_enabled: formData.thinking_enabled,
          task_creation_enabled: formData.task_creation_enabled,
          tools: formData.tools,
          rules: formData.rules,
          compaction_enabled: formData.compaction_enabled,
          compaction_token_threshold: formData.compaction_token_threshold,
          compaction_keep_recent: formData.compaction_keep_recent,
          compaction_provider: formData.compaction_provider || undefined,
          compaction_model: formData.compaction_model || undefined,
          compaction_prompt: formData.compaction_prompt || undefined,
        }
        await agentsApi.update(repo, agentId!, request, activeBranch)
        showSuccess('Updated', 'Agent updated successfully')
      }

      navigate(backPath)
    } catch (err: any) {
      console.error('Failed to save agent:', err)
      const errorMessage = err.message || 'Failed to save agent'
      setError(errorMessage)
      showError('Save Failed', errorMessage)
    } finally {
      setSaving(false)
    }
  }

  function handleAddRule() {
    if (newRule.trim()) {
      setFormData(prev => ({
        ...prev,
        rules: [...prev.rules, newRule.trim()]
      }))
      setNewRule('')
    }
  }

  function handleRemoveRule(index: number) {
    setFormData(prev => ({
      ...prev,
      rules: prev.rules.filter((_, i) => i !== index)
    }))
    if (editingRuleIndex === index) {
      setEditingRuleIndex(null)
      setEditingRuleText('')
    }
  }

  function handleEditRule(index: number) {
    setEditingRuleIndex(index)
    setEditingRuleText(formData.rules[index])
  }

  function handleSaveEditedRule() {
    if (editingRuleIndex === null) return
    const trimmed = editingRuleText.trim()
    if (trimmed) {
      setFormData(prev => {
        const newRules = [...prev.rules]
        newRules[editingRuleIndex!] = trimmed
        return { ...prev, rules: newRules }
      })
    }
    setEditingRuleIndex(null)
    setEditingRuleText('')
  }

  function handleCancelEditRule() {
    setEditingRuleIndex(null)
    setEditingRuleText('')
  }

  function handleMoveRule(index: number, direction: 'up' | 'down') {
    const newIndex = direction === 'up' ? index - 1 : index + 1
    if (newIndex < 0 || newIndex >= formData.rules.length) return
    setFormData(prev => {
      const newRules = [...prev.rules]
      const temp = newRules[index]
      newRules[index] = newRules[newIndex]
      newRules[newIndex] = temp
      return { ...prev, rules: newRules }
    })
  }

  function handleToolsChange(selectedPaths: string[]) {
    setFormData(prev => ({ ...prev, tools: selectedPaths }))
  }

  if (loading) {
    return (
      <div className="animate-fade-in">
        <div className="text-center text-zinc-400 py-12">Loading agent...</div>
      </div>
    )
  }

  const selectedToolPaths = formData.tools

  return (
    <div className="animate-fade-in">
      <div className="mb-8 flex items-center gap-4">
        <Link
          to={backPath}
          className="p-2 hover:bg-white/10 rounded-lg transition-colors"
        >
          <ArrowLeft className="w-6 h-6 text-zinc-400" />
        </Link>
        <div>
          <h1 className="text-4xl font-bold text-white flex items-center gap-3">
            <Bot className="w-10 h-10 text-primary-400" />
            {isNew ? 'Create Agent' : `Edit Agent: ${formData.name}`}
          </h1>
          <p className="text-zinc-400 mt-2">
            {isNew ? 'Configure a new AI agent' : 'Update agent configuration'}
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
          {/* Name */}
          <div>
            <label htmlFor="name" className="block text-sm font-medium text-zinc-300 mb-2">
              Name <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              id="name"
              required
              disabled={!isNew}
              value={formData.name}
              onChange={(e) => setFormData({ ...formData, name: e.target.value })}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20 disabled:opacity-50 disabled:cursor-not-allowed"
              placeholder="my-agent"
            />
            <p className="text-xs text-zinc-500 mt-1">Unique identifier for the agent</p>
          </div>

          {/* System Prompt */}
          <div>
            <label htmlFor="system_prompt" className="block text-sm font-medium text-zinc-300 mb-2">
              System Prompt <span className="text-red-400">*</span>
            </label>
            <textarea
              id="system_prompt"
              required
              rows={8}
              value={formData.system_prompt}
              onChange={(e) => setFormData({ ...formData, system_prompt: e.target.value })}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20 resize-y"
              placeholder="You are a helpful AI assistant that..."
            />
            <p className="text-xs text-zinc-500 mt-1">Define the agent's behavior and capabilities</p>
          </div>

          {/* Provider and Model */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label htmlFor="provider" className="block text-sm font-medium text-zinc-300 mb-2">
                Provider <span className="text-red-400">*</span>
              </label>
              <select
                id="provider"
                required
                value={formData.provider}
                onChange={(e) => setFormData({ ...formData, provider: e.target.value as Provider })}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
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
              <label htmlFor="model" className="block text-sm font-medium text-zinc-300 mb-2">
                Model <span className="text-red-400">*</span>
              </label>
              {availableModels.length > 0 ? (
                <select
                  id="model"
                  required
                  value={formData.model}
                  onChange={(e) => setFormData({ ...formData, model: e.target.value })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                >
                  <option value="">Select a model</option>
                  {availableModels.map(model => (
                    <option key={model} value={model}>{model}</option>
                  ))}
                </select>
              ) : (
                <input
                  type="text"
                  id="model"
                  required
                  value={formData.model}
                  onChange={(e) => setFormData({ ...formData, model: e.target.value })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                  placeholder="gpt-4"
                />
              )}
            </div>
          </div>

          {/* Temperature and Max Tokens */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div>
              <label htmlFor="temperature" className="block text-sm font-medium text-zinc-300 mb-2">
                Temperature: {formData.temperature}
              </label>
              <input
                type="range"
                id="temperature"
                min="0"
                max="2"
                step="0.1"
                value={formData.temperature}
                onChange={(e) => setFormData({ ...formData, temperature: parseFloat(e.target.value) })}
                className="w-full"
              />
              <p className="text-xs text-zinc-500 mt-1">Controls randomness (0 = deterministic, 2 = very creative)</p>
            </div>

            <div>
              <label htmlFor="max_tokens" className="block text-sm font-medium text-zinc-300 mb-2">
                Max Tokens
              </label>
              <input
                type="number"
                id="max_tokens"
                min="1"
                max="32000"
                value={formData.max_tokens}
                onChange={(e) => setFormData({ ...formData, max_tokens: parseInt(e.target.value) || 4096 })}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
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
                checked={formData.thinking_enabled}
                onChange={(e) => setFormData({ ...formData, thinking_enabled: e.target.checked })}
                className="w-5 h-5 bg-white/5 border border-white/10 rounded text-primary-500 focus:ring-2 focus:ring-primary-500/20"
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
                checked={formData.task_creation_enabled}
                onChange={(e) => setFormData({ ...formData, task_creation_enabled: e.target.checked })}
                className="w-5 h-5 bg-white/5 border border-white/10 rounded text-primary-500 focus:ring-2 focus:ring-primary-500/20"
              />
              <label htmlFor="task_creation_enabled" className="text-sm font-medium text-zinc-300">
                Enable Task Creation
              </label>
            </div>
            <p className="text-xs text-zinc-500 ml-8">Allow the agent to create and manage tasks</p>
          </div>

          {/* History Compaction */}
          <div className="space-y-4">
            <h3 className="text-lg font-medium text-zinc-200 border-b border-white/10 pb-2">History Compaction</h3>
            <div className="flex items-center gap-3">
              <input
                type="checkbox"
                id="compaction_enabled"
                checked={formData.compaction_enabled}
                onChange={(e) => setFormData({ ...formData, compaction_enabled: e.target.checked })}
                className="w-5 h-5 bg-white/5 border border-white/10 rounded text-primary-500 focus:ring-2 focus:ring-primary-500/20"
              />
              <label htmlFor="compaction_enabled" className="text-sm font-medium text-zinc-300">
                Enable History Compaction
              </label>
            </div>
            <p className="text-xs text-zinc-500 ml-8">Automatically summarize older messages to reduce token usage in long conversations</p>

            {formData.compaction_enabled && (
              <>
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div>
                    <label htmlFor="compaction_token_threshold" className="block text-sm font-medium text-zinc-300 mb-2">
                      Token Threshold
                    </label>
                    <input
                      type="number"
                      id="compaction_token_threshold"
                      min="1000"
                      max="100000"
                      value={formData.compaction_token_threshold}
                      onChange={(e) => setFormData({ ...formData, compaction_token_threshold: parseInt(e.target.value) || 8000 })}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                    />
                    <p className="text-xs text-zinc-500 mt-1">Trigger compaction when history exceeds this token count</p>
                  </div>

                  <div>
                    <label htmlFor="compaction_keep_recent" className="block text-sm font-medium text-zinc-300 mb-2">
                      Keep Recent Messages
                    </label>
                    <input
                      type="number"
                      id="compaction_keep_recent"
                      min="1"
                      max="100"
                      value={formData.compaction_keep_recent}
                      onChange={(e) => setFormData({ ...formData, compaction_keep_recent: parseInt(e.target.value) || 10 })}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                    />
                    <p className="text-xs text-zinc-500 mt-1">Number of recent messages to keep uncompacted</p>
                  </div>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div>
                    <label htmlFor="compaction_provider" className="block text-sm font-medium text-zinc-300 mb-2">
                      Compaction Provider <span className="text-zinc-500">(optional)</span>
                    </label>
                    <select
                      id="compaction_provider"
                      value={formData.compaction_provider}
                      onChange={(e) => setFormData({ ...formData, compaction_provider: e.target.value })}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
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
                    <label htmlFor="compaction_model" className="block text-sm font-medium text-zinc-300 mb-2">
                      Compaction Model <span className="text-zinc-500">(optional)</span>
                    </label>
                    <input
                      type="text"
                      id="compaction_model"
                      value={formData.compaction_model}
                      onChange={(e) => setFormData({ ...formData, compaction_model: e.target.value })}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                      placeholder="Use agent's model"
                    />
                    <p className="text-xs text-zinc-500 mt-1">Override model for summarization</p>
                  </div>
                </div>

                <div>
                  <label htmlFor="compaction_prompt" className="block text-sm font-medium text-zinc-300 mb-2">
                    Compaction Prompt <span className="text-zinc-500">(optional)</span>
                  </label>
                  <textarea
                    id="compaction_prompt"
                    rows={4}
                    value={formData.compaction_prompt}
                    onChange={(e) => setFormData({ ...formData, compaction_prompt: e.target.value })}
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20 resize-y"
                    placeholder="Custom summarization prompt (uses system default if empty)"
                  />
                  <p className="text-xs text-zinc-500 mt-1">Custom prompt template for summarizing older messages</p>
                </div>
              </>
            )}
          </div>

          {/* Tools Selection */}
          <div>
            <TagSelector
              label="Tools"
              value={selectedToolPaths}
              onChange={handleToolsChange}
              suggestions={availableFunctions}
              allowCustom={false}
              placeholder="Select functions..."
              helperText="Select functions this agent can use"
            />
          </div>

          {/* Rules */}
          <div>
            <label className="block text-sm font-medium text-zinc-300 mb-2">
              Rules
            </label>
            <div className="space-y-2">
              {formData.rules.map((rule, index) => (
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
                        className="flex-1 px-2 py-1 bg-white/5 border border-primary-500/50 rounded text-sm text-white placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-primary-500/30 resize-y"
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
                          disabled={index === formData.rules.length - 1}
                          className="p-1 hover:bg-white/10 text-zinc-400 disabled:text-zinc-600 disabled:cursor-not-allowed rounded transition-colors"
                          title="Move down"
                        >
                          <ArrowDown className="w-3.5 h-3.5" />
                        </button>
                        <button
                          type="button"
                          onClick={() => handleEditRule(index)}
                          className="p-1 hover:bg-primary-500/20 text-primary-400 rounded transition-colors"
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
                  className="flex-1 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-zinc-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-500/20"
                  placeholder="Add a rule..."
                />
                <button
                  type="button"
                  onClick={handleAddRule}
                  className="px-4 py-2 bg-primary-500 hover:bg-primary-600 text-white rounded-lg transition-colors flex items-center gap-2"
                >
                  <Plus className="w-4 h-4" />
                  Add
                </button>
              </div>
            </div>
            <p className="text-xs text-zinc-500 mt-1">Define rules and constraints for the agent</p>
          </div>

          {/* Submit Buttons */}
          <div className="flex gap-4 pt-4">
            <button
              type="submit"
              disabled={saving}
              className="flex-1 flex items-center justify-center gap-2 px-6 py-3 bg-primary-500 hover:bg-primary-600 disabled:bg-primary-500/50 text-white rounded-lg transition-colors font-medium"
            >
              <Save className="w-5 h-5" />
              {saving ? 'Saving...' : isNew ? 'Create Agent' : 'Save Changes'}
            </button>
            <Link
              to={backPath}
              className="px-6 py-3 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-colors font-medium"
            >
              Cancel
            </Link>
          </div>
        </form>
      </GlassCard>

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
