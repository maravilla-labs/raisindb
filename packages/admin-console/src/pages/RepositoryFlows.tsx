/**
 * Repository Flows Page
 *
 * Shows flow instances stored in the raisin:system workspace.
 * Features:
 * - List of flow instances (running, waiting, completed, failed)
 * - Real-time status updates via SSE
 * - Expandable rows with step timeline
 * - Filter by status and search by path
 */

import { useEffect, useState, useMemo } from 'react'
import { useParams } from 'react-router-dom'
import {
  Workflow,
  Trash2,
  ChevronDown,
  ChevronRight,
  Search,
  X,
  CheckCircle,
  XCircle,
  Filter,
  Circle,
  Play,
  Pause,
  AlertCircle,
  AlertTriangle,
  StopCircle,
} from 'lucide-react'
import { nodesApi, type Node } from '../api/nodes'
import { cancelFlowInstance, deleteFlowInstance } from '../api/flows'
import { sseManager } from '../api/management'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'

// Flow instance interface matching raisin:FlowInstance node properties
interface FlowInstance {
  id: string
  flow_ref: string
  flow_version: number
  status: 'pending' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled' | 'rolled_back'
  current_node_id?: string
  started_at?: string
  completed_at?: string
  error?: string
  input?: Record<string, unknown>
  output?: Record<string, unknown>
  variables?: Record<string, unknown>
  wait_info?: {
    wait_type: string
    reason: string
  }
  context?: {
    compensation_stack?: unknown[]
  }
}

// Status configuration for visual styling
const STATUS_CONFIG: Record<FlowInstance['status'], {
  icon: typeof Circle
  color: string
  bg: string
  animate: boolean
  label: string
}> = {
  pending: { icon: Circle, color: 'text-zinc-400', bg: 'bg-zinc-500/10', animate: false, label: 'Pending' },
  running: { icon: Play, color: 'text-blue-400', bg: 'bg-blue-500/10', animate: true, label: 'Running' },
  waiting: { icon: Pause, color: 'text-yellow-400', bg: 'bg-yellow-500/10', animate: false, label: 'Waiting' },
  completed: { icon: CheckCircle, color: 'text-green-400', bg: 'bg-green-500/10', animate: false, label: 'Completed' },
  failed: { icon: XCircle, color: 'text-red-400', bg: 'bg-red-500/10', animate: false, label: 'Failed' },
  cancelled: { icon: AlertCircle, color: 'text-orange-400', bg: 'bg-orange-500/10', animate: false, label: 'Cancelled' },
  rolled_back: { icon: AlertTriangle, color: 'text-purple-400', bg: 'bg-purple-500/10', animate: false, label: 'Rolled Back' },
}

type FilterStatus = 'all' | 'running' | 'waiting' | 'completed' | 'failed'

const FLOWS_WORKSPACE = 'raisin:system'
const INSTANCES_PATH = '/flows/instances'

