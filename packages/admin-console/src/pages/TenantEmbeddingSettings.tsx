import { useState, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import { Sparkles, Key, Eye, EyeOff, Settings, CheckCircle, XCircle, Loader2, Info, ChevronDown, ChevronRight } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ToastContainer, useToast } from '../components/Toast'
import { embeddingsApi, ConfigResponse, SetConfigRequest, TestConnectionResponse } from '../api/embeddings'
import { ApiError } from '../api/client'

// Model configurations
const OPENAI_MODELS = [
  { id: 'text-embedding-3-small', name: 'text-embedding-3-small (1536 dims)', dims: 1536 },
  { id: 'text-embedding-3-large', name: 'text-embedding-3-large (3072 dims)', dims: 3072 },
  { id: 'text-embedding-ada-002', name: 'text-embedding-ada-002 (1536 dims - Legacy)', dims: 1536 },
]

const CLAUDE_MODELS = [
  { id: 'voyage-large-2-instruct', name: 'Voyage Large 2 Instruct (1024 dims)', dims: 1024 },
  { id: 'voyage-code-2', name: 'Voyage Code 2 (1536 dims)', dims: 1536 },
  { id: 'voyage-3', name: 'Voyage 3 (1024 dims)', dims: 1024 },
  { id: 'voyage-3-lite', name: 'Voyage 3 Lite (512 dims)', dims: 512 },
]

const OLLAMA_MODELS = [
  { id: 'nomic-embed-text', name: 'Nomic Embed Text (768 dims)', dims: 768 },
  { id: 'all-minilm', name: 'All-MiniLM (384 dims)', dims: 384 },
  { id: 'mxbai-embed-large', name: 'mxbai-embed-large (1024 dims)', dims: 1024 },
  { id: 'snowflake-arctic-embed', name: 'Snowflake Arctic Embed (1024 dims)', dims: 1024 },
]

const HUGGINGFACE_MODELS = [
  { id: 'nomic-ai/nomic-embed-text-v1.5', name: 'Nomic Embed Text v1.5 (768 dims)', dims: 768 },
  { id: 'sentence-transformers/all-MiniLM-L6-v2', name: 'All-MiniLM-L6-v2 (384 dims)', dims: 384 },
]

type Provider = 'OpenAI' | 'Claude' | 'Ollama' | 'HuggingFace'

interface ProviderCardProps {
  provider: Provider
  selected: boolean
  disabled: boolean
  onSelect: () => void
}

function ProviderCard({ provider, selected, disabled, onSelect }: ProviderCardProps) {
  const config = {
    OpenAI: {
      name: 'OpenAI',
      description: 'Industry-leading embedding models',
      icon: '🤖',
    },
    Claude: {
      name: 'Claude (Voyage AI)',
      description: 'Optimized for code and semantic search',
      icon: '🚀',
    },
    Ollama: {
      name: 'Ollama',
      description: 'Self-hosted open source models',
      icon: '🦙',
    },
    HuggingFace: {
      name: 'HuggingFace',
      description: 'Local models via Candle inference',
      icon: '🤗',
    },
  }

  const { name, description, icon } = config[provider]

  return (
    <button
      onClick={onSelect}
      disabled={disabled}
      className={`
        relative glass rounded-xl p-4 transition-all duration-200
        ${selected ? 'border-2 border-purple-500 shadow-lg shadow-purple-500/20' : 'border border-white/10 hover:border-white/20'}
        ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer hover:scale-[1.02]'}
      `}
      aria-pressed={selected}
      aria-disabled={disabled}
    >
      {disabled && (
        <div className="absolute top-2 right-2 px-2 py-1 bg-yellow-500/20 border border-yellow-500/30 rounded text-xs text-yellow-300">
          Coming Soon
        </div>
      )}
      <div className="flex items-center gap-3">
        <div className="text-3xl">{icon}</div>
        <div className="flex-1 text-left">
          <h3 className="text-white font-medium">{name}</h3>
          <p className="text-gray-400 text-sm">{description}</p>
        </div>
        {selected && !disabled && (
          <CheckCircle className="w-5 h-5 text-purple-400" />
        )}
      </div>
    </button>
  )
}

