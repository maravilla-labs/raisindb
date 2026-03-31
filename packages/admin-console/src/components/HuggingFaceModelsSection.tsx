import { useState, useEffect } from 'react'
import {
  Download,
  Trash2,
  Loader2,
  CheckCircle,
  XCircle,
  HardDrive,
  ExternalLink,
  RefreshCw,
  AlertCircle,
  Package,
} from 'lucide-react'
import GlassCard from './GlassCard'
import {
  aiApi,
  HuggingFaceModel,
  HuggingFaceDownloadStatus,
} from '../api/ai'
import { ApiError } from '../api/client'

interface HuggingFaceModelsSectionProps {
  tenantId: string
  onError: (title: string, message: string) => void
  onSuccess: (title: string, message: string) => void
}

function getStatusColor(status: HuggingFaceDownloadStatus): string {
  switch (status.type) {
    case 'ready':
      return 'text-green-400'
    case 'downloading':
      return 'text-blue-400'
    case 'failed':
      return 'text-red-400'
    default:
      return 'text-gray-400'
  }
}

function getStatusIcon(status: HuggingFaceDownloadStatus) {
  switch (status.type) {
    case 'ready':
      return <CheckCircle className="w-4 h-4 text-green-400" />
    case 'downloading':
      return <Loader2 className="w-4 h-4 text-blue-400 animate-spin" />
    case 'failed':
      return <XCircle className="w-4 h-4 text-red-400" />
    default:
      return <Package className="w-4 h-4 text-gray-400" />
  }
}

function getStatusText(status: HuggingFaceDownloadStatus): string {
  switch (status.type) {
    case 'ready':
      return 'Downloaded'
    case 'downloading':
      return `Downloading ${Math.round(status.progress * 100)}%`
    case 'failed':
      return `Failed: ${status.error}`
    default:
      return 'Not Downloaded'
  }
}

function ModelTypeTag({ type }: { type: string }) {
  const colors: Record<string, string> = {
    'CLIP': 'bg-purple-500/20 text-purple-300 border-purple-500/30',
    'BLIP': 'bg-blue-500/20 text-blue-300 border-blue-500/30',
    'Text Embedding': 'bg-green-500/20 text-green-300 border-green-500/30',
    'OCR': 'bg-orange-500/20 text-orange-300 border-orange-500/30',
    'Whisper': 'bg-pink-500/20 text-pink-300 border-pink-500/30',
  }
  const colorClass = colors[type] || 'bg-gray-500/20 text-gray-300 border-gray-500/30'

  return (
    <span className={`px-2 py-0.5 text-xs rounded-full border ${colorClass}`}>
      {type}
    </span>
  )
}

function ModelCard({
  model,
  onDownload,
  onDelete,
  downloading,
  deleting,
}: {
  model: HuggingFaceModel
  onDownload: () => void
  onDelete: () => void
  downloading: boolean
  deleting: boolean
}) {
  const isReady = model.status.type === 'ready'
  const isDownloading = model.status.type === 'downloading' || downloading

  return (
    <GlassCard className="p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 mb-1">
            <h3 className="text-white font-medium truncate">{model.display_name}</h3>
            <ModelTypeTag type={model.model_type} />
          </div>
          <p className="text-sm text-gray-400 truncate mb-2">{model.model_id}</p>
          {model.description && (
            <p className="text-sm text-gray-500 line-clamp-2">{model.description}</p>
          )}
          <div className="flex items-center gap-4 mt-3">
            <div className="flex items-center gap-1.5 text-sm text-gray-400">
              <HardDrive className="w-4 h-4" />
              <span>{model.size_display}</span>
            </div>
            <div className="flex items-center gap-1.5 text-sm">
              {getStatusIcon(model.status)}
              <span className={getStatusColor(model.status)}>
                {getStatusText(model.status)}
              </span>
            </div>
            <a
              href={model.model_url}
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1 text-sm text-purple-400 hover:text-purple-300 transition-colors"
            >
              <ExternalLink className="w-4 h-4" />
              View on HuggingFace
            </a>
          </div>
        </div>
        <div className="flex items-center gap-2">
          {isReady ? (
            <button
              onClick={onDelete}
              disabled={deleting}
              className="flex items-center gap-1.5 px-3 py-1.5 text-sm text-red-400 hover:text-red-300 hover:bg-red-500/10 border border-red-500/30 rounded-lg transition-colors disabled:opacity-50"
            >
              {deleting ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Trash2 className="w-4 h-4" />
              )}
              Delete
            </button>
          ) : (
            <button
              onClick={onDownload}
              disabled={isDownloading}
              className="flex items-center gap-1.5 px-3 py-1.5 text-sm text-purple-400 hover:text-purple-300 hover:bg-purple-500/10 border border-purple-500/30 rounded-lg transition-colors disabled:opacity-50"
            >
              {isDownloading ? (
                <Loader2 className="w-4 h-4 animate-spin" />
              ) : (
                <Download className="w-4 h-4" />
              )}
              Download
            </button>
          )}
        </div>
      </div>

      {/* Download progress bar */}
      {model.status.type === 'downloading' && (
        <div className="mt-3">
          <div className="w-full bg-gray-700/50 rounded-full h-1.5">
            <div
              className="bg-blue-500 h-1.5 rounded-full transition-all duration-300"
              style={{ width: `${model.status.progress * 100}%` }}
            />
          </div>
        </div>
      )}
    </GlassCard>
  )
}