export default function RepositoryFlows() {
  const { repo } = useParams<{ repo: string }>()
  const [instances, setInstances] = useState<FlowInstance[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)
  const [expandedInstanceId, setExpandedInstanceId] = useState<string | null>(null)
  const [, setTick] = useState(0)
  const [clearConfirm, setClearConfirm] = useState<{ title: string; message: string; confirmText: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, warning: showWarning, closeToast } = useToast()

  // Filter state
  const [statusFilter, setStatusFilter] = useState<FilterStatus>('all')
  const [searchFilter, setSearchFilter] = useState('')
  const [showFilters, setShowFilters] = useState(false)

  // Apply filters
  const filteredInstances = useMemo(() => {
    return instances.filter(instance => {
      // Status filter
      if (statusFilter !== 'all') {
        if (statusFilter === 'running' && instance.status !== 'running') return false
        if (statusFilter === 'waiting' && instance.status !== 'waiting') return false
        if (statusFilter === 'completed' && instance.status !== 'completed') return false
        if (statusFilter === 'failed' && instance.status !== 'failed' && instance.status !== 'cancelled') return false
      }

      // Search filter
      if (searchFilter) {
        const searchLower = searchFilter.toLowerCase()
        if (!instance.flow_ref.toLowerCase().includes(searchLower) &&
            !instance.id.toLowerCase().includes(searchLower)) {
          return false
        }
      }

      return true
    })
  }, [instances, statusFilter, searchFilter])

  const hasActiveFilters = statusFilter !== 'all' || searchFilter !== ''

  const clearFilters = () => {
    setStatusFilter('all')
    setSearchFilter('')
  }

  // Update timer for duration display
  useEffect(() => {
    const interval = setInterval(() => setTick(prev => prev + 1), 1000)
    return () => clearInterval(interval)
  }, [])

  // Helper to parse node into FlowInstance with status derivation fallback
  const parseFlowInstance = (node: Node): FlowInstance => {
    const props = node.properties || {}

    // Read stored status
    let status = (props.status as FlowInstance['status']) || 'pending'

    // FRONTEND FALLBACK: Derive failed status from __function_result
    // This handles race condition where status is "running" but function already failed
    const funcResult = props.__function_result as { success?: boolean; error?: string } | undefined
    if (funcResult && funcResult.success === false && status === 'running') {
      status = 'failed'  // Override - function failed but status not updated yet
    }

    return {
      id: node.id,
      flow_ref: props.flow_ref as string || '',
      flow_version: props.flow_version as number || 1,
      status,
      current_node_id: props.current_node_id as string,
      started_at: props.started_at as string,
      completed_at: props.completed_at as string,
      error: funcResult?.error || props.error as string,
      input: props.input as Record<string, unknown>,
      output: props.output as Record<string, unknown>,
      variables: props.variables as Record<string, unknown>,
      wait_info: props.wait_info as FlowInstance['wait_info'],
      context: props.context as FlowInstance['context'],
    }
  }

  // Fetch flow instances from raisin:system workspace
  const fetchInstances = async () => {
    if (!repo) return

    try {
      // List children of /flows/instances in raisin:system workspace
      const nodes = await nodesApi.listChildrenAtHead(repo, 'main', FLOWS_WORKSPACE, INSTANCES_PATH)

      // Convert Node[] to FlowInstance[] with status derivation
      const flowInstances: FlowInstance[] = nodes.map(parseFlowInstance)

      // Sort by started_at descending (newest first)
      flowInstances.sort((a, b) => {
        const aTime = a.started_at ? new Date(a.started_at).getTime() : 0
        const bTime = b.started_at ? new Date(b.started_at).getTime() : 0
        return bTime - aTime
      })

      setInstances(flowInstances)
    } catch (err) {
      // If path doesn't exist, it's not an error - just no instances
      if (err instanceof Error && err.message.includes('Not found')) {
        setInstances([])
      } else {
        setError(err instanceof Error ? err.message : 'Failed to fetch flow instances')
      }
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    fetchInstances()
  }, [repo])

  // SSE connection for real-time updates via jobs endpoint
  useEffect(() => {
    if (!repo) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event) => {
        // Filter for flow-related jobs (FlowInstanceExecution)
        if (event.job_type === 'FlowInstanceExecution') {
          // Refresh instances when flow jobs complete or fail
          if (event.status === 'completed' || event.status === 'failed') {
            fetchInstances()
          }
        }
      },
      onOpen: () => setConnected(true),
      onError: () => setConnected(false),
    })

    return cleanup
  }, [repo])

  const handleCancelInstance = (instanceId: string) => {
    setClearConfirm({
      title: 'Cancel Flow Instance',
      message: 'Cancel this flow instance? This cannot be undone.',
      confirmText: 'Cancel Instance',
      onConfirm: async () => {
        if (!repo) return
        try {
          await cancelFlowInstance(repo, instanceId)
          setInstances(prev => prev.map(i =>
            i.id === instanceId
              ? { ...i, status: 'cancelled' as const, completed_at: new Date().toISOString(), wait_info: undefined }
              : i
          ))
        } catch (err) {
          showError('Error', `Failed to cancel: ${err instanceof Error ? err.message : 'Unknown error'}`)
        }
      }
    })
  }

  const handleDeleteInstance = async (instanceId: string) => {
    if (!repo) return

    try {
      await deleteFlowInstance(repo, instanceId)
      setInstances(prev => prev.filter(i => i.id !== instanceId))
      if (expandedInstanceId === instanceId) setExpandedInstanceId(null)
    } catch (err) {
      showError('Error', `Failed to delete: ${err instanceof Error ? err.message : 'Unknown error'}`)
    }
  }

  const handleClearCompleted = async () => {
    const completedInstances = filteredInstances.filter(
      i => i.status === 'completed' || i.status === 'cancelled' || i.status === 'failed' || i.status === 'rolled_back'
    )

    if (completedInstances.length === 0) {
      showWarning('No Instances', 'No completed flow instances to clear')
      return
    }

    setClearConfirm({
      title: 'Clear Flow Instances',
      message: `Delete ${completedInstances.length} completed flow instance(s)?`,
      confirmText: 'Delete All',
      onConfirm: async () => {
        for (const instance of completedInstances) {
          await handleDeleteInstance(instance.id)
        }
      }
    })
  }

  const getStatusInfo = (status: FlowInstance['status']) => {
    return STATUS_CONFIG[status] || STATUS_CONFIG.pending
  }

  const isInstanceCompleted = (instance: FlowInstance): boolean => {
    return instance.status === 'completed' || instance.status === 'cancelled' ||
           instance.status === 'failed' || instance.status === 'rolled_back'
  }

  const formatDuration = (ms: number): string => {
    if (ms < 1000) return `${ms}ms`
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
    return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`
  }

  const getDuration = (instance: FlowInstance): string => {
    if (!instance.started_at) return '-'
    const start = new Date(instance.started_at).getTime()
    const end = instance.completed_at ? new Date(instance.completed_at).getTime() : Date.now()
    return formatDuration(end - start)
  }

  if (loading) {
    return (
      <div className="p-8">
        <div className="animate-pulse space-y-4">
          <div className="h-8 bg-white/10 rounded w-64"></div>
          <div className="h-12 bg-white/5 rounded"></div>
          <div className="space-y-2">
            {[1, 2, 3, 4, 5].map((i) => (
              <div key={i} className="h-14 bg-white/5 rounded"></div>
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

  return (
    <div className="p-6 md:p-8 max-w-[1600px] mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-2xl font-bold text-white flex items-center gap-3">
            <Workflow className="w-7 h-7 text-purple-400" />
            Flow Instances
          </h1>
          <p className="text-zinc-400 text-sm mt-1">Running and completed flow executions</p>
        </div>
        <div className="flex items-center gap-3">
          {/* Connection Status */}
          <div className="flex items-center gap-2 px-3 py-1.5 bg-white/5 rounded-lg">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'} animate-pulse`}></div>
            <span className="text-xs text-zinc-400">{connected ? 'Live' : 'Offline'}</span>
          </div>

          {/* Filter Toggle */}
          <button
            onClick={() => setShowFilters(!showFilters)}
            className={`p-2 rounded-lg transition-colors ${
              hasActiveFilters || showFilters
                ? 'bg-purple-500/20 text-purple-400'
                : 'bg-white/5 text-zinc-400 hover:text-white'
            }`}
          >
            <Filter className="w-5 h-5" />
          </button>

          {/* Clear Completed */}
          {filteredInstances.some(isInstanceCompleted) && (
            <button
              onClick={handleClearCompleted}
              className="px-3 py-2 bg-white/5 hover:bg-white/10 rounded-lg text-zinc-400 hover:text-white text-sm flex items-center gap-2 transition-colors"
            >
              <Trash2 className="w-4 h-4" />
              Clear
            </button>
          )}
        </div>
      </div>

      {/* Filter Bar */}
      {showFilters && (
        <div className="mb-4 p-4 bg-white/5 border border-white/10 rounded-xl">
          <div className="flex flex-wrap items-end gap-4">
            {/* Status Filter */}
            <div>
              <label className="block text-xs text-zinc-500 mb-1.5">Status</label>
              <div className="flex gap-1">
                {(['all', 'running', 'waiting', 'completed', 'failed'] as FilterStatus[]).map((status) => (
                  <button
                    key={status}
                    onClick={() => setStatusFilter(status)}
                    className={`px-3 py-1.5 text-sm rounded-lg transition-colors capitalize ${
                      statusFilter === status
                        ? 'bg-purple-500 text-white'
                        : 'bg-white/5 text-zinc-400 hover:text-white'
                    }`}
                  >
                    {status}
                  </button>
                ))}
              </div>
            </div>

            {/* Search */}
            <div className="flex-1 min-w-[200px]">
              <label className="block text-xs text-zinc-500 mb-1.5">Search</label>
              <div className="relative">
                <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-500" />
                <input
                  type="text"
                  value={searchFilter}
                  onChange={(e) => setSearchFilter(e.target.value)}
                  placeholder="Search by flow path or instance ID..."
                  className="w-full pl-10 pr-3 py-1.5 bg-white/5 border border-white/10 rounded-lg text-sm text-zinc-300 placeholder-zinc-500 focus:outline-none focus:border-purple-500"
                />
              </div>
            </div>

            {/* Clear */}
            {hasActiveFilters && (
              <button
                onClick={clearFilters}
                className="px-3 py-1.5 text-sm text-zinc-400 hover:text-white flex items-center gap-1"
              >
                <X className="w-4 h-4" />
                Clear filters
              </button>
            )}
          </div>
        </div>
      )}

      {/* Results Count */}
      <div className="text-xs text-zinc-500 mb-3">
        {filteredInstances.length} instance{filteredInstances.length !== 1 ? 's' : ''}
        {hasActiveFilters && ` (filtered from ${instances.length})`}
      </div>

      {/* Table */}
      {filteredInstances.length === 0 ? (
        <div className="bg-white/5 border border-white/10 rounded-xl p-12 text-center">
          <Workflow className="w-12 h-12 text-zinc-600 mx-auto mb-4" />
          <p className="text-zinc-400">
            {instances.length === 0 ? 'No flow instances yet' : 'No instances match filters'}
          </p>
          {hasActiveFilters && (
            <button onClick={clearFilters} className="mt-3 text-sm text-purple-400 hover:text-purple-300">
              Clear filters
            </button>
          )}
        </div>
      ) : (
        <div className="bg-white/5 border border-white/10 rounded-xl overflow-hidden">
          {/* Table Header */}
          <div className="grid grid-cols-[auto_1fr_120px_100px_100px_80px] gap-4 px-4 py-3 bg-white/5 border-b border-white/10 text-xs text-zinc-500 font-medium uppercase tracking-wider">
            <div className="w-6"></div>
            <div>Flow</div>
            <div>Status</div>
            <div>Duration</div>
            <div>Started</div>
            <div></div>
          </div>

          {/* Table Body */}
          <div className="divide-y divide-white/5">
            {filteredInstances.map((instance) => {
              const isExpanded = expandedInstanceId === instance.id
              const statusInfo = getStatusInfo(instance.status)
              const StatusIcon = statusInfo.icon
              const isFailed = instance.status === 'failed'

              return (
                <div key={instance.id}>
                  {/* Row */}
                  <div
                    className={`grid grid-cols-[auto_1fr_120px_100px_100px_80px] gap-4 px-4 py-3 items-center hover:bg-white/5 cursor-pointer transition-colors ${
                      isFailed ? 'bg-red-500/5' : ''
                    }`}
                    onClick={() => setExpandedInstanceId(isExpanded ? null : instance.id)}
                  >
                    {/* Expand Icon */}
                    <div className="text-zinc-500">
                      {isExpanded ? (
                        <ChevronDown className="w-5 h-5" />
                      ) : (
                        <ChevronRight className="w-5 h-5" />
                      )}
                    </div>

                    {/* Flow Info */}
                    <div className="flex items-center gap-3 min-w-0">
                      <div className="p-1.5 rounded-lg bg-purple-500/20">
                        <Workflow className="w-4 h-4 text-purple-400" />
                      </div>
                      <div className="min-w-0">
                        <div className="text-sm text-white font-medium truncate" title={instance.flow_ref}>
                          {instance.flow_ref}
                        </div>
                        <div className="text-xs text-zinc-500 truncate">
                          {instance.id.slice(0, 8)}...
                          {instance.current_node_id && (
                            <span className="ml-2 text-zinc-400">@ {instance.current_node_id}</span>
                          )}
                        </div>
                      </div>
                    </div>

                    {/* Status */}
                    <div>
                      <span className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${statusInfo.bg} ${statusInfo.color}`}>
                        <StatusIcon className={`w-3 h-3 ${statusInfo.animate ? 'animate-spin' : ''}`} />
                        {statusInfo.label}
                      </span>
                    </div>

                    {/* Duration */}
                    <div className="text-sm text-zinc-400">
                      {instance.status === 'running' ? (
                        <span className="text-blue-400">{getDuration(instance)}</span>
                      ) : (
                        getDuration(instance)
                      )}
                    </div>

                    {/* Started */}
                    <div className="text-xs text-zinc-500">
                      {instance.started_at ? new Date(instance.started_at).toLocaleTimeString() : '-'}
                    </div>

                    {/* Actions */}
                    <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
                      {!isInstanceCompleted(instance) && (
                        <button
                          onClick={() => handleCancelInstance(instance.id)}
                          className="p-1.5 text-zinc-500 hover:text-red-400 hover:bg-red-500/10 rounded transition-colors"
                          title="Cancel"
                        >
                          <StopCircle className="w-4 h-4" />
                        </button>
                      )}
                      {isInstanceCompleted(instance) && (
                        <button
                          onClick={() => handleDeleteInstance(instance.id)}
                          className="p-1.5 text-zinc-500 hover:text-red-400 hover:bg-red-500/10 rounded transition-colors"
                          title="Delete"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      )}
                    </div>
                  </div>

                  {/* Expanded Content */}
                  {isExpanded && (
                    <div className="px-4 pb-4 bg-black/20 border-t border-white/5">
                      <div className="pt-4 pl-9">
                        {/* Error Display */}
                        {instance.error && (
                          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
                            <div className="text-sm font-medium text-red-300 mb-1">Error</div>
                            <div className="text-sm text-red-200/80 font-mono whitespace-pre-wrap">{instance.error}</div>
                          </div>
                        )}

                        {/* Wait Info */}
                        {instance.wait_info && (
                          <div className="mb-4 p-3 bg-yellow-500/10 border border-yellow-500/20 rounded-lg">
                            <div className="text-sm font-medium text-yellow-300 mb-1">Waiting: {instance.wait_info.wait_type}</div>
                            <div className="text-sm text-yellow-200/80">{instance.wait_info.reason}</div>
                          </div>
                        )}

                        {/* Instance Details */}
                        <div className="grid grid-cols-2 gap-4 mb-4">
                          <div>
                            <div className="text-xs text-zinc-500 mb-1">Instance ID</div>
                            <div className="text-sm text-zinc-300 font-mono">{instance.id}</div>
                          </div>
                          <div>
                            <div className="text-xs text-zinc-500 mb-1">Flow Version</div>
                            <div className="text-sm text-zinc-300">v{instance.flow_version}</div>
                          </div>
                        </div>

                        {/* Input */}
                        {instance.input && Object.keys(instance.input).length > 0 && (
                          <div className="mb-4">
                            <div className="text-xs text-zinc-500 mb-2">Input</div>
                            <pre className="text-xs text-zinc-400 bg-black/30 p-3 rounded-lg overflow-auto max-h-32">
                              {JSON.stringify(instance.input, null, 2)}
                            </pre>
                          </div>
                        )}

                        {/* Output */}
                        {instance.output && Object.keys(instance.output).length > 0 && (
                          <div className="mb-4">
                            <div className="text-xs text-zinc-500 mb-2">Output</div>
                            <pre className="text-xs text-zinc-400 bg-black/30 p-3 rounded-lg overflow-auto max-h-32">
                              {JSON.stringify(instance.output, null, 2)}
                            </pre>
                          </div>
                        )}

                        {/* Variables */}
                        {instance.variables && Object.keys(instance.variables).length > 0 && (
                          <div>
                            <div className="text-xs text-zinc-500 mb-2">Variables</div>
                            <pre className="text-xs text-zinc-400 bg-black/30 p-3 rounded-lg overflow-auto max-h-32">
                              {JSON.stringify(instance.variables, null, 2)}
                            </pre>
                          </div>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}

      <ConfirmDialog
        open={clearConfirm !== null}
        title={clearConfirm?.title || 'Confirm'}
        message={clearConfirm?.message || ''}
        variant="danger"
        confirmText={clearConfirm?.confirmText || 'Confirm'}
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
