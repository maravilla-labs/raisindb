import { useState, useEffect } from 'react'
import {
  Sparkles,
  Key,
  Eye,
  EyeOff,
  Settings,
  CheckCircle,
  XCircle,
  Loader2,
  Info,
  ChevronDown,
  ChevronRight,
  Zap,
  Globe,
  Server,
  AlertCircle,
  RefreshCw,
  Cloud,
  Layers,
  Cpu,
  SlidersHorizontal,
} from 'lucide-react'
import GlassCard from '../components/GlassCard'
import { ToastContainer, useToast } from '../components/Toast'
import HuggingFaceModelsSection from '../components/HuggingFaceModelsSection'
import {
  aiApi,
  AIConfigResponse,
  AIProvider,
  AIModelConfig,
  ProviderConfigResponse,
  ProviderConfigRequest,
  UpdateAIConfigRequest,
  EmbeddingSettings,
  SplitterType,
  DistanceMetric,
  QuantizationType,
  DEFAULT_CHUNKING_SETTINGS,
  DEFAULT_HNSW_PARAMS,
  EMBEDDING_CAPABLE_PROVIDERS,
} from '../api/ai'
import { ApiError } from '../api/client'

// Use "default" as tenant ID for single-tenant mode
const TENANT_ID = 'default'

interface ProviderSectionProps {
  icon: React.ReactNode
  name: string
  description: string
  enabled: boolean
  apiKeyConfigured: boolean
  apiEndpoint?: string
  models: AIModelConfig[]
  onToggle: (enabled: boolean) => void
  onApiKeyChange: (key: string) => void
  onEndpointChange: (endpoint: string) => void
  onTest: () => Promise<void>
  onRefreshModels: () => Promise<void>
  testing: boolean
  refreshing: boolean
  testResult?: { success: boolean; error?: string }
  /** Provider type for custom credential UI (e.g., Bedrock needs separate Access Key + Secret Key) */
  providerType?: string
}

