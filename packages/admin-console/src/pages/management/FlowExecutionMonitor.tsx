/**
 * Flow Execution Monitor Page
 *
 * Visual observability for flow runtime executions showing:
 * - List of flow instances (running, waiting, completed, failed)
 * - Real-time status updates via SSE
 * - Step-by-step execution timeline
 * - Drill-down into individual flow instances
 * - Variables/context at each step
 * - Error display with compensation stack
 */

import { useEffect, useState, useMemo } from 'react'
import { useNavigate, useSearchParams } from 'react-router-dom'
import {
  Workflow,
  RefreshCw,
  Trash2,
  ChevronRight,
  X,
  Search,
  Clock,
  CheckCircle,
  AlertCircle,
  XCircle,
  Circle,
  Play,
  Pause,
  AlertTriangle
} from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import ConfirmDialog from '../../components/ConfirmDialog'
import { useToast, ToastContainer } from '../../components/Toast'
import FlowInstanceDetail from '../../components/management/FlowInstanceDetail'

// Flow instance node structure (from raisin:FlowInstance node type)
export interface FlowInstance {
  id: string
  name: string
  path: string
  status: 'pending' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled' | 'rolled_back'
  flow_ref: string
  flow_version: number
  current_node_id?: string
  started_at: string
  completed_at?: string
  error?: string
  variables: Record<string, unknown>
  input: Record<string, unknown>
  output?: Record<string, unknown>
  wait_info?: {
    subscription_id: string
    wait_type: 'tool_call' | 'human_task' | 'scheduled' | 'event'
    target_path?: string
    timeout_at?: string
  }
  compensation_stack?: CompensationEntry[]
  metrics?: {
    total_duration_ms: number
    step_count: number
    retry_count: number
    compensation_count: number
  }
}

export interface CompensationEntry {
  step_id: string
  completed_at: string
  compensation_fn: string
  compensation_input: Record<string, unknown>
  compensation_status: 'pending' | 'executed' | 'failed'
}

const STATUS_CONFIG = {
  pending: {
    icon: Circle,
    color: 'text-zinc-400',
    bg: 'bg-zinc-500/20',
    border: 'border-zinc-500/30',
    label: 'Pending'
  },
  running: {
    icon: Play,
    color: 'text-blue-400',
    bg: 'bg-blue-500/20',
    border: 'border-blue-500/30',
    label: 'Running'
  },
  waiting: {
    icon: Pause,
    color: 'text-yellow-400',
    bg: 'bg-yellow-500/20',
    border: 'border-yellow-500/30',
    label: 'Waiting'
  },
  completed: {
    icon: CheckCircle,
    color: 'text-green-400',
    bg: 'bg-green-500/20',
    border: 'border-green-500/30',
    label: 'Completed'
  },
  failed: {
    icon: XCircle,
    color: 'text-red-400',
    bg: 'bg-red-500/20',
    border: 'border-red-500/30',
    label: 'Failed'
  },
  cancelled: {
    icon: AlertCircle,
    color: 'text-orange-400',
    bg: 'bg-orange-500/20',
    border: 'border-orange-500/30',
    label: 'Cancelled'
  },
  rolled_back: {
    icon: AlertTriangle,
    color: 'text-purple-400',
    bg: 'bg-purple-500/20',
    border: 'border-purple-500/30',
    label: 'Rolled Back'
  },
}

const STATUS_OPTIONS = [
  { value: 'all', label: 'All Statuses' },
  { value: 'running', label: 'Running' },
  { value: 'waiting', label: 'Waiting' },
  { value: 'completed', label: 'Completed' },
  { value: 'failed', label: 'Failed' },
  { value: 'cancelled', label: 'Cancelled' },
  { value: 'rolled_back', label: 'Rolled Back' },
]

