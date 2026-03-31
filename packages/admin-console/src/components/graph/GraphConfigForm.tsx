import { X, Plus, Trash2 } from 'lucide-react'
import { useState, useEffect, useCallback } from 'react'
import type { GraphAlgorithmConfig, GraphTarget, GraphScope, RefreshConfig } from '../../hooks/useGraphConfigs'

interface GraphConfigFormProps {
  isOpen: boolean
  onClose: () => void
  onSave: (config: GraphAlgorithmConfig) => Promise<{ success: boolean; error?: string }>
  initialConfig?: GraphAlgorithmConfig
  mode: 'create' | 'edit'
}

const ALGORITHMS = [
  { value: 'pagerank', label: 'PageRank' },
  { value: 'louvain', label: 'Louvain Community Detection' },
  { value: 'connected_components', label: 'Connected Components' },
  { value: 'triangle_count', label: 'Triangle Count' },
  { value: 'betweenness_centrality', label: 'Betweenness Centrality' },
  { value: 'relates_cache', label: 'RELATES Cache' },
]

const TARGET_MODES = [
  { value: 'all_branches', label: 'All Branches' },
  { value: 'branch', label: 'Specific Branches' },
  { value: 'branch_pattern', label: 'Branch Pattern (glob)' },
  { value: 'revision', label: 'Specific Revisions' },
]

const DEFAULT_CONFIG: GraphAlgorithmConfig = {
  id: '',
  algorithm: 'pagerank',
  enabled: true,
  target: {
    mode: 'all_branches',
    branches: [],
    revisions: [],
    branch_pattern: undefined,
  },
  scope: {
    paths: [],
    node_types: [],
    workspaces: [],
    relation_types: [],
  },
  algorithm_config: {},
  refresh: {
    ttl_seconds: 3600,
    on_branch_change: true,
    on_relation_change: false,
    cron: undefined,
  },
}

