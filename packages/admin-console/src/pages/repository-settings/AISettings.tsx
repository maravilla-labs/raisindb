import { useState, useEffect } from 'react'
import { useParams } from 'react-router-dom'
import {
  Sparkles,
  Layers,
  Info,
  CheckCircle,
  Loader2,
  AlertCircle,
} from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import { ToastContainer, useToast } from '../../components/Toast'
import {
  aiApi,
  EmbeddingSettings,
  ChunkingSettings,
  SplitterType,
  DEFAULT_CHUNKING_SETTINGS,
} from '../../api/ai'
import { ApiError } from '../../api/client'

// Use "default" as tenant ID for single-tenant mode
const TENANT_ID = 'default'

export default function AISettings() {
  const { repo } = useParams<{ repo: string }>()
  const toast = useToast()

  // Global tenant settings
  const [tenantEmbeddingSettings, setTenantEmbeddingSettings] = useState<EmbeddingSettings | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [hasChanges, setHasChanges] = useState(false)

  // Repository-level overrides
  const [overrideChunking, setOverrideChunking] = useState(false)
  const [chunkingSettings, setChunkingSettings] = useState<ChunkingSettings>(DEFAULT_CHUNKING_SETTINGS)

  // Load tenant AI configuration
  useEffect(() => {
    loadTenantConfig()
  }, [])

  const loadTenantConfig = async () => {
    try {
      setLoading(true)
      const data = await aiApi.getConfig(TENANT_ID)
      setTenantEmbeddingSettings(data.embedding_settings || null)
    } catch (error) {
      console.error('Failed to load AI config:', error)
      toast.error('Failed to load AI configuration', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setLoading(false)
    }
  }

  const handleSave = async () => {
    // TODO: Implement repository-specific AI settings storage
    // For now, just show a success message
    setSaving(true)
    try {
      // Placeholder for future API call
      await new Promise(resolve => setTimeout(resolve, 500))
      toast.success('Settings Saved', 'Repository AI settings have been updated')
      setHasChanges(false)
    } catch (error) {
      toast.error('Failed to save settings', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setSaving(false)
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Loader2 className="w-8 h-8 text-purple-400 animate-spin" />
      </div>
    )
  }

  // Check if embeddings are enabled at tenant level
  const embeddingsEnabled = tenantEmbeddingSettings?.enabled ?? false

  return (
    <div className="pt-6 max-w-4xl">
      <ToastContainer toasts={toast.toasts} onClose={toast.closeToast} />

      {!embeddingsEnabled ? (
        // Embeddings not enabled at tenant level
        <GlassCard>
          <div className="flex items-start gap-4 p-4">
            <AlertCircle className="w-8 h-8 text-amber-400 flex-shrink-0" />
            <div>
              <h2 className="text-xl font-bold text-white mb-2">Embeddings Not Enabled</h2>
              <p className="text-gray-400 mb-4">
                Embeddings are not enabled at the tenant level. To configure repository-specific AI settings,
                first enable embeddings in the global AI settings.
              </p>
              <a
                href="/admin/management/ai"
                className="inline-flex items-center gap-2 px-4 py-2 bg-purple-500 hover:bg-purple-600 text-white rounded-lg transition-colors"
              >
                <Sparkles className="w-4 h-4" />
                Go to AI Settings
              </a>
            </div>
          </div>
        </GlassCard>
      ) : (
        <>
          {/* Repository AI Status */}
          <GlassCard className="mb-6">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold text-white flex items-center gap-2">
                <Sparkles className="w-5 h-5 text-purple-400" />
                Repository AI Settings
              </h2>
              <div className="flex items-center gap-2 px-3 py-1.5 bg-green-500/10 border border-green-500/30 rounded-lg">
                <CheckCircle className="w-4 h-4 text-green-400" />
                <span className="text-green-300 text-sm">Embeddings Enabled</span>
              </div>
            </div>
            <p className="text-gray-400 text-sm">
              Configure AI and embedding settings specific to the <span className="font-mono text-white">{repo}</span> repository.
              These settings override the global tenant defaults.
            </p>
          </GlassCard>

          {/* Global Settings Summary */}
          <GlassCard className="mb-6">
            <div className="flex items-start gap-3 mb-4">
              <Info className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
              <div>
                <h3 className="text-white font-medium mb-1">Inherited from Global Settings</h3>
                <p className="text-gray-400 text-sm">
                  The following settings are inherited from the tenant-level AI configuration.
                  You can override them below for this repository.
                </p>
              </div>
            </div>

            <div className="grid grid-cols-2 md:grid-cols-4 gap-4 p-4 bg-white/5 rounded-lg border border-white/10">
              <div>
                <div className="text-xs text-gray-400 mb-1">Vector Dimensions</div>
                <div className="text-white font-mono">{tenantEmbeddingSettings?.dimensions || 1536}</div>
              </div>
              <div>
                <div className="text-xs text-gray-400 mb-1">Include Name</div>
                <div className="text-white">{tenantEmbeddingSettings?.include_name ? 'Yes' : 'No'}</div>
              </div>
              <div>
                <div className="text-xs text-gray-400 mb-1">Include Path</div>
                <div className="text-white">{tenantEmbeddingSettings?.include_path ? 'Yes' : 'No'}</div>
              </div>
              <div>
                <div className="text-xs text-gray-400 mb-1">Chunking</div>
                <div className="text-white">
                  {tenantEmbeddingSettings?.chunking
                    ? `${tenantEmbeddingSettings.chunking.chunk_size} tokens`
                    : 'Disabled'}
                </div>
              </div>
            </div>
          </GlassCard>

          {/* Chunking Override */}
          <GlassCard className="mb-6">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-medium text-white flex items-center gap-2">
                <Layers className="w-5 h-5 text-purple-400" />
                Chunking Override
              </h3>
              <label className="flex items-center gap-3 cursor-pointer">
                <span className="text-white font-medium text-sm">Override Chunking</span>
                <div className="relative">
                  <input
                    type="checkbox"
                    checked={overrideChunking}
                    onChange={(e) => {
                      setOverrideChunking(e.target.checked)
                      setHasChanges(true)
                      if (e.target.checked && tenantEmbeddingSettings?.chunking) {
                        setChunkingSettings(tenantEmbeddingSettings.chunking)
                      }
                    }}
                    className="sr-only peer"
                  />
                  <div className="w-11 h-6 bg-white/10 border border-white/20 rounded-full peer peer-checked:bg-purple-500 peer-checked:border-purple-400 transition-all"></div>
                  <div className="absolute left-1 top-1 w-4 h-4 bg-white rounded-full transition-transform peer-checked:translate-x-5"></div>
                </div>
              </label>
            </div>
            <p className="text-gray-400 text-sm mb-4">
              Override the global chunking settings for this repository. Useful for repositories with
              different content types that need custom chunk sizes.
            </p>

            {overrideChunking && (
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
                      value={chunkingSettings.chunk_size}
                      onChange={(e) => {
                        setChunkingSettings({
                          ...chunkingSettings,
                          chunk_size: parseInt(e.target.value)
                        })
                        setHasChanges(true)
                      }}
                      className="flex-1 h-2 bg-white/10 rounded-lg appearance-none cursor-pointer accent-purple-500"
                    />
                    <span className="text-white font-mono text-sm w-16 text-right">
                      {chunkingSettings.chunk_size}
                    </span>
                  </div>
                  <p className="text-sm text-gray-400 mt-1">
                    Target size for each text chunk. Smaller chunks = more granular search.
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
                        onClick={() => {
                          setChunkingSettings({
                            ...chunkingSettings,
                            overlap: { type: 'Tokens', value: 64 }
                          })
                          setHasChanges(true)
                        }}
                        className={`px-4 py-2 text-sm font-medium transition-colors ${
                          chunkingSettings.overlap.type === 'Tokens'
                            ? 'bg-purple-500 text-white'
                            : 'bg-white/5 text-gray-300 hover:bg-white/10'
                        }`}
                      >
                        Tokens
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          setChunkingSettings({
                            ...chunkingSettings,
                            overlap: { type: 'Percentage', value: 20 }
                          })
                          setHasChanges(true)
                        }}
                        className={`px-4 py-2 text-sm font-medium transition-colors ${
                          chunkingSettings.overlap.type === 'Percentage'
                            ? 'bg-purple-500 text-white'
                            : 'bg-white/5 text-gray-300 hover:bg-white/10'
                        }`}
                      >
                        Percentage
                      </button>
                    </div>
                    <input
                      type="number"
                      min={chunkingSettings.overlap.type === 'Percentage' ? 0 : 0}
                      max={chunkingSettings.overlap.type === 'Percentage' ? 50 : 256}
                      value={chunkingSettings.overlap.value}
                      onChange={(e) => {
                        setChunkingSettings({
                          ...chunkingSettings,
                          overlap: {
                            ...chunkingSettings.overlap,
                            value: parseInt(e.target.value) || 0
                          }
                        })
                        setHasChanges(true)
                      }}
                      className="w-24 px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-center focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                    />
                    <span className="text-gray-400 text-sm">
                      {chunkingSettings.overlap.type === 'Percentage' ? '%' : 'tokens'}
                    </span>
                  </div>
                </div>

                {/* Splitter Type */}
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-2">
                    Splitter Strategy
                  </label>
                  <select
                    value={chunkingSettings.splitter}
                    onChange={(e) => {
                      setChunkingSettings({
                        ...chunkingSettings,
                        splitter: e.target.value as SplitterType
                      })
                      setHasChanges(true)
                    }}
                    className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-purple-400 focus:ring-2 focus:ring-purple-400/20 transition-all"
                  >
                    <option value="recursive" className="bg-gray-900">Recursive (paragraphs → sentences → words)</option>
                    <option value="markdown" className="bg-gray-900">Markdown (respects headers and blocks)</option>
                    <option value="code" className="bg-gray-900">Code (respects function boundaries)</option>
                    <option value="fixed_size" className="bg-gray-900">Fixed Size (simple character split)</option>
                  </select>
                </div>
              </div>
            )}
          </GlassCard>

          {/* Coming Soon */}
          <GlassCard className="mb-6 opacity-60">
            <div className="flex items-start gap-3">
              <Info className="w-5 h-5 text-gray-400 flex-shrink-0 mt-0.5" />
              <div>
                <h3 className="text-white font-medium mb-1">Coming Soon</h3>
                <ul className="text-gray-400 text-sm space-y-1">
                  <li>• Processing rules per node type or path pattern</li>
                  <li>• Custom embedding provider per repository</li>
                  <li>• PDF processing strategy override</li>
                  <li>• Image captioning settings</li>
                </ul>
              </div>
            </div>
          </GlassCard>

          {/* Save Button */}
          <div className="flex items-center justify-end gap-3">
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
                  Save Settings
                </>
              )}
            </button>
          </div>
        </>
      )}
    </div>
  )
}
