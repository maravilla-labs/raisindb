import {
  Activity,
  AlertCircle,
  CheckCircle,
  ChevronDown,
  ChevronUp,
  Clock,
  Loader2,
  Play,
  RefreshCw,
  Trash2,
  XCircle,
} from 'lucide-react'
import { useState } from 'react'
import type { ConfigStatus } from '../../hooks/useGraphConfigs'
import type { ComputingConfig } from '../../hooks/useGraphCacheSSE'

interface GraphStatusCardProps {
  config: ConfigStatus
  computingProgress?: ComputingConfig
  onRecompute: (configId: string) => void
  onMarkStale: (configId: string) => void
  onEdit: (configId: string) => void
  onDelete: (configId: string) => void
  isActionPending?: boolean
}

const algorithmLabels: Record<string, string> = {
  pagerank: 'PageRank',
  louvain: 'Louvain',
  connected_components: 'Connected Components',
  triangle_count: 'Triangle Count',
  betweenness_centrality: 'Betweenness Centrality',
  relates_cache: 'RELATES Cache',
}

const statusConfig = {
  ready: {
    icon: CheckCircle,
    label: 'Ready',
    color: 'text-green-400',
    bg: 'bg-green-400/10',
    border: 'border-green-400/20',
  },
  computing: {
    icon: Loader2,
    label: 'Computing',
    color: 'text-blue-400',
    bg: 'bg-blue-400/10',
    border: 'border-blue-400/20',
  },
  stale: {
    icon: Clock,
    label: 'Stale',
    color: 'text-yellow-400',
    bg: 'bg-yellow-400/10',
    border: 'border-yellow-400/20',
  },
  pending: {
    icon: Clock,
    label: 'Pending',
    color: 'text-gray-400',
    bg: 'bg-gray-400/10',
    border: 'border-gray-400/20',
  },
  error: {
    icon: XCircle,
    label: 'Error',
    color: 'text-red-400',
    bg: 'bg-red-400/10',
    border: 'border-red-400/20',
  },
}

function formatTimestamp(ts: number | undefined): string {
  if (!ts) return 'Never'
  const date = new Date(ts)
  const now = new Date()
  const diffMs = now.getTime() - date.getTime()
  const diffMins = Math.floor(diffMs / 60000)

  if (diffMins < 1) return 'Just now'
  if (diffMins < 60) return `${diffMins}m ago`
  if (diffMins < 1440) return `${Math.floor(diffMins / 60)}h ago`
  return date.toLocaleDateString()
}

function formatNumber(n: number | undefined): string {
  if (n === undefined || n === null) return '--'
  if (n >= 1000000) return `${(n / 1000000).toFixed(1)}M`
  if (n >= 1000) return `${(n / 1000).toFixed(1)}K`
  return n.toString()
}

