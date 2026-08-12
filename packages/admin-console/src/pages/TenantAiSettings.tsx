import { useState, useEffect } from 'react'
import {
  Sparkles,
  Key,
  Eye,
  EyeOff,
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
  Plus,
  Trash2,
  Image as ImageIcon,
} from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { ToastContainer, useToast } from '../components/Toast'
import HuggingFaceModelsSection from '../components/HuggingFaceModelsSection'
import {
  aiApi,
  AIConfigResponse,
  AIProvider,
  AIModelConfig,
  ProviderConfigResponse,
  UpdateAIConfigRequest,
  EmbeddingSettings,
  SplitterType,
  DistanceMetric,
  QuantizationType,
  DEFAULT_CHUNKING_SETTINGS,
  DEFAULT_HNSW_PARAMS,
} from '../api/ai'
import {
  PROVIDER_KINDS,
  ProviderDraft,
  buildProviderRequests,
  draftsFromConfig,
  findProviderBySlug,
  isDraftDirty,
  isEmbeddingCapable,
  kindLabel,
  providerLabel,
  validateProviderSlug,
} from '../utils/aiProviders'
import { ApiError } from '../api/client'
import { useAuth } from '../contexts/AuthContext'

/**
 * Tenant AI settings.
 *
 * The page edits a LIST of provider entries keyed by slug, not a fixed row per
 * kind. Two entries can share a kind, so the kind can never be the state key: a
 * provisioned `marvel` gateway and a hand-made `custom` one would collapse into
 * one row, and saving would write one's endpoint under the other's name.
 *
 * Saving PUTs only the entries the operator touched, because the endpoint is a
 * merge — an omitted slug keeps exactly what is stored. Removal is a separate,
 * explicit DELETE.
 */

/** Icon for a provider KIND, used when an entry ships no `icon_url`. */
const KIND_ICONS: Record<AIProvider, React.ReactNode> = {
  openai: <Zap className="w-5 h-5 text-green-400" />,
  anthropic: <Sparkles className="w-5 h-5 text-orange-400" />,
  google: <Globe className="w-5 h-5 text-blue-400" />,
  azure_openai: <Cloud className="w-5 h-5 text-cyan-400" />,
  ollama: <Server className="w-5 h-5 text-blue-400" />,
  groq: <Zap className="w-5 h-5 text-yellow-400" />,
  openrouter: <Globe className="w-5 h-5 text-purple-400" />,
  bedrock: <Cloud className="w-5 h-5 text-orange-400" />,
  local: <Cpu className="w-5 h-5 text-cyan-400" />,
  custom: <Server className="w-5 h-5 text-purple-400" />,
}

/**
 * An entry's icon: the `icon_url` a provisioned gateway ships, falling back to
 * the kind glyph. A broken URL falls back too, rather than leaving a torn-image
 * box where the provider's identity should be.
 */
function ProviderIcon({ kind, iconUrl }: { kind: AIProvider; iconUrl?: string }) {
  const [failed, setFailed] = useState(false)
  useEffect(() => setFailed(false), [iconUrl])

  if (iconUrl && !failed) {
    return (
      <img
        src={iconUrl}
        alt=""
        className="w-5 h-5 rounded object-contain"
        onError={() => setFailed(true)}
      />
    )
  }
  return <>{KIND_ICONS[kind]}</>
}

interface ProviderEntryCardProps {
  draft: ProviderDraft
  /** What the server last returned for this slug; absent for a new entry. */
  original?: ProviderConfigResponse
  dirty: boolean
  onChange: (patch: Partial<ProviderDraft>) => void
  onRemove: () => void
  onTest: () => Promise<void>
  onRefreshModels: () => Promise<void>
}