export default function FlowExecutionMonitor() {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const selectedInstanceId = searchParams.get('instance')

  const [instances, setInstances] = useState<FlowInstance[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)
  const [, setTick] = useState(0)
  const [clearConfirm, setClearConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, warning: showWarning, closeToast } = useToast()

  // Filter state
  const [statusFilter, setStatusFilter] = useState('all')
  const [pathFilter, setPathFilter] = useState('')

  // Apply filters
  const filteredInstances = useMemo(() => {
    return instances.filter(instance => {
      // Status filter
      if (statusFilter !== 'all' && instance.status !== statusFilter) return false

      // Path filter
      if (pathFilter) {
        const searchLower = pathFilter.toLowerCase()
        if (!instance.path.toLowerCase().includes(searchLower) &&
            !instance.flow_ref.toLowerCase().includes(searchLower)) {
          return false
        }
      }

      return true
    })
  }, [instances, statusFilter, pathFilter])

  const hasActiveFilters = statusFilter !== 'all' || pathFilter !== ''

  const clearFilters = () => {
    setStatusFilter('all')
    setPathFilter('')
  }

  // Update timers for duration display
  useEffect(() => {
    const interval = setInterval(() => setTick(prev => prev + 1), 1000)
    return () => clearInterval(interval)
  }, [])

  // Fetch flow instances from flows workspace
  useEffect(() => {
    const fetchInstances = async () => {
      try {
        // TODO: Replace with actual API call to query flows workspace
        // This will query for raisin:FlowInstance nodes
        // const response = await nodesApi.query({
        //   workspace: 'flows',
        //   node_type: 'raisin:FlowInstance',
        //   include_children: true
        // })

        // Mock data for now
        setInstances([])
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch flow instances')
      } finally {
        setLoading(false)
      }
    }
    fetchInstances()
  }, [])

  // SSE connection for real-time updates
  useEffect(() => {
    // TODO: Connect to SSE endpoint for flow instance updates
    // Similar to the job updates in ExecutionLogs
    setConnected(true)

    return () => {
      setConnected(false)
    }
  }, [])

  const handleDeleteInstance = async (instanceId: string) => {
    try {
      // TODO: Implement delete via nodes API
      setInstances(prev => prev.filter(i => i.id !== instanceId))
    } catch (err) {
      showError('Error', `Failed to delete: ${err instanceof Error ? err.message : 'Unknown error'}`)
    }
  }

  const handleClearCompleted = async () => {
    const completedInstances = filteredInstances.filter(
      i => i.status === 'completed' || i.status === 'cancelled' || i.status === 'failed'
    )

    if (completedInstances.length === 0) {
      showWarning('No Instances', 'No completed flow instances to clear')
      return
    }

    setClearConfirm({
      message: `Delete ${completedInstances.length} completed flow instance(s)?`,
      onConfirm: async () => {
        for (const instance of completedInstances) {
          await handleDeleteInstance(instance.id)
        }
      }
    })
  }

  const handleSelectInstance = (instanceId: string) => {
    navigate(`?instance=${instanceId}`)
  }

  const handleCloseDetail = () => {
    navigate('')
  }

  const formatDuration = (startedAt: string, completedAt?: string): string => {
    const start = new Date(startedAt).getTime()
    const end = completedAt ? new Date(completedAt).getTime() : Date.now()
    const ms = end - start

    if (ms < 1000) return `${ms}ms`
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
    if (ms < 3600000) return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`
    return `${Math.floor(ms / 3600000)}h ${Math.floor((ms % 3600000) / 60000)}m`
  }

  if (loading) {
    return (
      <div className="p-8">
        <div className="animate-pulse">
          <div className="h-8 bg-white/10 rounded w-48 mb-8"></div>
          <div className="space-y-4">
            {[1, 2, 3].map((i) => (
              <div key={i} className="h-24 bg-white/5 rounded-xl"></div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-8">
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4 text-red-300">
          {error}
        </div>
      </div>
    )
  }

  // Show detail view if instance is selected
  const selectedInstance = instances.find(i => i.id === selectedInstanceId)
  if (selectedInstance) {
    return (
      <FlowInstanceDetail
        instance={selectedInstance}
        onClose={handleCloseDetail}
      />
    )
  }

  return (
    <div className="p-8 max-w-7xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-3xl font-bold text-white mb-2 flex items-center gap-3">
            <Workflow className="w-8 h-8 text-purple-400" />
            Flow Execution Monitor
          </h1>
          <p className="text-zinc-400">Real-time observability for flow runtime instances</p>
        </div>
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'} animate-pulse`}></div>
            <span className="text-sm text-zinc-400">{connected ? 'Live' : 'Disconnected'}</span>
          </div>
          {filteredInstances.some(i =>
            i.status === 'completed' || i.status === 'cancelled' || i.status === 'failed'
          ) && (
            <button
              onClick={handleClearCompleted}
              className="px-4 py-2 bg-zinc-700/50 hover:bg-zinc-700/70 border border-zinc-600 rounded-lg text-zinc-300 text-sm flex items-center gap-2 transition-colors"
            >
              <Trash2 className="w-4 h-4" />
              Clear Completed
            </button>
          )}
        </div>
      </div>

      {/* Filter Bar */}
      <div className="mb-6 p-4 bg-white/5 border border-white/10 rounded-lg">
        <div className="flex flex-wrap items-center gap-4">
          {/* Status Filter */}
          <div className="flex flex-col gap-1">
            <label className="text-xs text-zinc-500">Status</label>
            <select
              value={statusFilter}
              onChange={(e) => setStatusFilter(e.target.value)}
              className="px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 focus:outline-none focus:border-purple-500 focus:ring-2 focus:ring-purple-500/20"
            >
              {STATUS_OPTIONS.map(opt => (
                <option key={opt.value} value={opt.value}>{opt.label}</option>
              ))}
            </select>
          </div>

          {/* Path Filter */}
          <div className="flex flex-col gap-1 flex-1 min-w-[200px]">
            <label className="text-xs text-zinc-500">Search Path / Flow</label>
            <div className="relative">
              <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-500" />
              <input
                type="text"
                value={pathFilter}
                onChange={(e) => setPathFilter(e.target.value)}
                placeholder="e.g., /flows/instances/schedule-meeting"
                className="w-full pl-10 pr-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 placeholder-zinc-500 focus:outline-none focus:border-purple-500 focus:ring-2 focus:ring-purple-500/20"
              />
            </div>
          </div>

          {/* Clear Filters */}
          {hasActiveFilters && (
            <button
              onClick={clearFilters}
              className="px-3 py-2 mt-5 text-sm text-zinc-400 hover:text-white flex items-center gap-1 transition-colors"
            >
              <X className="w-4 h-4" />
              Clear
            </button>
          )}
        </div>

        {/* Results count */}
        <div className="mt-3 text-xs text-zinc-500">
          Showing {filteredInstances.length} of {instances.length} flow instances
        </div>
      </div>

      {/* Flow Instance List */}
      {filteredInstances.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <Workflow className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <p className="text-zinc-400">
              {instances.length === 0 ? 'No flow instances yet' : 'No instances match the current filters'}
            </p>
            {hasActiveFilters && instances.length > 0 && (
              <button onClick={clearFilters} className="mt-4 px-4 py-2 text-sm text-purple-400 hover:text-purple-300 transition-colors">
                Clear filters
              </button>
            )}
          </div>
        </GlassCard>
      ) : (
        <div className="space-y-3">
          {filteredInstances.map((instance) => {
            const statusConfig = STATUS_CONFIG[instance.status] || STATUS_CONFIG.pending
            const StatusIcon = statusConfig.icon
            const isActive = instance.status === 'running' || instance.status === 'waiting'
            const isCompleted = instance.status === 'completed' || instance.status === 'cancelled' || instance.status === 'failed'

            return (
              <div
                key={instance.id}
                className={`bg-white/5 border rounded-xl overflow-hidden transition-all hover:bg-white/10 cursor-pointer ${statusConfig.border}`}
                onClick={() => handleSelectInstance(instance.id)}
              >
                <div className="flex items-center gap-3 p-4">
                  {/* Status Icon */}
                  <StatusIcon className={`w-5 h-5 ${statusConfig.color} flex-shrink-0 ${
                    instance.status === 'running' ? 'animate-spin' : ''
                  }`} />

                  {/* Instance Info */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-medium text-white truncate">
                        {instance.name || instance.path}
                      </span>
                      <span className={`px-2 py-0.5 text-xs rounded ${statusConfig.bg} ${statusConfig.color}`}>
                        {statusConfig.label}
                      </span>
                      {instance.wait_info && (
                        <span className="px-2 py-0.5 bg-yellow-500/20 text-yellow-300 text-xs rounded">
                          Waiting: {instance.wait_info.wait_type}
                        </span>
                      )}
                      {instance.compensation_stack && instance.compensation_stack.length > 0 && (
                        <span className="px-2 py-0.5 bg-purple-500/20 text-purple-300 text-xs rounded">
                          {instance.compensation_stack.length} compensations
                        </span>
                      )}
                    </div>
                    <div className="text-xs text-zinc-500 mt-1 flex items-center gap-3 flex-wrap">
                      <span>Flow: {instance.flow_ref}</span>
                      <span className="flex items-center gap-1">
                        <Clock className="w-3 h-3" />
                        {new Date(instance.started_at).toLocaleString()}
                      </span>
                      <span>
                        Duration: {formatDuration(instance.started_at, instance.completed_at)}
                      </span>
                      {instance.metrics && (
                        <span>{instance.metrics.step_count} steps</span>
                      )}
                    </div>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                    {isActive && (
                      <RefreshCw className="w-4 h-4 text-blue-400 animate-spin" />
                    )}
                    {isCompleted && (
                      <button
                        onClick={() => handleDeleteInstance(instance.id)}
                        className="p-2 text-zinc-400 hover:text-red-400 hover:bg-red-500/10 rounded-lg transition-colors"
                        title="Delete instance"
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    )}
                    <ChevronRight className="w-5 h-5 text-zinc-400" />
                  </div>
                </div>

                {/* Error Preview */}
                {instance.error && (
                  <div className="px-4 pb-3 border-t border-white/5">
                    <div className="text-sm text-red-300 bg-red-500/10 rounded p-2 truncate mt-2">
                      {instance.error}
                    </div>
                  </div>
                )}
              </div>
            )
          })}
        </div>
      )}

      <ConfirmDialog
        open={clearConfirm !== null}
        title="Clear Flow Instances"
        message={clearConfirm?.message || ''}
        variant="danger"
        confirmText="Delete All"
        onConfirm={() => {
          clearConfirm?.onConfirm()
          setClearConfirm(null)
        }}
        onCancel={() => setClearConfirm(null)}
      />
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