export default function GraphStatusCard({
  config,
  computingProgress,
  onRecompute,
  onMarkStale,
  onEdit,
  onDelete,
  isActionPending,
}: GraphStatusCardProps) {
  const [isExpanded, setIsExpanded] = useState(false)

  const status = statusConfig[config.status] || statusConfig.pending
  const StatusIcon = status.icon
  const algorithmLabel = algorithmLabels[config.algorithm] || config.algorithm
  const isComputing = config.status === 'computing' || !!computingProgress

  return (
    <div
      className={`
        rounded-lg border ${status.border} ${status.bg}
        backdrop-blur-sm transition-all duration-200
        hover:border-white/20
      `}
    >
      {/* Main content */}
      <div className="p-4">
        {/* Header row */}
        <div className="flex items-start justify-between gap-4">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <h3 className="text-lg font-semibold text-white truncate">
                {config.id}
              </h3>
              {!config.enabled && (
                <span className="px-2 py-0.5 text-xs rounded-full bg-gray-600 text-gray-300">
                  Disabled
                </span>
              )}
            </div>
            <p className="text-sm text-gray-400 mt-0.5">
              Algorithm: {algorithmLabel}
            </p>
          </div>

          {/* Status badge */}
          <div className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full ${status.bg}`}>
            <StatusIcon
              className={`w-4 h-4 ${status.color} ${isComputing ? 'animate-spin' : ''}`}
            />
            <span className={`text-sm font-medium ${status.color}`}>
              {status.label}
            </span>
          </div>
        </div>

        {/* Stats row */}
        <div className="flex items-center gap-6 mt-4 text-sm">
          <div className="flex items-center gap-1.5">
            <Activity className="w-4 h-4 text-gray-500" />
            <span className="text-gray-400">Nodes:</span>
            <span className="text-white font-medium">
              {formatNumber(config.node_count)}
            </span>
          </div>

          <div className="flex items-center gap-1.5">
            <Clock className="w-4 h-4 text-gray-500" />
            <span className="text-gray-400">Last:</span>
            <span className="text-white font-medium">
              {formatTimestamp(config.last_computed_at)}
            </span>
          </div>
        </div>

        {/* Progress bar (when computing) */}
        {computingProgress && (
          <div className="mt-4">
            <div className="flex items-center justify-between text-sm mb-1.5">
              <span className="text-gray-400 truncate max-w-[70%]">
                {computingProgress.currentStep}
              </span>
              <span className="text-blue-400 font-medium">
                {computingProgress.progress}%
              </span>
            </div>
            <div className="h-2 bg-gray-700 rounded-full overflow-hidden">
              <div
                className="h-full bg-blue-500 rounded-full transition-all duration-300"
                style={{ width: `${computingProgress.progress}%` }}
              />
            </div>
          </div>
        )}

        {/* Error message */}
        {config.error && (
          <div className="mt-3 p-2 rounded bg-red-900/30 border border-red-500/30">
            <div className="flex items-start gap-2">
              <AlertCircle className="w-4 h-4 text-red-400 mt-0.5 flex-shrink-0" />
              <p className="text-sm text-red-300">{config.error}</p>
            </div>
          </div>
        )}

        {/* Stale notice */}
        {config.status === 'stale' && (
          <div className="mt-3 flex items-center gap-2 text-sm text-yellow-400">
            <Clock className="w-4 h-4" />
            <span>Will recompute at next tick</span>
          </div>
        )}

        {/* Actions row */}
        <div className="flex items-center justify-between mt-4 pt-3 border-t border-white/10">
          <div className="flex items-center gap-2">
            <button
              onClick={() => onRecompute(config.id)}
              disabled={isComputing || isActionPending}
              className={`
                flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm font-medium
                transition-all
                ${isComputing || isActionPending
                  ? 'bg-gray-700 text-gray-500 cursor-not-allowed'
                  : 'bg-purple-600 hover:bg-purple-500 text-white'
                }
              `}
            >
              <Play className="w-4 h-4" />
              Recompute Now
            </button>

            <button
              onClick={() => onMarkStale(config.id)}
              disabled={isComputing || config.status === 'stale' || isActionPending}
              className={`
                flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-sm font-medium
                transition-all
                ${isComputing || config.status === 'stale' || isActionPending
                  ? 'bg-gray-700/50 text-gray-500 cursor-not-allowed'
                  : 'bg-gray-700 hover:bg-gray-600 text-white'
                }
              `}
            >
              <RefreshCw className="w-4 h-4" />
              Mark Stale
            </button>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={() => onEdit(config.id)}
              disabled={isActionPending}
              className="px-3 py-1.5 rounded-lg text-sm font-medium bg-gray-700 hover:bg-gray-600 text-white transition-all"
            >
              Edit
            </button>

            <button
              onClick={() => onDelete(config.id)}
              disabled={isActionPending}
              className="p-1.5 rounded-lg text-gray-400 hover:text-red-400 hover:bg-red-900/30 transition-all"
              title="Delete config"
            >
              <Trash2 className="w-4 h-4" />
            </button>

            <button
              onClick={() => setIsExpanded(!isExpanded)}
              className="p-1.5 rounded-lg text-gray-400 hover:text-white hover:bg-white/10 transition-all"
              title={isExpanded ? 'Hide details' : 'Show details'}
            >
              {isExpanded ? (
                <ChevronUp className="w-4 h-4" />
              ) : (
                <ChevronDown className="w-4 h-4" />
              )}
            </button>
          </div>
        </div>
      </div>

      {/* Expanded details */}
      {isExpanded && (
        <div className="px-4 pb-4 pt-0 border-t border-white/10">
          <div className="grid grid-cols-2 gap-4 pt-4 text-sm">
            {/* Target */}
            <div>
              <h4 className="text-gray-400 font-medium mb-2">Target</h4>
              <div className="space-y-1">
                <p className="text-white">
                  Mode: <span className="text-gray-300">{config.config.target.mode}</span>
                </p>
                {config.config.target.branches && config.config.target.branches.length > 0 && (
                  <p className="text-white">
                    Branches: <span className="text-gray-300">{config.config.target.branches.join(', ')}</span>
                  </p>
                )}
                {config.config.target.branch_pattern && (
                  <p className="text-white">
                    Pattern: <span className="text-gray-300">{config.config.target.branch_pattern}</span>
                  </p>
                )}
              </div>
            </div>

            {/* Scope */}
            <div>
              <h4 className="text-gray-400 font-medium mb-2">Scope</h4>
              <div className="space-y-1">
                {config.config.scope.node_types && config.config.scope.node_types.length > 0 && (
                  <p className="text-white">
                    Types: <span className="text-gray-300">{config.config.scope.node_types.join(', ')}</span>
                  </p>
                )}
                {config.config.scope.relation_types && config.config.scope.relation_types.length > 0 && (
                  <p className="text-white">
                    Relations: <span className="text-gray-300">{config.config.scope.relation_types.join(', ')}</span>
                  </p>
                )}
                {config.config.scope.paths && config.config.scope.paths.length > 0 && (
                  <p className="text-white">
                    Paths: <span className="text-gray-300">{config.config.scope.paths.join(', ')}</span>
                  </p>
                )}
                {(!config.config.scope.node_types?.length &&
                  !config.config.scope.relation_types?.length &&
                  !config.config.scope.paths?.length) && (
                  <p className="text-gray-500 italic">All nodes</p>
                )}
              </div>
            </div>

            {/* Refresh */}
            <div>
              <h4 className="text-gray-400 font-medium mb-2">Refresh</h4>
              <div className="space-y-1">
                <p className="text-white">
                  TTL: <span className="text-gray-300">
                    {config.config.refresh.ttl_seconds > 0
                      ? `${config.config.refresh.ttl_seconds}s`
                      : 'Disabled'}
                  </span>
                </p>
                <p className="text-white">
                  On branch change: <span className="text-gray-300">
                    {config.config.refresh.on_branch_change ? 'Yes' : 'No'}
                  </span>
                </p>
                <p className="text-white">
                  On relation change: <span className="text-gray-300">
                    {config.config.refresh.on_relation_change ? 'Yes' : 'No'}
                  </span>
                </p>
              </div>
            </div>

            {/* Algorithm config */}
            <div>
              <h4 className="text-gray-400 font-medium mb-2">Algorithm Config</h4>
              <div className="space-y-1">
                {Object.entries(config.config.algorithm_config).map(([key, value]) => (
                  <p key={key} className="text-white">
                    {key}: <span className="text-gray-300">{String(value)}</span>
                  </p>
                ))}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