export default function TenantEmbeddingSettings() {
  const toast = useToast()
  const { tenant = 'default' } = useParams<{ tenant: string }>()

  // State
  const [config, setConfig] = useState<ConfigResponse | null>(null)
  const [apiKey, setApiKey] = useState('')
  const [showApiKey, setShowApiKey] = useState(false)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<TestConnectionResponse | null>(null)
  const [hasChanges, setHasChanges] = useState(false)
  const [showAdvanced, setShowAdvanced] = useState(false)
  const [showConfirmEnable, setShowConfirmEnable] = useState(false)

  // Form state (derived from config)
  const [enabled, setEnabled] = useState(false)
  const [provider, setProvider] = useState<Provider>('OpenAI')
  const [model, setModel] = useState('')
  const [dimensions, setDimensions] = useState(1536)
  const [includeName, setIncludeName] = useState(true)
  const [includePath, setIncludePath] = useState(true)
  const [maxEmbeddings, setMaxEmbeddings] = useState<number | null>(null)
  const [baseUrl, setBaseUrl] = useState('')

  // Load configuration on mount
  useEffect(() => {
    loadConfig()
  }, [tenant])

  // Track changes
  useEffect(() => {
    if (!config) return

    const hasChanged =
      enabled !== config.enabled ||
      provider !== config.provider ||
      model !== config.model ||
      dimensions !== config.dimensions ||
      includeName !== config.include_name ||
      includePath !== config.include_path ||
      maxEmbeddings !== config.max_embeddings_per_repo ||
      apiKey !== ''

    setHasChanges(hasChanged)
  }, [config, enabled, provider, model, dimensions, includeName, includePath, maxEmbeddings, apiKey])

  const loadConfig = async () => {
    try {
      setLoading(true)
      const data = await embeddingsApi.getConfig(tenant)
      setConfig(data)

      // Initialize form state
      setEnabled(data.enabled)
      setProvider(data.provider)
      setModel(data.model)
      setDimensions(data.dimensions)
      setIncludeName(data.include_name)
      setIncludePath(data.include_path)
      setMaxEmbeddings(data.max_embeddings_per_repo)
      setBaseUrl(data.base_url || '')
    } catch (error) {
      console.error('Failed to load config:', error)
      toast.error('Failed to load configuration', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setLoading(false)
    }
  }

  const handleSave = async () => {
    try {
      setSaving(true)

      // Validate
      if (enabled && provider !== 'Ollama' && !config?.has_api_key && !apiKey) {
        toast.error('API Key Required', 'Please enter an API key to enable embeddings')
        return
      }

      if (enabled && !model) {
        toast.error('Model Required', 'Please select a model')
        return
      }

      if (enabled && dimensions <= 0) {
        toast.error('Invalid Dimensions', 'Dimensions must be a positive number')
        return
      }

      const request: SetConfigRequest = {
        enabled,
        provider,
        model,
        dimensions,
        api_key_plain: apiKey || undefined,
        include_name: includeName,
        include_path: includePath,
        node_type_settings: config?.node_type_settings || {},
        max_embeddings_per_repo: maxEmbeddings,
        base_url: baseUrl || undefined,
      }

      const result = await embeddingsApi.setConfig(tenant, request)
      setConfig(result)
      setApiKey('') // Clear API key input after save
      setHasChanges(false)
      setTestResult(null) // Clear test results

      toast.success('Configuration Saved', 'Embedding settings have been updated successfully')
    } catch (error) {
      console.error('Failed to save config:', error)
      toast.error('Failed to save configuration', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setSaving(false)
    }
  }

  const handleTestConnection = async () => {
    try {
      setTesting(true)
      setTestResult(null)

      const result = await embeddingsApi.testConnection(tenant)
      setTestResult(result)

      if (result.success) {
        toast.success('Connection Successful', `Connected to ${result.model} (${result.dimensions} dimensions)`)
      } else {
        toast.error('Connection Failed', result.error || 'Unknown error')
      }
    } catch (error) {
      console.error('Failed to test connection:', error)
      const errorMessage = error instanceof ApiError ? error.message : 'Unknown error'
      setTestResult({
        success: false,
        model: model,
        error: errorMessage,
      })
      toast.error('Connection Test Failed', errorMessage)
    } finally {
      setTesting(false)
    }
  }

  const handleCancel = () => {
    if (!config) return

    // Reset to original values
    setEnabled(config.enabled)
    setProvider(config.provider)
    setModel(config.model)
    setDimensions(config.dimensions)
    setIncludeName(config.include_name)
    setIncludePath(config.include_path)
    setMaxEmbeddings(config.max_embeddings_per_repo)
    setApiKey('')
    setHasChanges(false)
    setTestResult(null)
  }

  const handleProviderChange = (newProvider: Provider) => {
    setProvider(newProvider)

    // Reset model selection when provider changes
    const models = newProvider === 'OpenAI' ? OPENAI_MODELS : newProvider === 'Claude' ? CLAUDE_MODELS : newProvider === 'HuggingFace' ? HUGGINGFACE_MODELS : OLLAMA_MODELS
    if (models.length > 0) {
      setModel(models[0].id)
      setDimensions(models[0].dims)
    }
  }

  const handleModelChange = (modelId: string) => {
    setModel(modelId)

    // Update dimensions based on selected model
    const models = provider === 'OpenAI' ? OPENAI_MODELS : provider === 'Claude' ? CLAUDE_MODELS : provider === 'HuggingFace' ? HUGGINGFACE_MODELS : OLLAMA_MODELS
    const selectedModel = models.find(m => m.id === modelId)
    if (selectedModel) {
      setDimensions(selectedModel.dims)
    }
  }

  const handleEnableToggle = (newEnabled: boolean) => {
    if (newEnabled && !config?.enabled) {
      // Show confirmation dialog when enabling for the first time
      setShowConfirmEnable(true)
    } else {
      setEnabled(newEnabled)
    }
  }

  const confirmEnable = () => {
    setEnabled(true)
    setShowConfirmEnable(false)
  }

  const getAvailableModels = () => {
    switch (provider) {
      case 'OpenAI':
        return OPENAI_MODELS
      case 'Claude':
        return CLAUDE_MODELS
      case 'Ollama':
        return OLLAMA_MODELS
      case 'HuggingFace':
        return HUGGINGFACE_MODELS
      default:
        return []
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="w-8 h-8 text-purple-400 animate-spin" />
      </div>
    )
  }

  const availableModels = getAvailableModels()
  const canTestConnection = enabled && (config?.has_api_key || apiKey) && model

  return (
    <div className="space-y-6">
      <ToastContainer toasts={toast.toasts} onClose={toast.closeToast} />

      <ConfirmDialog
        open={showConfirmEnable}
        title="Enable Vector Embeddings"
        message="Vector embeddings enable AI-powered semantic search across your content. This will allow the system to generate vector representations of your nodes for similarity-based queries. Do you want to enable this feature?"
        confirmText="Enable Embeddings"
        cancelText="Cancel"
        variant="info"
        onConfirm={confirmEnable}
        onCancel={() => setShowConfirmEnable(false)}
      />

      {/* Header */}
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-3">
          <div className="p-3 bg-purple-500/20 border border-purple-500/30 rounded-xl">
            <Sparkles className="w-6 h-6 text-purple-400" />
          </div>
          <div>
            <h1 className="text-3xl font-bold text-white">Embedding Configuration</h1>
            <p className="text-gray-400">Configure AI vector embeddings for tenant: {tenant}</p>
          </div>
        </div>

        {/* Enable/Disable Toggle */}
        <label className="flex items-center gap-3 cursor-pointer group">
          <span className="text-white font-medium">Enable Embeddings</span>
          <div className="relative">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(e) => handleEnableToggle(e.target.checked)}
              className="sr-only peer"
            />
            <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
            <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
          </div>
        </label>
      </div>

      {/* Provider Selection */}
      <GlassCard className={!enabled ? 'opacity-50 pointer-events-none' : ''}>
        <h2 className="text-xl font-bold text-white mb-4 flex items-center gap-2">
          <Sparkles className="w-5 h-5 text-purple-400" />
          Select Provider
        </h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          <ProviderCard
            provider="OpenAI"
            selected={provider === 'OpenAI'}
            disabled={false}
            onSelect={() => handleProviderChange('OpenAI')}
          />
          <ProviderCard
            provider="Claude"
            selected={provider === 'Claude'}
            disabled={false}
            onSelect={() => handleProviderChange('Claude')}
          />
          <ProviderCard
            provider="Ollama"
            selected={provider === 'Ollama'}
            disabled={false}
            onSelect={() => handleProviderChange('Ollama')}
          />
          <ProviderCard
            provider="HuggingFace"
            selected={provider === 'HuggingFace'}
            disabled={true}
            onSelect={() => handleProviderChange('HuggingFace')}
          />
        </div>
      </GlassCard>

      {/* Model Selection */}
      <GlassCard className={!enabled ? 'opacity-50 pointer-events-none' : ''}>
        <h2 className="text-xl font-bold text-white mb-4 flex items-center gap-2">
          <Settings className="w-5 h-5 text-purple-400" />
          Model Configuration
        </h2>
        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Model
            </label>
            <select
              value={model}
              onChange={(e) => handleModelChange(e.target.value)}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
            >
              {availableModels.map((m) => (
                <option key={m.id} value={m.id} className="bg-gray-900">
                  {m.name}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Dimensions
            </label>
            <input
              type="number"
              value={dimensions}
              onChange={(e) => setDimensions(parseInt(e.target.value) || 0)}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              min="1"
            />
            <p className="text-sm text-gray-400 mt-1">
              Vector dimensions for the selected model (automatically set based on model selection)
            </p>
          </div>
        </div>
      </GlassCard>

      {/* Connection Settings */}
      <GlassCard className={!enabled ? 'opacity-50 pointer-events-none' : ''}>
        <h2 className="text-xl font-bold text-white mb-4 flex items-center gap-2">
          <Key className="w-5 h-5 text-purple-400" />
          {provider === 'Ollama' ? 'Connection Settings' : 'API Key'}
        </h2>
        <div className="space-y-4">
          {/* Base URL field for Ollama */}
          {provider === 'Ollama' && (
            <div>
              <label className="block text-sm text-gray-400 mb-1">Base URL</label>
              <input
                type="text"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder="http://localhost:11434"
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              />
              <p className="text-xs text-gray-500 mt-1">Leave empty for local Ollama (localhost:11434). Set for remote/hosted instances.</p>
            </div>
          )}

          {config?.has_api_key && (
            <div className="flex items-center gap-2 px-3 py-2 bg-green-500/10 border border-green-500/30 rounded-lg">
              <CheckCircle className="w-5 h-5 text-green-400" />
              <span className="text-green-300 text-sm font-medium">API key configured</span>
            </div>
          )}

          <div>
            {provider === 'Ollama' && <label className="block text-sm text-gray-400 mb-1">API Key (optional, for authenticated instances)</label>}
            <div className="relative">
              <input
                type={showApiKey ? 'text' : 'password'}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={provider === 'Ollama' ? 'Optional API key for remote Ollama' : config?.has_api_key ? 'Enter new API key to update' : 'Enter your API key'}
                className="w-full px-4 py-2 pr-12 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              />
            <button
              type="button"
              onClick={() => setShowApiKey(!showApiKey)}
              className="absolute right-3 top-1/2 -translate-y-1/2 p-1 hover:bg-white/10 rounded transition-colors"
              aria-label={showApiKey ? 'Hide API key' : 'Show API key'}
            >
              {showApiKey ? (
                <EyeOff className="w-5 h-5 text-gray-400" />
              ) : (
                <Eye className="w-5 h-5 text-gray-400" />
              )}
            </button>
          </div>
          </div>

          {/* Test Connection */}
          <div className="flex items-center gap-3">
            <button
              onClick={handleTestConnection}
              disabled={!canTestConnection || testing}
              className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all flex items-center gap-2"
            >
              {testing ? (
                <>
                  <Loader2 className="w-4 h-4 animate-spin" />
                  Testing...
                </>
              ) : (
                <>
                  <CheckCircle className="w-4 h-4" />
                  Test Connection
                </>
              )}
            </button>

            {testResult && (
              <div className={`flex items-center gap-2 px-3 py-2 rounded-lg ${
                testResult.success
                  ? 'bg-green-500/10 border border-green-500/30'
                  : 'bg-red-500/10 border border-red-500/30'
              }`}>
                {testResult.success ? (
                  <>
                    <CheckCircle className="w-5 h-5 text-green-400" />
                    <span className="text-green-300 text-sm">
                      Connected: {testResult.model} ({testResult.dimensions} dims)
                    </span>
                  </>
                ) : (
                  <>
                    <XCircle className="w-5 h-5 text-red-400" />
                    <span className="text-red-300 text-sm">
                      {testResult.error || 'Connection failed'}
                    </span>
                  </>
                )}
              </div>
            )}
          </div>
        </div>
      </GlassCard>

      {/* Content Generation Options */}
      <GlassCard className={!enabled ? 'opacity-50 pointer-events-none' : ''}>
        <h2 className="text-xl font-bold text-white mb-4 flex items-center gap-2">
          <Settings className="w-5 h-5 text-purple-400" />
          Content Generation Options
        </h2>
        <div className="space-y-3">
          <label className="flex items-center gap-3 cursor-pointer group">
            <input
              type="checkbox"
              checked={includeName}
              onChange={(e) => setIncludeName(e.target.checked)}
              className="w-4 h-4 text-purple-500 border-white/20 rounded focus:ring-purple-500 focus:ring-2"
            />
            <div className="flex-1">
              <span className="text-white font-medium">Include node name in embeddings</span>
              <p className="text-sm text-gray-400">Node names will be included when generating embeddings</p>
            </div>
            <Info className="w-4 h-4 text-gray-500 group-hover:text-gray-400" />
          </label>

          <label className="flex items-center gap-3 cursor-pointer group">
            <input
              type="checkbox"
              checked={includePath}
              onChange={(e) => setIncludePath(e.target.checked)}
              className="w-4 h-4 text-purple-500 border-white/20 rounded focus:ring-purple-500 focus:ring-2"
            />
            <div className="flex-1">
              <span className="text-white font-medium">Include node path in embeddings</span>
              <p className="text-sm text-gray-400">Full node paths will be included for context</p>
            </div>
            <Info className="w-4 h-4 text-gray-500 group-hover:text-gray-400" />
          </label>
        </div>
      </GlassCard>

      {/* Advanced Settings */}
      <GlassCard className={!enabled ? 'opacity-50 pointer-events-none' : ''}>
        <button
          onClick={() => setShowAdvanced(!showAdvanced)}
          className="w-full flex items-center justify-between text-left group"
        >
          <h2 className="text-xl font-bold text-white flex items-center gap-2">
            <Settings className="w-5 h-5 text-purple-400" />
            Advanced Settings
          </h2>
          {showAdvanced ? (
            <ChevronDown className="w-5 h-5 text-gray-400 group-hover:text-white transition-colors" />
          ) : (
            <ChevronRight className="w-5 h-5 text-gray-400 group-hover:text-white transition-colors" />
          )}
        </button>

        {showAdvanced && (
          <div className="mt-4 pt-4 border-t border-white/10">
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2">
                Max Embeddings Per Repository
              </label>
              <input
                type="number"
                value={maxEmbeddings ?? ''}
                onChange={(e) => setMaxEmbeddings(e.target.value ? parseInt(e.target.value) : null)}
                placeholder="Unlimited"
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                min="0"
              />
              <p className="text-sm text-gray-400 mt-1">
                Limit the number of nodes that can have embeddings generated per repository (leave empty for unlimited)
              </p>
            </div>
          </div>
        )}
      </GlassCard>

      {/* Action Buttons */}
      <div className="flex items-center justify-end gap-3">
        <button
          onClick={handleCancel}
          disabled={!hasChanges || saving}
          className="px-6 py-2 bg-white/10 hover:bg-white/20 disabled:bg-white/5 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all"
        >
          Cancel
        </button>
        <button
          onClick={handleSave}
          disabled={!hasChanges || saving}
          className="px-6 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all flex items-center gap-2"
        >
          {saving ? (
            <>
              <Loader2 className="w-4 h-4 animate-spin" />
              Saving...
            </>
          ) : (
            <>
              <CheckCircle className="w-4 h-4" />
              Save Configuration
            </>
          )}
        </button>
      </div>
    </div>
  )
}