function ProviderEntryCard({
  draft,
  original,
  dirty,
  onChange,
  onRemove,
  onTest,
  onRefreshModels,
}: ProviderEntryCardProps) {
  const [expanded, setExpanded] = useState(draft.isNew)
  const [showApiKey, setShowApiKey] = useState(false)

  // Bedrock takes two credentials rather than one key; they are stored joined
  // as "access_key_id:secret_access_key" in the same api_key field.
  const isBedrock = draft.kind === 'bedrock'
  const [bedrockAccessKey, setBedrockAccessKey] = useState('')
  const [bedrockSecretKey, setBedrockSecretKey] = useState('')

  const apiKeyConfigured = original?.has_api_key ?? false
  const models: AIModelConfig[] = original?.models ?? []

  const handleBedrockCredentialChange = (accessKey: string, secretKey: string) => {
    setBedrockAccessKey(accessKey)
    setBedrockSecretKey(secretKey)
    if (accessKey && secretKey) {
      onChange({ apiKey: `${accessKey}:${secretKey}` })
    }
  }

  return (
    <GlassCard>
      <div className="space-y-4">
        {/* Header */}
        <div className="flex items-center justify-between">
          <button
            onClick={() => setExpanded(!expanded)}
            className="flex items-center gap-3 flex-1 text-left group min-w-0"
          >
            <div
              className={`p-2 rounded-lg ${draft.enabled ? 'bg-purple-500/20 border border-purple-500/30' : 'bg-white/5 border border-white/10'}`}
            >
              <ProviderIcon kind={draft.kind} iconUrl={draft.iconUrl || undefined} />
            </div>
            <div className="flex-1 min-w-0">
              <h2 className="text-xl font-bold text-white flex items-center gap-2 truncate">
                {draft.displayName.trim() || draft.slug}
                {apiKeyConfigured && <CheckCircle className="w-4 h-4 text-green-400 flex-shrink-0" />}
                {dirty && (
                  <span className="px-2 py-0.5 text-xs rounded bg-yellow-500/20 border border-yellow-500/30 text-yellow-300 flex-shrink-0">
                    unsaved
                  </span>
                )}
              </h2>
              <p className="text-gray-400 text-sm flex items-center gap-2 truncate">
                {/* Slug and kind are both always shown: the slug is the identity an
                    agent node or a model id names, the kind is only the protocol. */}
                <span className="font-mono text-purple-300">{draft.slug}</span>
                <span className="text-gray-600">·</span>
                <span>{kindLabel(draft.kind)}</span>
                {draft.apiEndpoint && (
                  <>
                    <span className="text-gray-600">·</span>
                    <span className="truncate">{draft.apiEndpoint}</span>
                  </>
                )}
              </p>
            </div>
            {expanded ? (
              <ChevronDown className="w-5 h-5 text-gray-400 group-hover:text-white transition-colors flex-shrink-0" />
            ) : (
              <ChevronRight className="w-5 h-5 text-gray-400 group-hover:text-white transition-colors flex-shrink-0" />
            )}
          </button>

          <label className="flex items-center gap-3 cursor-pointer ml-4">
            <span className="text-white font-medium text-sm">Enabled</span>
            <div className="relative">
              <input
                type="checkbox"
                checked={draft.enabled}
                onChange={(e) => onChange({ enabled: e.target.checked })}
                className="sr-only peer"
              />
              <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
              <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
            </div>
          </label>

          <button
            onClick={onRemove}
            title={`Remove provider '${draft.slug}'`}
            aria-label={`Remove provider '${draft.slug}'`}
            className="ml-3 p-2 rounded-lg text-gray-400 hover:text-red-300 hover:bg-red-500/10 transition-colors"
          >
            <Trash2 className="w-4 h-4" />
          </button>
        </div>

        {/* Expanded Content */}
        {expanded && (
          <div className="space-y-4 pt-4 border-t border-white/10">
            {/* Identity: the slug is immutable and the kind is bound to it. */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">Slug</label>
                <input
                  type="text"
                  value={draft.slug}
                  readOnly
                  disabled
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-gray-400 font-mono cursor-not-allowed"
                />
                <p className="text-xs text-gray-500 mt-1">
                  Immutable. Model ids read <span className="font-mono">{draft.slug}:model</span>, and
                  agents reference this provider by it.
                </p>
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">Kind</label>
                <input
                  type="text"
                  value={kindLabel(draft.kind)}
                  readOnly
                  disabled
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-gray-400 cursor-not-allowed"
                />
                <p className="text-xs text-gray-500 mt-1">
                  Chosen at creation. To change protocol, remove this entry and add a new one.
                </p>
              </div>
            </div>

            {/* Presentation */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">Display Name</label>
                <input
                  type="text"
                  value={draft.displayName}
                  onChange={(e) => onChange({ displayName: e.target.value })}
                  placeholder={draft.slug}
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                  <ImageIcon className="w-4 h-4 text-purple-400" />
                  Icon URL
                </label>
                <input
                  type="text"
                  value={draft.iconUrl}
                  onChange={(e) => onChange({ iconUrl: e.target.value })}
                  placeholder="https://…/logo.svg"
                  className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                />
              </div>
            </div>

            {/* Credentials */}
            {isBedrock ? (
              <>
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
              </>
            ) : (
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                  <Key className="w-4 h-4 text-purple-400" />
                  API Key
                </label>
                {apiKeyConfigured && !draft.apiKey && (
                  <div className="flex items-center gap-2 px-3 py-2 bg-green-500/10 border border-green-500/30 rounded-lg mb-2">
                    <CheckCircle className="w-4 h-4 text-green-400" />
                    <span className="text-green-300 text-sm font-medium">API key configured</span>
                  </div>
                )}
                <div className="relative">
                  <input
                    type={showApiKey ? 'text' : 'password'}
                    value={draft.apiKey}
                    onChange={(e) => onChange({ apiKey: e.target.value })}
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
                <p className="text-xs text-gray-500 mt-1">
                  Leave blank to keep the stored key. It is never sent back to the browser.
                </p>
              </div>
            )}

            {/* Endpoint. For Bedrock this field carries the AWS region. */}
            <div>
              <label className="block text-sm font-medium text-gray-300 mb-2 flex items-center gap-2">
                <Globe className="w-4 h-4 text-purple-400" />
                {isBedrock ? 'AWS Region' : 'API Endpoint'}
              </label>
              <input
                type="text"
                value={draft.apiEndpoint}
                onChange={(e) => onChange({ apiEndpoint: e.target.value })}
                placeholder={isBedrock ? 'us-east-1' : `Default ${kindLabel(draft.kind)} endpoint`}
                className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
              />
            </div>

            {/* Test Connection */}
            <div className="flex items-center gap-3 flex-wrap">
              <button
                onClick={onTest}
                disabled={!draft.enabled || draft.isNew || draft.testing}
                title={draft.isNew ? 'Save this provider before testing it' : undefined}
                className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all flex items-center gap-2"
              >
                {draft.testing ? (
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
                disabled={!draft.enabled || draft.isNew || draft.refreshing}
                title={draft.isNew ? 'Save this provider before refreshing its models' : undefined}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 disabled:bg-white/5 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all flex items-center gap-2"
              >
                {draft.refreshing ? (
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

              {draft.testResult && (
                <div
                  className={`flex items-center gap-2 px-3 py-2 rounded-lg ${
                    draft.testResult.success
                      ? 'bg-green-500/10 border border-green-500/30'
                      : 'bg-red-500/10 border border-red-500/30'
                  }`}
                >
                  {draft.testResult.success ? (
                    <>
                      <CheckCircle className="w-4 h-4 text-green-400" />
                      <span className="text-green-300 text-sm">Connected</span>
                    </>
                  ) : (
                    <>
                      <XCircle className="w-4 h-4 text-red-400" />
                      <span className="text-red-300 text-sm">
                        {draft.testResult.error || 'Connection failed'}
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
                          {/* The addressable id, which this entry's slug prefixes. */}
                          <div className="text-xs text-gray-500 font-mono mt-0.5 truncate">
                            {draft.slug}:{model.model_id}
                          </div>
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

            {models.length === 0 && draft.enabled && !draft.isNew && (
              <div className="p-4 bg-yellow-500/10 border border-yellow-500/30 rounded-lg flex items-start gap-3">
                <AlertCircle className="w-5 h-5 text-yellow-400 flex-shrink-0 mt-0.5" />
                <div>
                  <p className="text-yellow-300 text-sm font-medium">No models available</p>
                  <p className="text-yellow-300/80 text-xs mt-1">
                    Configure the API key, save, then refresh the model list.
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

interface AddProviderFormProps {
  /** Slugs already in play — a slug is an address, so it has to be unique. */
  existingSlugs: string[]
  onAdd: (draft: ProviderDraft) => void
  onCancel: () => void
}

function AddProviderForm({ existingSlugs, onAdd, onCancel }: AddProviderFormProps) {
  const [slug, setSlug] = useState('')
  const [kind, setKind] = useState<AIProvider>('openai')
  const [displayName, setDisplayName] = useState('')
  const [touched, setTouched] = useState(false)

  const error = validateProviderSlug(slug, existingSlugs)

  const submit = () => {
    setTouched(true)
    if (error) return
    onAdd({
      slug,
      kind,
      displayName,
      iconUrl: '',
      apiEndpoint: '',
      enabled: true,
      apiKey: '',
      isNew: true,
      testing: false,
      refreshing: false,
    })
  }

  return (
    <GlassCard>
      <div className="space-y-4">
        <h3 className="text-lg font-bold text-white">Add Provider</h3>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">
              Slug <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              value={slug}
              autoFocus
              onChange={(e) => setSlug(e.target.value)}
              onBlur={() => setTouched(true)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') submit()
              }}
              placeholder="marvel"
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 font-mono focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
            />
            <p className="text-xs text-gray-500 mt-1">
              Permanent. Becomes the model-id prefix and the name agents use.
            </p>
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Kind</label>
            <select
              value={kind}
              onChange={(e) => setKind(e.target.value as AIProvider)}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
            >
              {PROVIDER_KINDS.map((k) => (
                <option key={k} value={k} className="bg-gray-900">
                  {kindLabel(k)}
                </option>
              ))}
            </select>
            <p className="text-xs text-gray-500 mt-1">Wire protocol. Cannot be changed later.</p>
          </div>
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Display Name</label>
            <input
              type="text"
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder="Optional"
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder:text-gray-500 focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
            />
          </div>
        </div>

        {touched && error && (
          <div className="flex items-center gap-2 px-3 py-2 bg-red-500/10 border border-red-500/30 rounded-lg">
            <XCircle className="w-4 h-4 text-red-400" />
            <span className="text-red-300 text-sm">{error}</span>
          </div>
        )}

        <div className="flex justify-end gap-3">
          <button
            onClick={onCancel}
            className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-all"
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={!!error}
            className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 disabled:text-gray-500 disabled:cursor-not-allowed text-white rounded-lg transition-all flex items-center gap-2"
          >
            <Plus className="w-4 h-4" />
            Add
          </button>
        </div>
      </div>
    </GlassCard>
  )
}

export default function TenantAiSettings() {
  const toast = useToast()
  // Tenant resolved from /api/admin/bootstrap (via AuthContext). Replaces
  // the previous module-level `TENANT_ID = 'default'` hardcode so the AI
  // settings target the correct tenant on multi-tenant deployments.
  const { tenantId: TENANT_ID } = useAuth()

  // State
  const [config, setConfig] = useState<AIConfigResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [hasChanges, setHasChanges] = useState(false)

  // The editable provider list: one draft per stored entry, in server order.
  // Never a fixed row per kind — that is what collapsed same-kind entries.
  const [drafts, setDrafts] = useState<ProviderDraft[]>([])
  const [showAddForm, setShowAddForm] = useState(false)
  const [pendingRemoval, setPendingRemoval] = useState<ProviderDraft | null>(null)
  const [removing, setRemoving] = useState(false)

  // Embedding settings
  const [embeddingSettings, setEmbeddingSettings] = useState<EmbeddingSettings>({
    enabled: false,
    include_name: true,
    include_path: true,
    dimensions: 1536,
  })
  const [showAdvancedHnsw, setShowAdvancedHnsw] = useState(false)

  // Load configuration on mount (and when tenant changes — bootstrap
  // settles before the first protected route renders, but re-loading on
  // tenant change keeps the panel honest if the value updates later).
  useEffect(() => {
    loadConfig()

  }, [TENANT_ID])

  const storedProviders = config?.providers ?? []

  // Track changes
  useEffect(() => {
    const hasProviderChanges = drafts.some((draft) =>
      isDraftDirty(draft, findProviderBySlug(config?.providers ?? [], draft.slug))
    )

    // Check chunking changes
    const chunkingChanged = (() => {
      const current = embeddingSettings.chunking
      const original = config?.embedding_settings?.chunking
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
      const original = config?.embedding_settings?.hnsw_params
      if (!current && !original) return false
      if (!current || !original) return true
      return (
        current.connectivity !== original.connectivity ||
        current.expansion_add !== original.expansion_add ||
        current.expansion_search !== original.expansion_search
      )
    })()

    const hasEmbeddingChanges =
      embeddingSettings.enabled !== (config?.embedding_settings?.enabled ?? false) ||
      embeddingSettings.ai_provider_ref !== config?.embedding_settings?.ai_provider_ref ||
      embeddingSettings.ai_model_ref !== config?.embedding_settings?.ai_model_ref ||
      embeddingSettings.include_name !== (config?.embedding_settings?.include_name ?? true) ||
      embeddingSettings.include_path !== (config?.embedding_settings?.include_path ?? true) ||
      embeddingSettings.max_embeddings_per_repo !== config?.embedding_settings?.max_embeddings_per_repo ||
      embeddingSettings.dimensions !== (config?.embedding_settings?.dimensions ?? 1536) ||
      embeddingSettings.default_max_distance !== config?.embedding_settings?.default_max_distance ||
      embeddingSettings.distance_metric !== config?.embedding_settings?.distance_metric ||
      embeddingSettings.quantization !== config?.embedding_settings?.quantization ||
      chunkingChanged ||
      hnswChanged

    setHasChanges(hasProviderChanges || hasEmbeddingChanges)
  }, [config, drafts, embeddingSettings])

  const loadConfig = async () => {
    try {
      setLoading(true)
      const data = await aiApi.getConfig(TENANT_ID)
      setConfig(data)
      setDrafts(draftsFromConfig(data.providers))

      if (data.embedding_settings) {
        setEmbeddingSettings(data.embedding_settings)
      }
    } catch (error) {
      console.error('Failed to load config:', error)
      toast.error('Failed to load configuration', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setLoading(false)
    }
  }

  const patchDraft = (slug: string, patch: Partial<ProviderDraft>) => {
    setDrafts((prev) => prev.map((d) => (d.slug === slug ? { ...d, ...patch } : d)))
  }

  const handleSave = async () => {
    try {
      setSaving(true)

      // Only the entries the operator touched. PUT is a merge, so everything
      // omitted keeps its stored value — including entries this console has
      // never displayed.
      const providers = buildProviderRequests(drafts, storedProviders)
      const request: UpdateAIConfigRequest = {
        providers,
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
    setShowAddForm(false)
    loadConfig()
    setHasChanges(false)
  }

  const handleAdd = (draft: ProviderDraft) => {
    setDrafts((prev) => [...prev, draft])
    setShowAddForm(false)
  }

  const requestRemoval = (draft: ProviderDraft) => {
    // An entry that was never saved has nothing on the server to delete.
    if (draft.isNew) {
      setDrafts((prev) => prev.filter((d) => d.slug !== draft.slug))
      return
    }
    setPendingRemoval(draft)
  }

  const confirmRemoval = async () => {
    if (!pendingRemoval) return
    const slug = pendingRemoval.slug
    try {
      setRemoving(true)
      // Removal is an explicit DELETE, never omission from the PUT payload —
      // the merge leaves omitted slugs alone by design.
      await aiApi.deleteProvider(TENANT_ID, slug)
      setPendingRemoval(null)
      await loadConfig()
      toast.success('Provider Removed', `'${slug}' is no longer configured`)
    } catch (error) {
      console.error('Failed to remove provider:', error)
      toast.error('Failed to remove provider', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setRemoving(false)
    }
  }

  const handleTest = async (slug: string) => {
    try {
      patchDraft(slug, { testing: true, testResult: undefined })

      const result = await aiApi.testProvider(TENANT_ID, slug)

      patchDraft(slug, {
        testing: false,
        testResult: { success: result.success, error: result.error },
      })

      if (result.success) {
        toast.success('Connection Successful', `Connected to '${slug}' successfully`)
        // Refresh config to get updated models
        await loadConfig()
      } else {
        toast.error('Connection Failed', result.error || 'Unknown error')
      }
    } catch (error) {
      console.error('Failed to test connection:', error)
      const errorMessage = error instanceof ApiError ? error.message : 'Unknown error'
      patchDraft(slug, { testing: false, testResult: { success: false, error: errorMessage } })
      toast.error('Connection Test Failed', errorMessage)
    }
  }

  const handleRefreshModels = async (slug: string) => {
    try {
      patchDraft(slug, { refreshing: true })

      // The `provider` query parameter is a SLUG: refreshing one of two
      // same-kind gateways must not go out to the other one's endpoint.
      await aiApi.getAvailableModels(TENANT_ID, { provider: slug, refresh: true })
      await loadConfig()

      patchDraft(slug, { refreshing: false })

      toast.success('Models Refreshed', `Successfully refreshed models for '${slug}'`)
    } catch (error) {
      console.error('Failed to refresh models:', error)
      patchDraft(slug, { refreshing: false })
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

  // Entries offered to the embedding picker, by slug. An entry qualifies on the
  // models it actually publishes, so a `custom` gateway serving an embedding
  // model is offered alongside OpenAI.
  const embeddingCandidates = storedProviders.filter(isEmbeddingCapable)
  const selectedEmbeddingProvider = findProviderBySlug(
    storedProviders,
    embeddingSettings.ai_provider_ref
  )
  const selectedEmbeddingModels = selectedEmbeddingProvider?.models ?? []

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

      {/* Providers */}
      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <h2 className="text-xl font-bold text-white flex items-center gap-2">
            <Layers className="w-5 h-5 text-purple-400" />
            AI Providers ({drafts.length})
          </h2>
          <button
            onClick={() => setShowAddForm(true)}
            disabled={showAddForm}
            className="px-4 py-2 bg-purple-500 hover:bg-purple-600 disabled:bg-white/10 disabled:text-gray-500 text-white rounded-lg transition-all flex items-center gap-2"
          >
            <Plus className="w-4 h-4" />
            Add Provider
          </button>
        </div>

        {showAddForm && (
          <AddProviderForm
            existingSlugs={drafts.map((d) => d.slug)}
            onAdd={handleAdd}
            onCancel={() => setShowAddForm(false)}
          />
        )}

        {drafts.length === 0 && !showAddForm && (
          <GlassCard>
            <div className="flex items-start gap-3">
              <Info className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
              <div>
                <p className="text-white font-medium">No providers configured</p>
                <p className="text-gray-400 text-sm mt-1">
                  Add one to start using AI features. Each provider gets a slug you choose —
                  agents and model ids reference that slug, so a tenant can run several
                  gateways of the same kind side by side.
                </p>
              </div>
            </div>
          </GlassCard>
        )}

        {drafts.map((draft) => {
          const original = findProviderBySlug(storedProviders, draft.slug)
          return (
            <ProviderEntryCard
              key={draft.slug}
              draft={draft}
              original={original}
              dirty={isDraftDirty(draft, original)}
              onChange={(patch) => patchDraft(draft.slug, patch)}
              onRemove={() => requestRemoval(draft)}
              onTest={() => handleTest(draft.slug)}
              onRefreshModels={() => handleRefreshModels(draft.slug)}
            />
          )
        })}

        <GlassCard>
          <div className="flex items-start gap-3">
            <Cpu className="w-5 h-5 text-cyan-400 flex-shrink-0 mt-0.5" />
            <div>
              <p className="text-cyan-300 text-sm font-medium mb-1">On-device models need no API key</p>
              <p className="text-cyan-300/80 text-xs">
                Add a provider of kind <span className="font-mono">Local (Candle)</span> to run
                Moondream (vision), BLIP (captions) and CLIP (embeddings) on your own server.
              </p>
            </div>
          </div>
        </GlassCard>
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
              {/* Provider Selection — by SLUG. `ai_provider_ref` names one
                  configured entry, not a kind. */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">
                  Embedding Provider
                </label>
                <div className="flex items-center gap-3">
                  {selectedEmbeddingProvider && (
                    <div className="p-2 rounded-lg bg-white/5 border border-white/10">
                      <ProviderIcon
                        kind={selectedEmbeddingProvider.provider}
                        iconUrl={selectedEmbeddingProvider.icon_url}
                      />
                    </div>
                  )}
                  <select
                    value={embeddingSettings.ai_provider_ref || ''}
                    onChange={(e) => {
                      const slug = e.target.value
                      // Clear the model selection: model ids are provider-scoped.
                      setEmbeddingSettings({
                        ...embeddingSettings,
                        ai_provider_ref: slug || undefined,
                        ai_model_ref: undefined,
                      })
                    }}
                    className="flex-1 px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                  >
                    <option value="" className="bg-gray-900">Select a provider...</option>
                    {embeddingCandidates.map((entry) => {
                      const isConfigured =
                        entry.enabled &&
                        (entry.has_api_key || entry.provider === 'local' || entry.provider === 'ollama')
                      return (
                        <option
                          key={entry.slug}
                          value={entry.slug}
                          className="bg-gray-900"
                          disabled={!isConfigured}
                        >
                          {providerLabel(entry)}
                          {!isConfigured && ' (not configured)'}
                        </option>
                      )
                    })}
                    {/* A stored slug that no longer resolves stays visible, or
                        saving this form would silently repoint embeddings. */}
                    {embeddingSettings.ai_provider_ref && !selectedEmbeddingProvider && (
                      <option value={embeddingSettings.ai_provider_ref} className="bg-gray-900">
                        {embeddingSettings.ai_provider_ref} (not configured)
                      </option>
                    )}
                  </select>
                </div>
                <p className="text-sm text-gray-400 mt-1">
                  Which configured provider generates embeddings. Providers are referenced by slug.
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
                      const selectedModel = selectedEmbeddingModels.find((m) => m.model_id === modelId)
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
                    {selectedEmbeddingModels
                      .filter((m) => m.use_cases.includes('embedding'))
                      .map((model) => (
                        <option key={model.model_id} value={model.model_id} className="bg-gray-900">
                          {model.display_name || model.model_id}
                          {model.metadata?.embedding_length ? ` (${model.metadata.embedding_length}d)` : ''}
                        </option>
                      ))}
                    {!selectedEmbeddingModels.some((m) => m.use_cases.includes('embedding')) && (
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
                  const selectedModel = embeddingSettings.ai_model_ref
                    ? selectedEmbeddingModels.find((m) => m.model_id === embeddingSettings.ai_model_ref)
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
                  {embeddingSettings.ai_model_ref &&
                   selectedEmbeddingModels.find((m) => m.model_id === embeddingSettings.ai_model_ref)?.metadata?.embedding_length
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
                          min={0}
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
              <li>Each provider has a permanent slug; models are addressed as <span className="font-mono">slug:model</span></li>
              <li>Several providers may share a kind — give each its own slug and display name</li>
              <li>Saving only sends the providers you edited; the rest are left untouched</li>
              <li>Removing a provider is immediate and breaks anything referencing its slug</li>
              <li>Enable embeddings and select a provider and model for semantic search features</li>
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

      <ConfirmDialog
        open={pendingRemoval !== null}
        variant="danger"
        title={`Remove provider '${pendingRemoval?.slug}'?`}
        message={
          `This deletes the entry and its stored API key immediately — it is not part of Save.\n\n` +
          `Anything naming this slug stops resolving: model ids '${pendingRemoval?.slug}:…', ` +
          `agent nodes with provider '${pendingRemoval?.slug}', and an embedding configuration ` +
          `pointing at it. Those failures surface at call time, not now.`
        }
        confirmText={removing ? 'Removing...' : 'Remove'}
        onConfirm={confirmRemoval}
        onCancel={() => setPendingRemoval(null)}
      />
    </div>
  )
}