function ProviderSection({
  icon,
  name,
  description,
  enabled,
  apiKeyConfigured,
  apiEndpoint,
  models,
  onToggle,
  onApiKeyChange,
  onEndpointChange,
  onTest,
  onRefreshModels,
  testing,
  refreshing,
  testResult,
  providerType,
}: ProviderSectionProps) {
  const [expanded, setExpanded] = useState(enabled)
  const [showApiKey, setShowApiKey] = useState(false)
  const [apiKey, setApiKey] = useState('')
  const [endpoint, setEndpoint] = useState(apiEndpoint || '')
  const [showEndpoint, setShowEndpoint] = useState(false)

  // Bedrock-specific state: split credentials into Access Key ID + Secret Key
  const isBedrock = providerType === 'bedrock'
  const [bedrockAccessKey, setBedrockAccessKey] = useState('')
  const [bedrockSecretKey, setBedrockSecretKey] = useState('')
  const [bedrockRegion, setBedrockRegion] = useState(apiEndpoint || 'us-east-1')

  const handleBedrockCredentialChange = (accessKey: string, secretKey: string) => {
    setBedrockAccessKey(accessKey)
    setBedrockSecretKey(secretKey)
    // Store as "access_key_id:secret_access_key" in the api_key field
    if (accessKey && secretKey) {
      onApiKeyChange(`${accessKey}:${secretKey}`)
    }
  }

  const handleBedrockRegionChange = (region: string) => {
    setBedrockRegion(region)
    onEndpointChange(region)
  }

  useEffect(() => {
    if (enabled) setExpanded(true)
  }, [enabled])

  useEffect(() => {
    setEndpoint(apiEndpoint || '')
  }, [apiEndpoint])

  const handleApiKeyChange = (value: string) => {
    setApiKey(value)
    onApiKeyChange(value)
  }

  const handleEndpointChange = (value: string) => {
    setEndpoint(value)
    onEndpointChange(value)
  }

  return (
    <GlassCard>
      <div className="space-y-4">
        {/* Header */}
        <div className="flex items-center justify-between">
          <button
            onClick={() => setExpanded(!expanded)}
            className="flex items-center gap-3 flex-1 text-left group"
          >
            <div className={`p-2 rounded-lg ${enabled ? 'bg-purple-500/20 border border-purple-500/30' : 'bg-white/5 border border-white/10'}`}>
              {icon}
            </div>
            <div className="flex-1">
              <h2 className="text-xl font-bold text-white flex items-center gap-2">
                {name}
                {apiKeyConfigured && (
                  <CheckCircle className="w-4 h-4 text-green-400" />
                )}
              </h2>
              <p className="text-gray-400 text-sm">{description}</p>
            </div>
            {expanded ? (
              <ChevronDown className="w-5 h-5 text-gray-400 group-hover:text-white transition-colors" />
            ) : (
              <ChevronRight className="w-5 h-5 text-gray-400 group-hover:text-white transition-colors" />
            )}
          </button>

          {/* Enable Toggle */}
          <label className="flex items-center gap-3 cursor-pointer ml-4">
            <span className="text-white font-medium text-sm">Enabled</span>
            <div className="relative">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => onToggle(e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
              <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
            </div>
          </label>
        </div>

        {/* Expanded Content */}
        {expanded && (
          <div className="space-y-4 pt-4 border-t border-white/10">
            {/* Credentials */}
            {isBedrock ? (
              <>
                {/* Bedrock: Access Key ID + Secret Access Key + Region */}
                {apiKeyConfigured && !bedrockAccessKey && !bedrockSecretKey && (
                  <div className="flex items-center gap-2 px-3 py-2 bg-green-500/10 border border-green-500/30 rounded-lg">
                    <CheckCircle className="w-4 h-4 text-green-400" />
                    <span className="text-green-300 text-sm font-medium">AWS credentials configured</span>
                  </div>
                )}
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                    <Key className="w-4 h-4 text-purple-400" />
                    AWS Access Key ID
                  </label>
                  <input
                    type="text"
                    value={bedrockAccessKey}
                    onChange={(e) => handleBedrockCredentialChange(e.target.value, bedrockSecretKey)}
                    placeholder={apiKeyConfigured ? 'Enter new Access Key ID to update' : 'AKIA...'}
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all font-mono"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                    <Key className="w-4 h-4 text-purple-400" />
                    AWS Secret Access Key
                  </label>
                  <div className="relative">
                    <input
                      type={showApiKey ? 'text' : 'password'}
                      value={bedrockSecretKey}
                      onChange={(e) => handleBedrockCredentialChange(bedrockAccessKey, e.target.value)}
                      placeholder={apiKeyConfigured ? 'Enter new Secret Key to update' : 'Your secret access key'}
                      className="w-full px-4 py-2 pr-12 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all font-mono"
                    />
                    <button
                      type="button"
                      onClick={() => setShowApiKey(!showApiKey)}
                      className="absolute right-3 top-1/2 -translate-y-1/2 p-1 hover:bg-white/10 rounded transition-colors"
                    >
                      {showApiKey ? <EyeOff className="w-5 h-5 text-gray-400" /> : <Eye className="w-5 h-5 text-gray-400" />}
                    </button>
                  </div>
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                    <Globe className="w-4 h-4 text-purple-400" />
                    AWS Region
                  </label>
                  <input
                    type="text"
                    value={bedrockRegion}
                    onChange={(e) => handleBedrockRegionChange(e.target.value)}
                    placeholder="us-east-1"
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                  />
                </div>
              </>
            ) : (
              <>
                {/* Standard providers: API Key */}
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                    <Key className="w-4 h-4 text-purple-400" />
                    API Key
                  </label>
                  {apiKeyConfigured && !apiKey && (
                    <div className="flex items-center gap-2 px-3 py-2 bg-green-500/10 border border-green-500/30 rounded-lg mb-2">
                      <CheckCircle className="w-4 h-4 text-green-400" />
                      <span className="text-green-300 text-sm font-medium">API key configured</span>
                    </div>
                  )}
                  <div className="relative">
                    <input
                      type={showApiKey ? 'text' : 'password'}
                      value={apiKey}
                      onChange={(e) => handleApiKeyChange(e.target.value)}
                      placeholder={apiKeyConfigured ? 'Enter new API key to update' : 'Enter your API key'}
                      className="w-full px-4 py-2 pr-12 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
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

                {/* API Endpoint (Optional) */}
                <div>
                  <button
                    onClick={() => setShowEndpoint(!showEndpoint)}
                    className="flex items-center gap-2 text-sm text-gray-400 hover:text-white transition-colors mb-2"
                  >
                    <Globe className="w-4 h-4" />
                    Custom API Endpoint (Optional)
                    {showEndpoint ? (
                      <ChevronDown className="w-4 h-4" />
                    ) : (
                      <ChevronRight className="w-4 h-4" />
                    )}
                  </button>
                  {showEndpoint && (
                    <input
                      type="text"
                      value={endpoint}
                      onChange={(e) => handleEndpointChange(e.target.value)}
                      placeholder={`Default ${name} endpoint`}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                  )}
                </div>
              </>
            )}

            {/* Test Connection */}
            <div className="flex items-center gap-3">
              <button
                onClick={onTest}
                disabled={!enabled || (!apiKeyConfigured && !apiKey) || testing}
                className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all flex items-center gap-2"
              >
                {testing ? (
                  <>
                    <Loader2 className="w-4 h-4 animate-spin" />
                    Testing...
                  </>
                ) : (
                  <>
                    <Zap className="w-4 h-4" />
                    Test Connection
                  </>
                )}
              </button>

              <button
                onClick={onRefreshModels}
                disabled={!enabled || (!apiKeyConfigured && !apiKey) || refreshing}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 disabled:bg-white/5 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all flex items-center gap-2"
              >
                {refreshing ? (
                  <>
                    <Loader2 className="w-4 h-4 animate-spin" />
                    Refreshing...
                  </>
                ) : (
                  <>
                    <RefreshCw className="w-4 h-4" />
                    Refresh Models
                  </>
                )}
              </button>

              {testResult && (
                <div
                  className={`flex items-center gap-2 px-3 py-2 rounded-lg ${
                    testResult.success
                      ? 'bg-green-500/10 border border-green-500/30'
                      : 'bg-red-500/10 border border-red-500/30'
                  }`}
                >
                  {testResult.success ? (
                    <>
                      <CheckCircle className="w-4 h-4 text-green-400" />
                      <span className="text-green-300 text-sm">Connected</span>
                    </>
                  ) : (
                    <>
                      <XCircle className="w-4 h-4 text-red-400" />
                      <span className="text-red-300 text-sm">
                        {testResult.error || 'Connection failed'}
                      </span>
                    </>
                  )}
                </div>
              )}
            </div>

            {/* Available Models */}
            {models.length > 0 && (
              <div>
                <h3 className="text-sm font-medium text-gray-300 mb-3 flex items-center gap-2">
                  <Sparkles className="w-4 h-4 text-purple-400" />
                  Available Models ({models.length})
                </h3>
                <div className="space-y-2 max-h-60 overflow-y-auto">
                  {models.map((model) => (
                    <div
                      key={model.model_id}
                      className="p-3 bg-white/5 border border-white/10 rounded-lg hover:border-white/20 transition-colors"
                    >
                      <div className="flex items-start justify-between gap-3">
                        <div className="flex-1 min-w-0">
                          <div className="text-white font-medium text-sm truncate">
                            {model.display_name || model.model_id}
                          </div>
                          {/* Show architecture/embedding info from metadata if available */}
                          {model.metadata && (model.metadata.architecture || model.metadata.embedding_length) && (
                            <div className="text-xs text-gray-500 mt-1">
                              {model.metadata.architecture && (
                                <span className="mr-2">arch: {model.metadata.architecture}</span>
                              )}
                              {model.metadata.embedding_length && (
                                <span>dims: {model.metadata.embedding_length}</span>
                              )}
                            </div>
                          )}
                          <div className="flex flex-wrap gap-1 mt-2">
                            {model.use_cases.map((useCase) => (
                              <span
                                key={useCase}
                                className={`px-2 py-0.5 text-xs rounded ${
                                  useCase === 'embedding'
                                    ? 'bg-green-500/20 border border-green-500/30 text-green-300'
                                    : 'bg-purple-500/20 border border-purple-500/30 text-purple-300'
                                }`}
                              >
                                {useCase}
                              </span>
                            ))}
                          </div>
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {models.length === 0 && enabled && (
              <div className="p-4 bg-yellow-500/10 border border-yellow-500/30 rounded-lg flex items-start gap-3">
                <AlertCircle className="w-5 h-5 text-yellow-400 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="text-yellow-300 text-sm font-medium">No models available</p>
                  <p className="text-yellow-300/80 text-xs mt-1">
                    Configure your API key and test the connection to load available models.
                  </p>
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </GlassCard>
  )
}

// Helper to convert provider array to map for local state
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

// Local models section component - simpler than ProviderSection
// Local models are enabled by default and don't require API keys
interface LocalModelsSectionProps {
  enabled: boolean
  onToggle: (enabled: boolean) => void
}

function LocalModelsSection({ enabled, onToggle }: LocalModelsSectionProps) {
  return (
    <GlassCard>
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className={`p-2 rounded-lg ${enabled ? 'bg-purple-500/20 border border-purple-500/30' : 'bg-white/5 border border-white/10'}`}>
              <Cpu className="w-5 h-5 text-cyan-400" />
            </div>
            <div>
              <h2 className="text-xl font-bold text-white flex items-center gap-2">
                Local AI Models
                {enabled && <CheckCircle className="w-4 h-4 text-green-400" />}
              </h2>
              <p className="text-gray-400 text-sm">On-device AI for image captioning and embeddings</p>
            </div>
          </div>

          <label className="flex items-center gap-3 cursor-pointer">
            <span className="text-white font-medium text-sm">Enabled</span>
            <div className="relative">
              <input
                type="checkbox"
                checked={enabled}
                onChange={(e) => onToggle(e.target.checked)}
                className="sr-only peer"
              />
              <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
              <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
            </div>
          </label>
        </div>

        <div className="p-4 bg-cyan-500/10 border border-cyan-500/30 rounded-lg">
          <div className="flex items-start gap-3">
            <Info className="w-5 h-5 text-cyan-400 flex-shrink-0 mt-0.5" />
            <div>
              <p className="text-cyan-300 text-sm font-medium mb-2">No API keys required</p>
              <p className="text-cyan-300/80 text-xs">
                Local models run on your server using the Candle inference engine.
                These models are automatically available without any configuration.
              </p>
              <div className="mt-3 flex flex-wrap gap-2">
                <span className="px-2 py-1 bg-cyan-500/20 border border-cyan-500/30 text-cyan-300 text-xs rounded">
                  Moondream (vision)
                </span>
                <span className="px-2 py-1 bg-cyan-500/20 border border-cyan-500/30 text-cyan-300 text-xs rounded">
                  BLIP (captions)
                </span>
                <span className="px-2 py-1 bg-cyan-500/20 border border-cyan-500/30 text-cyan-300 text-xs rounded">
                  CLIP (embeddings)
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </GlassCard>
  )
}

export default function TenantAiSettings() {
  const toast = useToast()

  // State
  const [config, setConfig] = useState<AIConfigResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [hasChanges, setHasChanges] = useState(false)

  // Provider states (local editable state)
  const [providers, setProviders] = useState<{
    [key in AIProvider]: {
      enabled: boolean
      apiKey: string
      apiEndpoint: string
      testing: boolean
      refreshing: boolean
      testResult?: { success: boolean; error?: string }
    }
  }>({
    openai: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    anthropic: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    google: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    azure_openai: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    ollama: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    groq: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    openrouter: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    bedrock: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
    local: { enabled: true, apiKey: '', apiEndpoint: '', testing: false, refreshing: false }, // Enabled by default
    custom: { enabled: false, apiKey: '', apiEndpoint: '', testing: false, refreshing: false },
  })

  // Local models enabled state (enabled by default, can be disabled)
  // This is separate from the providers state for simpler handling
  const [localModelsEnabled, setLocalModelsEnabled] = useState(true)

  // Embedding settings
  const [embeddingSettings, setEmbeddingSettings] = useState<EmbeddingSettings>({
    enabled: false,
    include_name: true,
    include_path: true,
    dimensions: 1536,
  })
  const [showAdvancedHnsw, setShowAdvancedHnsw] = useState(false)

  // Load configuration on mount
  useEffect(() => {
    loadConfig()
  }, [])

  // Track changes
  useEffect(() => {
    // If config failed to load, allow saving if any provider is enabled or has an API key
    if (!config) {
      const hasAnyProviderConfig = Object.values(providers).some(
        (p) => p.enabled || p.apiKey !== ''
      )
      setHasChanges(hasAnyProviderConfig || embeddingSettings.enabled)
      return
    }

    const providerMap = providersArrayToMap(config.providers)
    const hasProviderChanges = Object.entries(providers).some(([key, value]) => {
      const provider = key as AIProvider
      const configProvider = providerMap[provider]
      return (
        value.enabled !== (configProvider?.enabled ?? false) ||
        value.apiKey !== '' ||
        value.apiEndpoint !== (configProvider?.api_endpoint || '')
      )
    })

    // Check chunking changes
    const chunkingChanged = (() => {
      const current = embeddingSettings.chunking
      const original = config.embedding_settings?.chunking
      if (!current && !original) return false
      if (!current || !original) return true
      return (
        current.chunk_size !== original.chunk_size ||
        current.splitter !== original.splitter ||
        current.overlap.type !== original.overlap.type ||
        current.overlap.value !== original.overlap.value
      )
    })()

    // Check HNSW params changes
    const hnswChanged = (() => {
      const current = embeddingSettings.hnsw_params
      const original = config.embedding_settings?.hnsw_params
      if (!current && !original) return false
      if (!current || !original) return true
      return (
        current.connectivity !== original.connectivity ||
        current.expansion_add !== original.expansion_add ||
        current.expansion_search !== original.expansion_search
      )
    })()

    const hasEmbeddingChanges =
      embeddingSettings.enabled !== (config.embedding_settings?.enabled ?? false) ||
      embeddingSettings.ai_provider_ref !== config.embedding_settings?.ai_provider_ref ||
      embeddingSettings.ai_model_ref !== config.embedding_settings?.ai_model_ref ||
      embeddingSettings.include_name !== (config.embedding_settings?.include_name ?? true) ||
      embeddingSettings.include_path !== (config.embedding_settings?.include_path ?? true) ||
      embeddingSettings.max_embeddings_per_repo !== config.embedding_settings?.max_embeddings_per_repo ||
      embeddingSettings.dimensions !== (config.embedding_settings?.dimensions ?? 1536) ||
      embeddingSettings.default_max_distance !== config.embedding_settings?.default_max_distance ||
      embeddingSettings.distance_metric !== config.embedding_settings?.distance_metric ||
      embeddingSettings.quantization !== config.embedding_settings?.quantization ||
      chunkingChanged ||
      hnswChanged

    // Check local models changes (enabled by default if no config)
    const localConfig = providerMap['local']
    const originalLocalEnabled = localConfig ? localConfig.enabled : true
    const hasLocalChanges = localModelsEnabled !== originalLocalEnabled

    setHasChanges(hasProviderChanges || hasEmbeddingChanges || hasLocalChanges)
  }, [config, providers, embeddingSettings, localModelsEnabled])

  const loadConfig = async () => {
    try {
      setLoading(true)
      const data = await aiApi.getConfig(TENANT_ID)
      setConfig(data)

      // Initialize provider states from response array
      const providerMap = providersArrayToMap(data.providers)
      const newProviders = { ...providers }
      for (const key of Object.keys(newProviders) as AIProvider[]) {
        const providerConfig = providerMap[key]
        newProviders[key] = {
          enabled: providerConfig?.enabled ?? false,
          apiKey: '',
          apiEndpoint: providerConfig?.api_endpoint || '',
          testing: false,
          refreshing: false,
        }
      }
      setProviders(newProviders)

      // Initialize embedding settings
      if (data.embedding_settings) {
        setEmbeddingSettings(data.embedding_settings)
      }

      // Initialize local models state
      // Local models are enabled by default unless explicitly disabled in config
      const localConfig = providerMap['local']
      setLocalModelsEnabled(localConfig ? localConfig.enabled : true)
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

      const providerMap: Record<AIProvider, ProviderConfigResponse | undefined> = config
        ? providersArrayToMap(config.providers)
        : { openai: undefined, anthropic: undefined, google: undefined, azure_openai: undefined, ollama: undefined, groq: undefined, openrouter: undefined, bedrock: undefined, local: undefined, custom: undefined }

      // Build providers array for request
      const providerRequests: ProviderConfigRequest[] = (Object.keys(providers) as AIProvider[])
        .filter((provider) => provider !== 'local') // Handle local separately
        .map((provider) => {
          const state = providers[provider]
          const existingConfig = providerMap[provider]
          return {
            provider,
            enabled: state.enabled,
            api_key_plain: state.apiKey || undefined,
            api_endpoint: state.apiEndpoint || undefined,
            models: existingConfig?.models || [],
          }
        })

      // Add local provider config (only if explicitly disabled, since enabled is default)
      // We only need to save it if user wants to disable local models
      if (!localModelsEnabled) {
        providerRequests.push({
          provider: 'local',
          enabled: false,
          api_key_plain: undefined,
          api_endpoint: undefined,
          models: [],
        })
      } else {
        // If local is enabled, include it with enabled: true
        providerRequests.push({
          provider: 'local',
          enabled: true,
          api_key_plain: undefined,
          api_endpoint: undefined,
          models: [],
        })
      }

      const request: UpdateAIConfigRequest = {
        providers: providerRequests,
        embedding_settings: embeddingSettings,
      }

      await aiApi.updateConfig(TENANT_ID, request)

      // Reload config to get updated state
      await loadConfig()

      setHasChanges(false)
      toast.success('Configuration Saved', 'AI settings have been updated successfully')
    } catch (error) {
      console.error('Failed to save config:', error)
      toast.error('Failed to save configuration', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setSaving(false)
    }
  }

  const handleCancel = () => {
    if (!config) return
    loadConfig()
    setHasChanges(false)
  }

  const handleProviderToggle = (provider: AIProvider, enabled: boolean) => {
    setProviders((prev) => ({
      ...prev,
      [provider]: { ...prev[provider], enabled },
    }))
  }

  const handleApiKeyChange = (provider: AIProvider, key: string) => {
    setProviders((prev) => ({
      ...prev,
      [provider]: { ...prev[provider], apiKey: key },
    }))
  }

  const handleEndpointChange = (provider: AIProvider, endpoint: string) => {
    setProviders((prev) => ({
      ...prev,
      [provider]: { ...prev[provider], apiEndpoint: endpoint },
    }))
  }

  const handleTest = async (provider: AIProvider) => {
    try {
      setProviders((prev) => ({
        ...prev,
        [provider]: { ...prev[provider], testing: true, testResult: undefined },
      }))

      const result = await aiApi.testProvider(TENANT_ID, provider)

      setProviders((prev) => ({
        ...prev,
        [provider]: {
          ...prev[provider],
          testing: false,
          testResult: { success: result.success, error: result.error },
        },
      }))

      if (result.success) {
        toast.success('Connection Successful', `Connected to ${provider} successfully`)
        // Refresh config to get updated models
        await loadConfig()
      } else {
        toast.error('Connection Failed', result.error || 'Unknown error')
      }
    } catch (error) {
      console.error('Failed to test connection:', error)
      const errorMessage = error instanceof ApiError ? error.message : 'Unknown error'
      setProviders((prev) => ({
        ...prev,
        [provider]: {
          ...prev[provider],
          testing: false,
          testResult: { success: false, error: errorMessage },
        },
      }))
      toast.error('Connection Test Failed', errorMessage)
    }
  }

  const handleRefreshModels = async (provider: AIProvider) => {
    try {
      setProviders((prev) => ({
        ...prev,
        [provider]: { ...prev[provider], refreshing: true },
      }))

      // Fetch models from provider API with refresh=true
      await aiApi.getAvailableModels(TENANT_ID, { provider, refresh: true })
      await loadConfig()

      setProviders((prev) => ({
        ...prev,
        [provider]: { ...prev[provider], refreshing: false },
      }))

      toast.success('Models Refreshed', `Successfully refreshed models for ${provider}`)
    } catch (error) {
      console.error('Failed to refresh models:', error)
      setProviders((prev) => ({
        ...prev,
        [provider]: { ...prev[provider], refreshing: false },
      }))
      toast.error('Failed to refresh models', error instanceof ApiError ? error.message : 'Unknown error')
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="w-8 h-8 text-purple-400 animate-spin" />
      </div>
    )
  }

  // Get provider map for rendering
  const providerMap: Record<AIProvider, ProviderConfigResponse | undefined> = config
    ? providersArrayToMap(config.providers)
    : { openai: undefined, anthropic: undefined, google: undefined, azure_openai: undefined, ollama: undefined, groq: undefined, openrouter: undefined, bedrock: undefined, local: undefined, custom: undefined }

  return (
    <div className="space-y-6">
      <ToastContainer toasts={toast.toasts} onClose={toast.closeToast} />

      {/* Header */}
      <div className="flex items-start justify-between">
        <div className="flex items-center gap-3">
          <div className="p-3 bg-purple-500/20 border border-purple-500/30 rounded-xl">
            <Sparkles className="w-6 h-6 text-purple-400" />
          </div>
          <div>
            <h1 className="text-3xl font-bold text-white">AI Configuration</h1>
            <p className="text-gray-400">Configure AI providers and models for tenant: {TENANT_ID}</p>
          </div>
        </div>
      </div>

      {/* Local AI Models - first because it requires no config */}
      <LocalModelsSection
        enabled={localModelsEnabled}
        onToggle={setLocalModelsEnabled}
      />

      {/* Cloud Providers */}
      <div className="space-y-4">
        <h2 className="text-xl font-bold text-white flex items-center gap-2">
          <Settings className="w-5 h-5 text-purple-400" />
          Cloud AI Providers
        </h2>

        {/* OpenAI */}
        <ProviderSection
          icon={<Zap className="w-5 h-5 text-green-400" />}
          name="OpenAI"
          description="Industry-leading models for chat and embeddings"
          enabled={providers.openai.enabled}
          apiKeyConfigured={providerMap.openai?.has_api_key || false}
          apiEndpoint={providers.openai.apiEndpoint}
          models={providerMap.openai?.models || []}
          onToggle={(enabled) => handleProviderToggle('openai', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('openai', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('openai', endpoint)}
          onTest={() => handleTest('openai')}
          onRefreshModels={() => handleRefreshModels('openai')}
          testing={providers.openai.testing}
          refreshing={providers.openai.refreshing}
          testResult={providers.openai.testResult}
        />

        {/* Anthropic */}
        <ProviderSection
          icon={<Sparkles className="w-5 h-5 text-orange-400" />}
          name="Anthropic"
          description="Claude models for advanced reasoning and analysis"
          enabled={providers.anthropic.enabled}
          apiKeyConfigured={providerMap.anthropic?.has_api_key || false}
          apiEndpoint={providers.anthropic.apiEndpoint}
          models={providerMap.anthropic?.models || []}
          onToggle={(enabled) => handleProviderToggle('anthropic', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('anthropic', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('anthropic', endpoint)}
          onTest={() => handleTest('anthropic')}
          onRefreshModels={() => handleRefreshModels('anthropic')}
          testing={providers.anthropic.testing}
          refreshing={providers.anthropic.refreshing}
          testResult={providers.anthropic.testResult}
        />

        {/* Ollama */}
        <ProviderSection
          icon={<Server className="w-5 h-5 text-blue-400" />}
          name="Ollama"
          description="Self-hosted open source models"
          enabled={providers.ollama.enabled}
          apiKeyConfigured={providerMap.ollama?.has_api_key || false}
          apiEndpoint={providers.ollama.apiEndpoint}
          models={providerMap.ollama?.models || []}
          onToggle={(enabled) => handleProviderToggle('ollama', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('ollama', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('ollama', endpoint)}
          onTest={() => handleTest('ollama')}
          onRefreshModels={() => handleRefreshModels('ollama')}
          testing={providers.ollama.testing}
          refreshing={providers.ollama.refreshing}
          testResult={providers.ollama.testResult}
        />

        {/* Google Gemini */}
        <ProviderSection
          icon={<Globe className="w-5 h-5 text-blue-400" />}
          name="Google Gemini"
          description="Google's Gemini models for multimodal AI tasks"
          enabled={providers.google.enabled}
          apiKeyConfigured={providerMap.google?.has_api_key || false}
          apiEndpoint={providers.google.apiEndpoint}
          models={providerMap.google?.models || []}
          onToggle={(enabled) => handleProviderToggle('google', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('google', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('google', endpoint)}
          onTest={() => handleTest('google')}
          onRefreshModels={() => handleRefreshModels('google')}
          testing={providers.google.testing}
          refreshing={providers.google.refreshing}
          testResult={providers.google.testResult}
        />

        {/* Azure OpenAI */}
        <ProviderSection
          icon={<Cloud className="w-5 h-5 text-cyan-400" />}
          name="Azure OpenAI"
          description="Enterprise Azure-hosted OpenAI models"
          enabled={providers.azure_openai.enabled}
          apiKeyConfigured={providerMap.azure_openai?.has_api_key || false}
          apiEndpoint={providers.azure_openai.apiEndpoint}
          models={providerMap.azure_openai?.models || []}
          onToggle={(enabled) => handleProviderToggle('azure_openai', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('azure_openai', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('azure_openai', endpoint)}
          onTest={() => handleTest('azure_openai')}
          onRefreshModels={() => handleRefreshModels('azure_openai')}
          testing={providers.azure_openai.testing}
          refreshing={providers.azure_openai.refreshing}
          testResult={providers.azure_openai.testResult}
        />

        {/* Groq */}
        <ProviderSection
          icon={<Zap className="w-5 h-5 text-yellow-400" />}
          name="Groq"
          description="Lightning-fast inference with open source models"
          enabled={providers.groq.enabled}
          apiKeyConfigured={providerMap.groq?.has_api_key || false}
          apiEndpoint={providers.groq.apiEndpoint}
          models={providerMap.groq?.models || []}
          onToggle={(enabled) => handleProviderToggle('groq', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('groq', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('groq', endpoint)}
          onTest={() => handleTest('groq')}
          onRefreshModels={() => handleRefreshModels('groq')}
          testing={providers.groq.testing}
          refreshing={providers.groq.refreshing}
          testResult={providers.groq.testResult}
        />

        {/* OpenRouter */}
        <ProviderSection
          icon={<Globe className="w-5 h-5 text-purple-400" />}
          name="OpenRouter"
          description="Universal access to multiple AI model providers"
          enabled={providers.openrouter.enabled}
          apiKeyConfigured={providerMap.openrouter?.has_api_key || false}
          apiEndpoint={providers.openrouter.apiEndpoint}
          models={providerMap.openrouter?.models || []}
          onToggle={(enabled) => handleProviderToggle('openrouter', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('openrouter', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('openrouter', endpoint)}
          onTest={() => handleTest('openrouter')}
          onRefreshModels={() => handleRefreshModels('openrouter')}
          testing={providers.openrouter.testing}
          refreshing={providers.openrouter.refreshing}
          testResult={providers.openrouter.testResult}
        />

        {/* AWS Bedrock */}
        <ProviderSection
          icon={<Cloud className="w-5 h-5 text-orange-400" />}
          name="AWS Bedrock"
          description="AWS-managed foundation models (Claude, Titan, etc.)"
          enabled={providers.bedrock.enabled}
          apiKeyConfigured={providerMap.bedrock?.has_api_key || false}
          apiEndpoint={providers.bedrock.apiEndpoint}
          models={providerMap.bedrock?.models || []}
          onToggle={(enabled) => handleProviderToggle('bedrock', enabled)}
          onApiKeyChange={(key) => handleApiKeyChange('bedrock', key)}
          onEndpointChange={(endpoint) => handleEndpointChange('bedrock', endpoint)}
          onTest={() => handleTest('bedrock')}
          onRefreshModels={() => handleRefreshModels('bedrock')}
          testing={providers.bedrock.testing}
          refreshing={providers.bedrock.refreshing}
          testResult={providers.bedrock.testResult}
          providerType="bedrock"
        />
      </div>

      {/* Embedding Settings */}
      <GlassCard>
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h2 className="text-xl font-bold text-white flex items-center gap-2">
              <Sparkles className="w-5 h-5 text-purple-400" />
              Embedding Settings
            </h2>
            <label className="flex items-center gap-3 cursor-pointer">
              <span className="text-white font-medium text-sm">Enable Embeddings</span>
              <div className="relative">
                <input
                  type="checkbox"
                  checked={embeddingSettings.enabled}
                  onChange={(e) => setEmbeddingSettings({ ...embeddingSettings, enabled: e.target.checked })}
                  className="sr-only peer"
                />
                <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
                <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
              </div>
            </label>
          </div>
          <p className="text-gray-400 text-sm">
            Configure how embeddings are generated for semantic search and similarity features.
          </p>

          {embeddingSettings.enabled && (
            <div className="space-y-4 pt-4 border-t border-white/10">
              {/* Provider Selection */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Embedding Provider
                </label>
                <select
                  value={embeddingSettings.ai_provider_ref || ''}
                  onChange={(e) => {
                    const provider = e.target.value as AIProvider | ''
                    // Clear model selection when provider changes - user must select from available models
                    setEmbeddingSettings({
                      ...embeddingSettings,
                      ai_provider_ref: provider || undefined,
                      ai_model_ref: undefined,
                    })
                  }}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                >
                  <option value="" className="bg-gray-900">Select a provider...</option>
                  {EMBEDDING_CAPABLE_PROVIDERS.map((provider) => {
                    const providerConfig = providerMap[provider]
                    const isConfigured = providerConfig?.enabled && providerConfig?.has_api_key
                    return (
                      <option
                        key={provider}
                        value={provider}
                        className="bg-gray-900"
                        disabled={!isConfigured}
                      >
                        {provider.charAt(0).toUpperCase() + provider.slice(1)}
                        {!isConfigured && ' (not configured)'}
                      </option>
                    )
                  })}
                </select>
                <p className="text-sm text-gray-400 mt-1">
                  Select which AI provider to use for generating embeddings. Configure providers above first.
                </p>
              </div>

              {/* Model Selection (only show if provider is selected) */}
              {embeddingSettings.ai_provider_ref && (
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2">
                    Embedding Model
                  </label>
                  <select
                    value={embeddingSettings.ai_model_ref || ''}
                    onChange={(e) => {
                      const modelId = e.target.value || undefined
                      // Find the selected model to get its embedding_length
                      const selectedModel = embeddingSettings.ai_provider_ref
                        ? providerMap[embeddingSettings.ai_provider_ref]?.models
                            .find(m => m.model_id === modelId)
                        : undefined
                      const detectedDimensions = selectedModel?.metadata?.embedding_length as number | undefined

                      setEmbeddingSettings({
                        ...embeddingSettings,
                        ai_model_ref: modelId,
                        // Auto-set dimensions if model has embedding_length in metadata
                        ...(detectedDimensions ? { dimensions: detectedDimensions } : {}),
                      })
                    }}
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                  >
                    <option value="" className="bg-gray-900">Select a model...</option>
                    {providerMap[embeddingSettings.ai_provider_ref]?.models
                      .filter(m => m.use_cases.includes('embedding'))
                      .map(model => (
                        <option key={model.model_id} value={model.model_id} className="bg-gray-900">
                          {model.display_name || model.model_id}
                          {model.metadata?.embedding_length && ` (${model.metadata.embedding_length}d)`}
                        </option>
                      ))
                    }
                    {(!providerMap[embeddingSettings.ai_provider_ref]?.models?.length ||
                      !providerMap[embeddingSettings.ai_provider_ref]?.models.some(m => m.use_cases.includes('embedding'))) && (
                      <option value="" disabled className="bg-gray-900">No embedding models - click Refresh Models above</option>
                    )}
                  </select>
                  <p className="text-sm text-gray-400 mt-1">
                    The embedding model determines vector dimensions and quality. Refresh models on the provider above to see available options.
                  </p>
                </div>
              )}

              {/* Content Options */}
              <div className="grid grid-cols-2 gap-4">
                <label className="flex items-center gap-3 cursor-pointer p-3 bg-white/5 border border-white/10 rounded-lg hover:border-white/20 transition-colors">
                  <input
                    type="checkbox"
                    checked={embeddingSettings.include_name}
                    onChange={(e) => setEmbeddingSettings({ ...embeddingSettings, include_name: e.target.checked })}
                    className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500 focus:ring-purple-400"
                  />
                  <div>
                    <span className="text-white font-medium text-sm">Include Node Name</span>
                    <p className="text-gray-400 text-xs mt-0.5">Add node names to embedding content</p>
                  </div>
                </label>
                <label className="flex items-center gap-3 cursor-pointer p-3 bg-white/5 border border-white/10 rounded-lg hover:border-white/20 transition-colors">
                  <input
                    type="checkbox"
                    checked={embeddingSettings.include_path}
                    onChange={(e) => setEmbeddingSettings({ ...embeddingSettings, include_path: e.target.checked })}
                    className="w-4 h-4 rounded border-white/20 bg-white/5 text-purple-500 focus:ring-purple-400"
                  />
                  <div>
                    <span className="text-white font-medium text-sm">Include Node Path</span>
                    <p className="text-gray-400 text-xs mt-0.5">Add node paths to embedding content</p>
                  </div>
                </label>
              </div>

              {/* Dimensions */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Vector Dimensions
                </label>
                {(() => {
                  // Find selected model's embedding_length
                  const selectedModel = embeddingSettings.ai_model_ref && embeddingSettings.ai_provider_ref
                    ? providerMap[embeddingSettings.ai_provider_ref]?.models
                        .find(m => m.model_id === embeddingSettings.ai_model_ref)
                    : undefined
                  const detectedDims = selectedModel?.metadata?.embedding_length as number | undefined

                  if (detectedDims) {
                    // Show detected dimensions (read-only display)
                    return (
                      <div className="px-4 py-2 bg-green-500/10 border border-green-500/30 rounded-lg text-green-300 flex items-center gap-2">
                        <CheckCircle className="w-4 h-4" />
                        <span className="font-medium">{detectedDims}</span>
                        <span className="text-green-400/70">(auto-detected from model)</span>
                      </div>
                    )
                  }

                  // Fallback to dropdown for models without embedding_length
                  return (
                    <select
                      value={embeddingSettings.dimensions}
                      onChange={(e) => setEmbeddingSettings({ ...embeddingSettings, dimensions: parseInt(e.target.value) })}
                      className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    >
                      <option value="384" className="bg-gray-900">384 (small)</option>
                      <option value="768" className="bg-gray-900">768 (medium)</option>
                      <option value="1024" className="bg-gray-900">1024 (large)</option>
                      <option value="1536" className="bg-gray-900">1536 (OpenAI default)</option>
                      <option value="3072" className="bg-gray-900">3072 (OpenAI large)</option>
                    </select>
                  )
                })()}
                <p className="text-sm text-gray-400 mt-1">
                  {embeddingSettings.ai_model_ref && embeddingSettings.ai_provider_ref &&
                   providerMap[embeddingSettings.ai_provider_ref]?.models
                     .find(m => m.model_id === embeddingSettings.ai_model_ref)?.metadata?.embedding_length
                    ? "Dimensions auto-detected from selected model."
                    : "Vector size for embeddings. Select an embedding model to auto-detect."}
                </p>
              </div>

              {/* Max Embeddings Per Repo */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Max Embeddings Per Repository (Optional)
                </label>
                <input
                  type="number"
                  value={embeddingSettings.max_embeddings_per_repo || ''}
                  onChange={(e) => setEmbeddingSettings({
                    ...embeddingSettings,
                    max_embeddings_per_repo: e.target.value ? parseInt(e.target.value) : undefined
                  })}
                  placeholder="Unlimited"
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                />
                <p className="text-sm text-gray-400 mt-1">
                  Limit the number of embeddings per repository to control costs. Leave empty for unlimited.
                </p>
              </div>

              {/* Distance Metric */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Distance Metric
                </label>
                <select
                  value={embeddingSettings.distance_metric || 'Cosine'}
                  onChange={(e) => setEmbeddingSettings({
                    ...embeddingSettings,
                    distance_metric: e.target.value as DistanceMetric,
                    // Reset default_max_distance when metric changes
                    default_max_distance: e.target.value === 'Cosine' ? 0.6 : embeddingSettings.default_max_distance,
                  })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                >
                  <option value="Cosine" className="bg-gray-900">Cosine (default)</option>
                  <option value="L2" className="bg-gray-900">L2 (Euclidean)</option>
                  <option value="InnerProduct" className="bg-gray-900">Inner Product</option>
                  <option value="Hamming" className="bg-gray-900">Hamming</option>
                </select>
                <p className="text-sm text-gray-400 mt-1">
                  Distance function used for vector similarity comparisons.
                </p>
              </div>

              {/* Default Max Distance */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Default Max Distance: <span className="text-purple-400 font-mono">{(embeddingSettings.default_max_distance ?? 0.6).toFixed(2)}</span>
                </label>
                <input
                  type="range"
                  min="0.1"
                  max="1.0"
                  step="0.05"
                  value={embeddingSettings.default_max_distance ?? 0.6}
                  onChange={(e) => setEmbeddingSettings({
                    ...embeddingSettings,
                    default_max_distance: parseFloat(e.target.value),
                  })}
                  className="w-full h-2 bg-white/10 rounded-lg appearance-none cursor-pointer accent-purple-500"
                />
                <div className="flex justify-between text-xs text-gray-500 mt-1">
                  <span>0.1 (strict)</span>
                  <span>1.0 (permissive)</span>
                </div>
                <p className="text-sm text-gray-400 mt-1">
                  Default distance threshold for vector search results. Lower values return more similar results only.
                </p>
              </div>

              {/* Quantization */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Vector Quantization
                </label>
                <select
                  value={embeddingSettings.quantization || 'F32'}
                  onChange={(e) => setEmbeddingSettings({
                    ...embeddingSettings,
                    quantization: e.target.value as QuantizationType,
                  })}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                >
                  <option value="F32" className="bg-gray-900">F32 (full precision, default)</option>
                  <option value="F16" className="bg-gray-900">F16 (half precision, 50% memory)</option>
                  <option value="Int8" className="bg-gray-900">Int8 (quantized, 25% memory)</option>
                </select>
                <p className="text-sm text-gray-400 mt-1">
                  Lower precision reduces memory usage at the cost of some accuracy.
                </p>
              </div>

              {/* Advanced Index Parameters (HNSW) */}
              <div className="pt-4 border-t border-white/10">
                <button
                  type="button"
                  onClick={() => setShowAdvancedHnsw(!showAdvancedHnsw)}
                  className="flex items-center gap-2 text-sm text-gray-400 hover:text-white transition-colors mb-4"
                >
                  <SlidersHorizontal className="w-4 h-4" />
                  Advanced Index Parameters (HNSW)
                  {showAdvancedHnsw ? (
                    <ChevronDown className="w-4 h-4" />
                  ) : (
                    <ChevronRight className="w-4 h-4" />
                  )}
                </button>

                {showAdvancedHnsw && (
                  <div className="space-y-4 p-4 bg-white/5 border border-white/10 rounded-lg">
                    <div className="flex items-start gap-3 mb-4 p-3 bg-yellow-500/10 border border-yellow-500/30 rounded-lg">
                      <AlertCircle className="w-5 h-5 text-yellow-400 flex-shrink-0 mt-0.5" />
                      <p className="text-yellow-300 text-sm">
                        These parameters control the HNSW index structure. Set to 0 for automatic tuning. Only change these if you understand their impact on search quality and performance.
                      </p>
                    </div>

                    <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
                      {/* Connectivity (M) */}
                      <div>
                        <label className="block text-sm font-medium text-gray-300 mb-2">
                          Connectivity (M)
                        </label>
                        <input
                          type="number"
                          min="0"
                          max="128"
                          value={embeddingSettings.hnsw_params?.connectivity ?? 0}
                          onChange={(e) => setEmbeddingSettings({
                            ...embeddingSettings,
                            hnsw_params: {
                              ...(embeddingSettings.hnsw_params || DEFAULT_HNSW_PARAMS),
                              connectivity: parseInt(e.target.value) || 0,
                            },
                          })}
                          placeholder="0 (auto)"
                          className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                        />
                        <p className="text-xs text-gray-500 mt-1">
                          Max edges per node. 0 = auto.
                        </p>
                      </div>

                      {/* Expansion Add (ef_construction) */}
                      <div>
                        <label className="block text-sm font-medium text-gray-300 mb-2">
                          Build Expansion (ef)
                        </label>
                        <input
                          type="number"
                          min="0"
                          max="1000"
                          value={embeddingSettings.hnsw_params?.expansion_add ?? 0}
                          onChange={(e) => setEmbeddingSettings({
                            ...embeddingSettings,
                            hnsw_params: {
                              ...(embeddingSettings.hnsw_params || DEFAULT_HNSW_PARAMS),
                              expansion_add: parseInt(e.target.value) || 0,
                            },
                          })}
                          placeholder="0 (auto)"
                          className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                        />
                        <p className="text-xs text-gray-500 mt-1">
                          Index build quality. 0 = auto.
                        </p>
                      </div>

                      {/* Expansion Search (ef_search) */}
                      <div>
                        <label className="block text-sm font-medium text-gray-300 mb-2">
                          Search Expansion (ef)
                        </label>
                        <input
                          type="number"
                          min="0"
                          max="1000"
                          value={embeddingSettings.hnsw_params?.expansion_search ?? 0}
                          onChange={(e) => setEmbeddingSettings({
                            ...embeddingSettings,
                            hnsw_params: {
                              ...(embeddingSettings.hnsw_params || DEFAULT_HNSW_PARAMS),
                              expansion_search: parseInt(e.target.value) || 0,
                            },
                          })}
                          placeholder="0 (auto)"
                          className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                        />
                        <p className="text-xs text-gray-500 mt-1">
                          Search accuracy. 0 = auto.
                        </p>
                      </div>
                    </div>
                  </div>
                )}
              </div>

              {/* Chunking Configuration */}
              <div className="pt-4 border-t border-white/10">
                <div className="flex items-center justify-between mb-4">
                  <h3 className="text-lg font-medium text-white flex items-center gap-2">
                    <Layers className="w-5 h-5 text-purple-400" />
                    Chunking Configuration
                  </h3>
                  <label className="flex items-center gap-3 cursor-pointer">
                    <span className="text-white font-medium text-sm">Enable Chunking</span>
                    <div className="relative">
                      <input
                        type="checkbox"
                        checked={!!embeddingSettings.chunking}
                        onChange={(e) => setEmbeddingSettings({
                          ...embeddingSettings,
                          chunking: e.target.checked ? DEFAULT_CHUNKING_SETTINGS : undefined
                        })}
                        className="sr-only peer"
                      />
                      <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
                      <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
                    </div>
                  </label>
                </div>
                <p className="text-gray-400 text-sm mb-4">
                  Split long text into smaller chunks for better embedding quality and retrieval accuracy.
                </p>

                {embeddingSettings.chunking && (
                  <div className="space-y-4 p-4 bg-white/5 border border-white/10 rounded-lg">
                    {/* Chunk Size */}
                    <div>
                      <label className="block text-sm font-medium text-gray-300 mb-2">
                        Chunk Size (tokens)
                      </label>
                      <div className="flex items-center gap-4">
                        <input
                          type="range"
                          min="128"
                          max="512"
                          step="64"
                          value={embeddingSettings.chunking.chunk_size}
                          onChange={(e) => setEmbeddingSettings({
                            ...embeddingSettings,
                            chunking: {
                              ...embeddingSettings.chunking!,
                              chunk_size: parseInt(e.target.value)
                            }
                          })}
                          className="flex-1 h-2 bg-white/10 rounded-lg appearance-none cursor-pointer accent-purple-500"
                        />
                        <span className="text-white font-mono text-sm w-16 text-right">
                          {embeddingSettings.chunking.chunk_size}
                        </span>
                      </div>
                      <p className="text-sm text-gray-400 mt-1">
                        Target size for each text chunk. Smaller chunks = more granular search, larger chunks = more context.
                      </p>
                    </div>

                    {/* Overlap Type */}
                    <div>
                      <label className="block text-sm font-medium text-gray-300 mb-2">
                        Overlap Configuration
                      </label>
                      <div className="flex items-center gap-3">
                        <div className="flex rounded-lg overflow-hidden border border-white/20">
                          <button
                            type="button"
                            onClick={() => setEmbeddingSettings({
                              ...embeddingSettings,
                              chunking: {
                                ...embeddingSettings.chunking!,
                                overlap: { type: 'Tokens', value: 64 }
                              }
                            })}
                            className={`px-4 py-2 text-sm font-medium transition-colors ${
                              embeddingSettings.chunking.overlap.type === 'Tokens'
                                ? 'bg-purple-500 text-white'
                                : 'bg-white/5 text-gray-300 hover:bg-white/10'
                            }`}
                          >
                            Tokens
                          </button>
                          <button
                            type="button"
                            onClick={() => setEmbeddingSettings({
                              ...embeddingSettings,
                              chunking: {
                                ...embeddingSettings.chunking!,
                                overlap: { type: 'Percentage', value: 20 }
                              }
                            })}
                            className={`px-4 py-2 text-sm font-medium transition-colors ${
                              embeddingSettings.chunking.overlap.type === 'Percentage'
                                ? 'bg-purple-500 text-white'
                                : 'bg-white/5 text-gray-300 hover:bg-white/10'
                            }`}
                          >
                            Percentage
                          </button>
                        </div>
                        <input
                          type="number"
                          min={embeddingSettings.chunking.overlap.type === 'Percentage' ? 0 : 0}
                          max={embeddingSettings.chunking.overlap.type === 'Percentage' ? 50 : 256}
                          value={embeddingSettings.chunking.overlap.value}
                          onChange={(e) => setEmbeddingSettings({
                            ...embeddingSettings,
                            chunking: {
                              ...embeddingSettings.chunking!,
                              overlap: {
                                ...embeddingSettings.chunking!.overlap,
                                value: parseInt(e.target.value) || 0
                              }
                            }
                          })}
                          className="w-24 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-center focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                        />
                        <span className="text-gray-400 text-sm">
                          {embeddingSettings.chunking.overlap.type === 'Percentage' ? '%' : 'tokens'}
                        </span>
                      </div>
                      <p className="text-sm text-gray-400 mt-1">
                        Overlap between consecutive chunks helps preserve context at boundaries.
                      </p>
                    </div>

                    {/* Splitter Type */}
                    <div>
                      <label className="block text-sm font-medium text-gray-300 mb-2">
                        Splitter Strategy
                      </label>
                      <select
                        value={embeddingSettings.chunking.splitter}
                        onChange={(e) => setEmbeddingSettings({
                          ...embeddingSettings,
                          chunking: {
                            ...embeddingSettings.chunking!,
                            splitter: e.target.value as SplitterType
                          }
                        })}
                        className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                      >
                        <option value="recursive" className="bg-gray-900">Recursive (paragraphs → sentences → words)</option>
                        <option value="markdown" className="bg-gray-900">Markdown (respects headers and blocks)</option>
                        <option value="code" className="bg-gray-900">Code (respects function boundaries)</option>
                        <option value="fixed_size" className="bg-gray-900">Fixed Size (simple character split)</option>
                      </select>
                      <p className="text-sm text-gray-400 mt-1">
                        How text is split into chunks. Recursive works best for most content.
                      </p>
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      </GlassCard>

      {/* Info Card */}
      <GlassCard>
        <div className="flex items-start gap-3">
          <Info className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
          <div>
            <h3 className="text-white font-medium mb-1">Configuration Tips</h3>
            <ul className="text-sm text-gray-400 space-y-1">
              <li>Enable at least one provider to use AI features</li>
              <li>Test connections after configuring API keys to verify setup</li>
              <li>Refresh models to see the latest available options from each provider</li>
              <li>Custom endpoints allow using self-hosted or proxy services</li>
              <li>Enable embeddings and select a model for semantic search features</li>
            </ul>
          </div>
        </div>
      </GlassCard>

      {/* HuggingFace Models Section */}
      <GlassCard className="p-6">
        <HuggingFaceModelsSection
          tenantId={TENANT_ID}
          onError={(title, message) => toast.error(title, message)}
          onSuccess={(title, _message) => toast.success(title)}
        />
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