export default function HuggingFaceModelsSection({
  tenantId,
  onError,
  onSuccess,
}: HuggingFaceModelsSectionProps) {
  const [models, setModels] = useState<HuggingFaceModel[]>([])
  const [totalDiskUsage, setTotalDiskUsage] = useState<string>('0 B')
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [downloadingModels, setDownloadingModels] = useState<Set<string>>(new Set())
  const [deletingModels, setDeletingModels] = useState<Set<string>>(new Set())

  const loadModels = async (showRefreshing = false) => {
    try {
      if (showRefreshing) setRefreshing(true)
      else setLoading(true)

      const response = await aiApi.listHuggingFaceModels(tenantId)
      setModels(response.models)
      setTotalDiskUsage(response.total_disk_usage)
    } catch (error) {
      console.error('Failed to load HuggingFace models:', error)
      onError('Failed to load models', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setLoading(false)
      setRefreshing(false)
    }
  }

  useEffect(() => {
    loadModels()
  }, [tenantId])

  const handleDownload = async (modelId: string) => {
    try {
      setDownloadingModels((prev) => new Set(prev).add(modelId))
      const response = await aiApi.downloadHuggingFaceModel(tenantId, modelId)
      onSuccess('Download started', `Model ${modelId} is being downloaded (job: ${response.job_id})`)

      // Poll for status updates
      const pollInterval = setInterval(async () => {
        try {
          const updated = await aiApi.getHuggingFaceModel(tenantId, modelId)
          setModels((prev) =>
            prev.map((m) => (m.model_id === modelId ? updated : m))
          )
          if (updated.status.type === 'ready' || updated.status.type === 'failed') {
            clearInterval(pollInterval)
            setDownloadingModels((prev) => {
              const next = new Set(prev)
              next.delete(modelId)
              return next
            })
            if (updated.status.type === 'ready') {
              onSuccess('Download complete', `Model ${modelId} is ready to use`)
            } else if (updated.status.type === 'failed') {
              onError('Download failed', updated.status.error)
            }
            // Refresh to get updated disk usage
            loadModels(true)
          }
        } catch {
          // Ignore polling errors
        }
      }, 2000)
    } catch (error) {
      console.error('Failed to download model:', error)
      onError('Download failed', error instanceof ApiError ? error.message : 'Unknown error')
      setDownloadingModels((prev) => {
        const next = new Set(prev)
        next.delete(modelId)
        return next
      })
    }
  }

  const handleDelete = async (modelId: string) => {
    try {
      setDeletingModels((prev) => new Set(prev).add(modelId))
      await aiApi.deleteHuggingFaceModel(tenantId, modelId)
      onSuccess('Model deleted', `Model ${modelId} has been removed`)
      loadModels(true)
    } catch (error) {
      console.error('Failed to delete model:', error)
      onError('Delete failed', error instanceof ApiError ? error.message : 'Unknown error')
    } finally {
      setDeletingModels((prev) => {
        const next = new Set(prev)
        next.delete(modelId)
        return next
      })
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-32">
        <Loader2 className="w-6 h-6 text-purple-400 animate-spin" />
      </div>
    )
  }

  const downloadedCount = models.filter((m) => m.status.type === 'ready').length

  return (
    <div className="space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-lg font-semibold text-white flex items-center gap-2">
            <Package className="w-5 h-5 text-purple-400" />
            HuggingFace Models
          </h3>
          <p className="text-sm text-gray-400">
            {downloadedCount} of {models.length} models downloaded ({totalDiskUsage} used)
          </p>
        </div>
        <button
          onClick={() => loadModels(true)}
          disabled={refreshing}
          className="flex items-center gap-1.5 px-3 py-1.5 text-sm text-gray-400 hover:text-white hover:bg-white/5 rounded-lg transition-colors disabled:opacity-50"
        >
          <RefreshCw className={`w-4 h-4 ${refreshing ? 'animate-spin' : ''}`} />
          Refresh
        </button>
      </div>

      {/* Info banner */}
      <div className="flex items-start gap-3 p-3 bg-blue-500/10 border border-blue-500/20 rounded-lg">
        <AlertCircle className="w-5 h-5 text-blue-400 flex-shrink-0 mt-0.5" />
        <p className="text-sm text-blue-300">
          These models are used for local AI inference (image embeddings, captioning, OCR).
          Download models to enable processing without external API calls.
        </p>
      </div>

      {/* Models list */}
      <div className="space-y-3">
        {models.map((model) => (
          <ModelCard
            key={model.model_id}
            model={model}
            onDownload={() => handleDownload(model.model_id)}
            onDelete={() => handleDelete(model.model_id)}
            downloading={downloadingModels.has(model.model_id)}
            deleting={deletingModels.has(model.model_id)}
          />
        ))}
      </div>

      {models.length === 0 && (
        <div className="text-center py-8 text-gray-500">
          No HuggingFace models available
        </div>
      )}
    </div>
  )
}