export default function GraphConfigForm({
  isOpen,
  onClose,
  onSave,
  initialConfig,
  mode,
}: GraphConfigFormProps) {
  const [config, setConfig] = useState<GraphAlgorithmConfig>(DEFAULT_CONFIG)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // Reset form when opened
  useEffect(() => {
    if (isOpen) {
      if (initialConfig) {
        setConfig(initialConfig)
      } else {
        setConfig(DEFAULT_CONFIG)
      }
      setError(null)
    }
  }, [isOpen, initialConfig])

  const handleSubmit = useCallback(async (e: React.FormEvent) => {
    e.preventDefault()
    setError(null)

    // Validate ID
    if (!config.id.trim()) {
      setError('Config ID is required')
      return
    }

    // Validate ID format (alphanumeric and hyphens only)
    if (!/^[a-z0-9-]+$/.test(config.id)) {
      setError('Config ID must contain only lowercase letters, numbers, and hyphens')
      return
    }

    setSaving(true)
    const result = await onSave(config)
    setSaving(false)

    if (result.success) {
      onClose()
    } else {
      setError(result.error || 'Failed to save config')
    }
  }, [config, onSave, onClose])

  const updateTarget = useCallback((updates: Partial<GraphTarget>) => {
    setConfig(prev => ({
      ...prev,
      target: { ...prev.target, ...updates },
    }))
  }, [])

  const updateScope = useCallback((updates: Partial<GraphScope>) => {
    setConfig(prev => ({
      ...prev,
      scope: { ...prev.scope, ...updates },
    }))
  }, [])

  const updateRefresh = useCallback((updates: Partial<RefreshConfig>) => {
    setConfig(prev => ({
      ...prev,
      refresh: { ...prev.refresh, ...updates },
    }))
  }, [])

  const updateAlgorithmConfig = useCallback((key: string, value: unknown) => {
    setConfig(prev => ({
      ...prev,
      algorithm_config: { ...prev.algorithm_config, [key]: value },
    }))
  }, [])

  if (!isOpen) return null

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 backdrop-blur-sm"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="relative z-10 w-full max-w-2xl max-h-[90vh] overflow-y-auto bg-gray-900 border border-white/10 rounded-xl shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b border-white/10">
          <h2 className="text-xl font-semibold text-white">
            {mode === 'create' ? 'Create Graph Algorithm Config' : 'Edit Graph Algorithm Config'}
          </h2>
          <button
            onClick={onClose}
            className="p-2 rounded-lg text-gray-400 hover:text-white hover:bg-white/10 transition-all"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="p-4 space-y-6">
          {/* Error message */}
          {error && (
            <div className="p-3 rounded-lg bg-red-900/30 border border-red-500/30 text-red-300 text-sm">
              {error}
            </div>
          )}

          {/* Basic Info Section */}
          <section className="space-y-4">
            <h3 className="text-sm font-medium text-gray-400 uppercase tracking-wide">Basic Info</h3>

            <div className="grid grid-cols-2 gap-4">
              {/* ID */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Config ID *
                </label>
                <input
                  type="text"
                  value={config.id}
                  onChange={(e) => setConfig(prev => ({ ...prev, id: e.target.value.toLowerCase() }))}
                  disabled={mode === 'edit'}
                  placeholder="e.g., pagerank-social"
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white placeholder-gray-500 focus:outline-none focus:border-purple-500 disabled:opacity-50"
                />
              </div>

              {/* Algorithm */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Algorithm *
                </label>
                <select
                  value={config.algorithm}
                  onChange={(e) => setConfig(prev => ({ ...prev, algorithm: e.target.value, algorithm_config: {} }))}
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                >
                  {ALGORITHMS.map(alg => (
                    <option key={alg.value} value={alg.value} className="bg-gray-800">
                      {alg.label}
                    </option>
                  ))}
                </select>
              </div>
            </div>

            {/* Enabled toggle */}
            <div className="flex items-center gap-3">
              <label className="relative inline-flex items-center cursor-pointer">
                <input
                  type="checkbox"
                  checked={config.enabled}
                  onChange={(e) => setConfig(prev => ({ ...prev, enabled: e.target.checked }))}
                  className="sr-only peer"
                />
                <div className="w-11 h-6 bg-gray-700 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full rtl:peer-checked:after:-translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:start-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:bg-purple-600"></div>
              </label>
              <span className="text-sm text-gray-300">Enabled</span>
            </div>
          </section>

          {/* Target Section */}
          <section className="space-y-4">
            <h3 className="text-sm font-medium text-gray-400 uppercase tracking-wide">Target</h3>

            <div>
              <label className="block text-sm font-medium text-gray-300 mb-1">
                Target Mode
              </label>
              <select
                value={config.target.mode}
                onChange={(e) => updateTarget({ mode: e.target.value as GraphTarget['mode'] })}
                className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
              >
                {TARGET_MODES.map(mode => (
                  <option key={mode.value} value={mode.value} className="bg-gray-800">
                    {mode.label}
                  </option>
                ))}
              </select>
            </div>

            {/* Branches (when mode=branch) */}
            {config.target.mode === 'branch' && (
              <ArrayInput
                label="Branches"
                value={config.target.branches || []}
                onChange={(branches) => updateTarget({ branches })}
                placeholder="e.g., main"
              />
            )}

            {/* Branch Pattern (when mode=branch_pattern) */}
            {config.target.mode === 'branch_pattern' && (
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Branch Pattern (glob)
                </label>
                <input
                  type="text"
                  value={config.target.branch_pattern || ''}
                  onChange={(e) => updateTarget({ branch_pattern: e.target.value || undefined })}
                  placeholder="e.g., release/*"
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white placeholder-gray-500 focus:outline-none focus:border-purple-500"
                />
              </div>
            )}

            {/* Revisions (when mode=revision) */}
            {config.target.mode === 'revision' && (
              <ArrayInput
                label="Revisions"
                value={config.target.revisions || []}
                onChange={(revisions) => updateTarget({ revisions })}
                placeholder="e.g., abc123"
              />
            )}
          </section>

          {/* Scope Section */}
          <section className="space-y-4">
            <h3 className="text-sm font-medium text-gray-400 uppercase tracking-wide">Scope (Optional)</h3>
            <p className="text-xs text-gray-500">Leave empty to include all nodes</p>

            <div className="grid grid-cols-2 gap-4">
              <ArrayInput
                label="Node Types"
                value={config.scope.node_types || []}
                onChange={(node_types) => updateScope({ node_types })}
                placeholder="e.g., raisin:User"
              />
              <ArrayInput
                label="Relation Types"
                value={config.scope.relation_types || []}
                onChange={(relation_types) => updateScope({ relation_types })}
                placeholder="e.g., FOLLOWS"
              />
            </div>

            <ArrayInput
              label="Paths"
              value={config.scope.paths || []}
              onChange={(paths) => updateScope({ paths })}
              placeholder="e.g., /users/**"
            />
          </section>

          {/* Algorithm Config Section */}
          <section className="space-y-4">
            <h3 className="text-sm font-medium text-gray-400 uppercase tracking-wide">Algorithm Settings</h3>

            {config.algorithm === 'pagerank' && (
              <div className="grid grid-cols-3 gap-4">
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-1">
                    Damping Factor
                  </label>
                  <input
                    type="number"
                    step="0.01"
                    min="0"
                    max="1"
                    value={(config.algorithm_config.damping_factor as number) ?? 0.85}
                    onChange={(e) => updateAlgorithmConfig('damping_factor', parseFloat(e.target.value))}
                    className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-1">
                    Max Iterations
                  </label>
                  <input
                    type="number"
                    min="1"
                    value={(config.algorithm_config.max_iterations as number) ?? 100}
                    onChange={(e) => updateAlgorithmConfig('max_iterations', parseInt(e.target.value))}
                    className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-1">
                    Convergence
                  </label>
                  <input
                    type="number"
                    step="0.00001"
                    min="0"
                    value={(config.algorithm_config.convergence_threshold as number) ?? 0.0001}
                    onChange={(e) => updateAlgorithmConfig('convergence_threshold', parseFloat(e.target.value))}
                    className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                  />
                </div>
              </div>
            )}

            {config.algorithm === 'louvain' && (
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-1">
                    Resolution
                  </label>
                  <input
                    type="number"
                    step="0.1"
                    min="0"
                    value={(config.algorithm_config.resolution as number) ?? 1.0}
                    onChange={(e) => updateAlgorithmConfig('resolution', parseFloat(e.target.value))}
                    className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-1">
                    Max Iterations
                  </label>
                  <input
                    type="number"
                    min="1"
                    value={(config.algorithm_config.max_iterations as number) ?? 100}
                    onChange={(e) => updateAlgorithmConfig('max_iterations', parseInt(e.target.value))}
                    className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                  />
                </div>
              </div>
            )}

            {config.algorithm === 'relates_cache' && (
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Max Depth
                </label>
                <input
                  type="number"
                  min="1"
                  max="10"
                  value={(config.algorithm_config.max_depth as number) ?? 2}
                  onChange={(e) => updateAlgorithmConfig('max_depth', parseInt(e.target.value))}
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                />
              </div>
            )}

            {['connected_components', 'triangle_count', 'betweenness_centrality'].includes(config.algorithm) && (
              <p className="text-sm text-gray-500 italic">No additional settings for this algorithm</p>
            )}
          </section>

          {/* Refresh Section */}
          <section className="space-y-4">
            <h3 className="text-sm font-medium text-gray-400 uppercase tracking-wide">Refresh Triggers</h3>

            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  TTL (seconds)
                </label>
                <input
                  type="number"
                  min="0"
                  value={config.refresh.ttl_seconds}
                  onChange={(e) => updateRefresh({ ttl_seconds: parseInt(e.target.value) || 0 })}
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white focus:outline-none focus:border-purple-500"
                />
                <p className="text-xs text-gray-500 mt-1">0 = disabled</p>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Cron Schedule
                </label>
                <input
                  type="text"
                  value={config.refresh.cron || ''}
                  onChange={(e) => updateRefresh({ cron: e.target.value || undefined })}
                  placeholder="e.g., 0 */6 * * *"
                  className="w-full px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white placeholder-gray-500 focus:outline-none focus:border-purple-500"
                />
              </div>
            </div>

            <div className="flex gap-6">
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={config.refresh.on_branch_change}
                  onChange={(e) => updateRefresh({ on_branch_change: e.target.checked })}
                  className="w-4 h-4 rounded border-gray-600 bg-white/5 text-purple-600 focus:ring-purple-500"
                />
                <span className="text-sm text-gray-300">Recompute on branch change</span>
              </label>

              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={config.refresh.on_relation_change}
                  onChange={(e) => updateRefresh({ on_relation_change: e.target.checked })}
                  className="w-4 h-4 rounded border-gray-600 bg-white/5 text-purple-600 focus:ring-purple-500"
                />
                <span className="text-sm text-gray-300">Recompute on relation change</span>
              </label>
            </div>
          </section>

          {/* Footer */}
          <div className="flex justify-end gap-3 pt-4 border-t border-white/10">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-lg text-gray-300 hover:text-white hover:bg-white/10 transition-all"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={saving}
              className="px-4 py-2 rounded-lg bg-purple-600 hover:bg-purple-500 text-white font-medium transition-all disabled:opacity-50"
            >
              {saving ? 'Saving...' : mode === 'create' ? 'Create Config' : 'Save Changes'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}

// Helper component for array inputs
interface ArrayInputProps {
  label: string
  value: string[]
  onChange: (value: string[]) => void
  placeholder: string
}

function ArrayInput({ label, value, onChange, placeholder }: ArrayInputProps) {
  const [inputValue, setInputValue] = useState('')

  const handleAdd = () => {
    const trimmed = inputValue.trim()
    if (trimmed && !value.includes(trimmed)) {
      onChange([...value, trimmed])
      setInputValue('')
    }
  }

  const handleRemove = (item: string) => {
    onChange(value.filter(v => v !== item))
  }

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault()
      handleAdd()
    }
  }

  return (
    <div>
      <label className="block text-sm font-medium text-gray-300 mb-1">
        {label}
      </label>
      <div className="flex gap-2">
        <input
          type="text"
          value={inputValue}
          onChange={(e) => setInputValue(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          className="flex-1 px-3 py-2 rounded-lg bg-white/5 border border-white/10 text-white placeholder-gray-500 focus:outline-none focus:border-purple-500"
        />
        <button
          type="button"
          onClick={handleAdd}
          className="px-3 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white transition-all"
        >
          <Plus className="w-4 h-4" />
        </button>
      </div>
      {value.length > 0 && (
        <div className="flex flex-wrap gap-2 mt-2">
          {value.map((item) => (
            <span
              key={item}
              className="inline-flex items-center gap-1 px-2 py-1 rounded-md bg-purple-600/20 text-purple-300 text-sm"
            >
              {item}
              <button
                type="button"
                onClick={() => handleRemove(item)}
                className="p-0.5 rounded hover:bg-white/10"
              >
                <Trash2 className="w-3 h-3" />
              </button>
            </span>
          ))}
        </div>
      )}
    </div>
  )
}
