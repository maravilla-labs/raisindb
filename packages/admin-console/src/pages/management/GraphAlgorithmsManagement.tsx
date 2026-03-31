import { Plus, RefreshCw, Wifi, WifiOff, AlertTriangle } from 'lucide-react'
import { useState, useCallback } from 'react'
import { useParams } from 'react-router-dom'
import GraphStatusCard from '../../components/graph/GraphStatusCard'
import GraphConfigForm from '../../components/graph/GraphConfigForm'
import { useGraphConfigs, type GraphAlgorithmConfig } from '../../hooks/useGraphConfigs'
import { useGraphCacheSSE } from '../../hooks/useGraphCacheSSE'

interface GraphAlgorithmsManagementProps {
  repo?: string
}

export default function GraphAlgorithmsManagement({ repo: propRepo }: GraphAlgorithmsManagementProps) {
  const { repo: paramRepo } = useParams<{ repo: string }>()
  const repo = propRepo || paramRepo
  const {
    configs,
    loading,
    error,
    refresh,
    triggerRecompute,
    markStale,
    createConfig,
    updateConfig,
    deleteConfig,
  } = useGraphConfigs(repo)

  const {
    connectionStatus,
    formattedTimeRemaining,
    getComputingProgress,
  } = useGraphCacheSSE(repo)

  const [actionPending, setActionPending] = useState<string | null>(null)

  // Form state
  const [isFormOpen, setIsFormOpen] = useState(false)
  const [formMode, setFormMode] = useState<'create' | 'edit'>('create')
  const [editingConfig, setEditingConfig] = useState<GraphAlgorithmConfig | undefined>(undefined)

  // Delete confirmation state
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false)
  const [deletingConfigId, setDeletingConfigId] = useState<string | null>(null)
  const [deleteError, setDeleteError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState(false)

  const handleRecompute = useCallback(async (configId: string) => {
    setActionPending(configId)
    const result = await triggerRecompute(configId)
    if (!result.success) {
      console.error('Failed to trigger recompute:', result.error)
    }
    setActionPending(null)
  }, [triggerRecompute])

  const handleMarkStale = useCallback(async (configId: string) => {
    setActionPending(configId)
    const result = await markStale(configId)
    if (!result.success) {
      console.error('Failed to mark stale:', result.error)
    }
    setActionPending(null)
  }, [markStale])

  const handleEdit = useCallback((configId: string) => {
    const configStatus = configs.find(c => c.id === configId)
    if (configStatus) {
      setEditingConfig(configStatus.config)
      setFormMode('edit')
      setIsFormOpen(true)
    }
  }, [configs])

  const handleDelete = useCallback((configId: string) => {
    setDeletingConfigId(configId)
    setDeleteError(null)
    setDeleteConfirmOpen(true)
  }, [])

  const handleConfirmDelete = useCallback(async () => {
    if (!deletingConfigId) return

    setDeleting(true)
    setDeleteError(null)

    const result = await deleteConfig(deletingConfigId)

    setDeleting(false)

    if (result.success) {
      setDeleteConfirmOpen(false)
      setDeletingConfigId(null)
    } else {
      setDeleteError(result.error || 'Failed to delete config')
    }
  }, [deletingConfigId, deleteConfig])

  const handleCancelDelete = useCallback(() => {
    setDeleteConfirmOpen(false)
    setDeletingConfigId(null)
    setDeleteError(null)
  }, [])

  const handleNewConfig = useCallback(() => {
    setEditingConfig(undefined)
    setFormMode('create')
    setIsFormOpen(true)
  }, [])

  const handleFormClose = useCallback(() => {
    setIsFormOpen(false)
    setEditingConfig(undefined)
  }, [])

  const handleFormSave = useCallback(async (config: GraphAlgorithmConfig) => {
    if (formMode === 'create') {
      return createConfig(config)
    } else {
      return updateConfig(config.id, config)
    }
  }, [formMode, createConfig, updateConfig])

  return (
    <div className="p-6 space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-2xl font-bold text-white">Graph Algorithm Configurations</h2>
          <p className="text-gray-400 mt-1">
            Manage precomputed graph algorithm results for efficient querying
          </p>
        </div>

        <button
          onClick={handleNewConfig}
          className="flex items-center gap-2 px-4 py-2 bg-purple-600 hover:bg-purple-500 text-white rounded-lg font-medium transition-all"
        >
          <Plus className="w-5 h-5" />
          New Config
        </button>
      </div>

      {/* Status bar */}
      <div className="flex items-center justify-between p-4 rounded-lg bg-white/5 border border-white/10">
        <div className="flex items-center gap-6">
          {/* Connection status */}
          <div className="flex items-center gap-2">
            {connectionStatus === 'connected' ? (
              <>
                <span className="relative flex h-2.5 w-2.5">
                  <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-green-400 opacity-75"></span>
                  <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-green-500"></span>
                </span>
                <span className="text-sm text-green-400 font-medium">Connected</span>
                <Wifi className="w-4 h-4 text-green-400" />
              </>
            ) : connectionStatus === 'connecting' ? (
              <>
                <RefreshCw className="w-4 h-4 text-yellow-400 animate-spin" />
                <span className="text-sm text-yellow-400">Connecting...</span>
              </>
            ) : (
              <>
                <span className="relative flex h-2.5 w-2.5">
                  <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-gray-500"></span>
                </span>
                <span className="text-sm text-gray-400">Disconnected</span>
                <WifiOff className="w-4 h-4 text-gray-400" />
              </>
            )}
          </div>

          {/* Next tick countdown */}
          <div className="flex items-center gap-2 text-sm">
            <span className="text-gray-400">Next background tick in:</span>
            <span className="font-mono font-medium text-purple-400">
              {formattedTimeRemaining}
            </span>
          </div>
        </div>

        <button
          onClick={refresh}
          disabled={loading}
          className="flex items-center gap-2 px-3 py-1.5 rounded-lg text-sm font-medium bg-gray-700 hover:bg-gray-600 text-white transition-all disabled:opacity-50"
        >
          <RefreshCw className={`w-4 h-4 ${loading ? 'animate-spin' : ''}`} />
          Refresh
        </button>
      </div>

      {/* Error state */}
      {error && (
        <div className="p-4 rounded-lg bg-red-900/30 border border-red-500/30 text-red-300">
          <p className="font-medium">Failed to load configurations</p>
          <p className="text-sm text-red-400 mt-1">{error}</p>
        </div>
      )}

      {/* Loading state */}
      {loading && configs.length === 0 && (
        <div className="flex items-center justify-center py-12">
          <RefreshCw className="w-8 h-8 text-purple-400 animate-spin" />
        </div>
      )}

      {/* Empty state */}
      {!loading && configs.length === 0 && !error && (
        <div className="text-center py-12">
          <div className="w-16 h-16 mx-auto mb-4 rounded-full bg-gray-800 flex items-center justify-center">
            <Plus className="w-8 h-8 text-gray-500" />
          </div>
          <h3 className="text-lg font-medium text-white mb-2">No configurations yet</h3>
          <p className="text-gray-400 max-w-md mx-auto mb-6">
            Graph algorithm configurations enable you to precompute PageRank, community detection,
            and other graph analytics for efficient querying.
          </p>
          <button
            onClick={handleNewConfig}
            className="inline-flex items-center gap-2 px-4 py-2 bg-purple-600 hover:bg-purple-500 text-white rounded-lg font-medium transition-all"
          >
            <Plus className="w-5 h-5" />
            Create Your First Config
          </button>
        </div>
      )}

      {/* Config cards */}
      {configs.length > 0 && (
        <div className="space-y-4">
          {configs.map((config) => (
            <GraphStatusCard
              key={config.id}
              config={config}
              computingProgress={getComputingProgress(config.id)}
              onRecompute={handleRecompute}
              onMarkStale={handleMarkStale}
              onEdit={handleEdit}
              onDelete={handleDelete}
              isActionPending={actionPending === config.id}
            />
          ))}
        </div>
      )}

      {/* Create/Edit Form Modal */}
      <GraphConfigForm
        isOpen={isFormOpen}
        onClose={handleFormClose}
        onSave={handleFormSave}
        initialConfig={editingConfig}
        mode={formMode}
      />

      {/* Delete Confirmation Dialog */}
      {deleteConfirmOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center">
          {/* Backdrop */}
          <div
            className="absolute inset-0 bg-black/60 backdrop-blur-sm"
            onClick={handleCancelDelete}
          />

          {/* Dialog */}
          <div className="relative z-10 w-full max-w-md bg-gray-900 border border-white/10 rounded-xl shadow-2xl p-6">
            <div className="flex items-start gap-4">
              <div className="flex-shrink-0 w-10 h-10 rounded-full bg-red-900/50 flex items-center justify-center">
                <AlertTriangle className="w-5 h-5 text-red-400" />
              </div>
              <div className="flex-1">
                <h3 className="text-lg font-semibold text-white">Delete Configuration</h3>
                <p className="text-gray-400 mt-2">
                  Are you sure you want to delete the configuration{' '}
                  <span className="font-mono text-purple-400">{deletingConfigId}</span>?
                  This will also remove all cached computation results.
                </p>
                <p className="text-sm text-red-400 mt-2">
                  This action cannot be undone.
                </p>

                {deleteError && (
                  <div className="mt-3 p-3 rounded-lg bg-red-900/30 border border-red-500/30 text-red-300 text-sm">
                    {deleteError}
                  </div>
                )}

                <div className="flex justify-end gap-3 mt-6">
                  <button
                    type="button"
                    onClick={handleCancelDelete}
                    disabled={deleting}
                    className="px-4 py-2 rounded-lg text-gray-300 hover:text-white hover:bg-white/10 transition-all disabled:opacity-50"
                  >
                    Cancel
                  </button>
                  <button
                    type="button"
                    onClick={handleConfirmDelete}
                    disabled={deleting}
                    className="px-4 py-2 rounded-lg bg-red-600 hover:bg-red-500 text-white font-medium transition-all disabled:opacity-50"
                  >
                    {deleting ? 'Deleting...' : 'Delete'}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
