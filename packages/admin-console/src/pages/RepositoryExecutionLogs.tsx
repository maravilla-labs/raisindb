/**
 * Repository Execution Logs Page
 *
 * Modern table-based view for function and flow execution logs at the repository level.
 * Features expandable rows for detailed log viewing.
 */

import { useEffect, useState, useMemo } from 'react'
import { useParams } from 'react-router-dom'
import {
  Trash2,
  ChevronDown,
  ChevronRight,
  Terminal,
  Workflow,
  Code,
  Search,
  X,
  CheckCircle,
  XCircle,
  Clock,
  Loader2,
  Filter
} from 'lucide-react'
import FunctionExecutionCard from '../components/management/FunctionExecutionCard'
import FlowExecutionCard from '../components/management/FlowExecutionCard'
import { managementApi, JobInfo, JobEvent, sseManager, formatDuration } from '../api/management'
import ConfirmDialog from '../components/ConfirmDialog'
import { useToast, ToastContainer } from '../components/Toast'

// Status configuration
const STATUS_CONFIG = {
  Running: { icon: Loader2, color: 'text-blue-400', bg: 'bg-blue-500/10', animate: true },
  Scheduled: { icon: Clock, color: 'text-yellow-400', bg: 'bg-yellow-500/10', animate: false },
  Completed: { icon: CheckCircle, color: 'text-green-400', bg: 'bg-green-500/10', animate: false },
  Cancelled: { icon: XCircle, color: 'text-zinc-400', bg: 'bg-zinc-500/10', animate: false },
  Failed: { icon: XCircle, color: 'text-red-400', bg: 'bg-red-500/10', animate: false },
}

type FilterType = 'all' | 'function' | 'flow'
type FilterStatus = 'all' | 'running' | 'completed' | 'failed'

export default function RepositoryExecutionLogs() {
  const { repo: _repo } = useParams<{ repo: string }>() // Will be used for filtering when backend supports it
  const [jobs, setJobs] = useState<JobInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null)
  const [, setTick] = useState(0)
  const [clearConfirm, setClearConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, warning: showWarning, closeToast } = useToast()

  // Filter state
  const [typeFilter, setTypeFilter] = useState<FilterType>('all')
  const [statusFilter, setStatusFilter] = useState<FilterStatus>('all')
  const [searchFilter, setSearchFilter] = useState('')
  const [showFilters, setShowFilters] = useState(false)

  // Filter to only show FunctionExecution and FlowExecution jobs for this repo
  const executionJobs = useMemo(() => {
    return jobs.filter(job => {
      const jobTypeStr = typeof job.job_type === 'string' ? job.job_type : JSON.stringify(job.job_type)
      const isExecution = jobTypeStr.startsWith('FunctionExecution') || jobTypeStr.startsWith('FlowExecution')

      // TODO: Filter by repository when job context includes repo info
      // For now show all executions
      return isExecution
    })
  }, [jobs])

  // Apply additional filters
  const filteredJobs = useMemo(() => {
    return executionJobs.filter(job => {
      const jobTypeStr = typeof job.job_type === 'string' ? job.job_type : JSON.stringify(job.job_type)

      // Type filter
      if (typeFilter !== 'all') {
        if (typeFilter === 'function' && !jobTypeStr.startsWith('FunctionExecution')) return false
        if (typeFilter === 'flow' && !jobTypeStr.startsWith('FlowExecution')) return false
      }

      // Status filter
      if (statusFilter !== 'all') {
        const statusStr = typeof job.status === 'string' ? job.status.toLowerCase() : 'failed'
        if (statusFilter === 'running' && statusStr !== 'running') return false
        if (statusFilter === 'completed' && statusStr !== 'completed') return false
        if (statusFilter === 'failed' && typeof job.status !== 'object') return false
      }

      // Search filter
      if (searchFilter) {
        const searchLower = searchFilter.toLowerCase()
        if (!jobTypeStr.toLowerCase().includes(searchLower)) return false
      }

      return true
    })
  }, [executionJobs, typeFilter, statusFilter, searchFilter])

  const hasActiveFilters = typeFilter !== 'all' || statusFilter !== 'all' || searchFilter !== ''

  const clearFilters = () => {
    setTypeFilter('all')
    setStatusFilter('all')
    setSearchFilter('')
  }

  // Update timers
  useEffect(() => {
    const interval = setInterval(() => setTick(prev => prev + 1), 1000)
    return () => clearInterval(interval)
  }, [])

  // Fetch initial jobs
  useEffect(() => {
    const fetchJobs = async () => {
      try {
        const response = await managementApi.listJobs()
        if (response.success && response.data) {
          setJobs(response.data)
        } else {
          setError(response.error || 'Failed to fetch jobs')
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to fetch jobs')
      } finally {
        setLoading(false)
      }
    }
    fetchJobs()
  }, [])

  // SSE connection
  useEffect(() => {
    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        setJobs((prevJobs) => {
          const existingIndex = prevJobs.findIndex((j) => j.id === event.job_id)

          if (existingIndex >= 0) {
            const updated = [...prevJobs]
            updated[existingIndex] = {
              ...updated[existingIndex],
              status: event.status as any,
              progress: event.progress,
              error: event.error,
              retry_count: event.retry_count ?? updated[existingIndex].retry_count,
              max_retries: event.max_retries ?? updated[existingIndex].max_retries,
              last_heartbeat: event.last_heartbeat ?? updated[existingIndex].last_heartbeat,
              timeout_seconds: event.timeout_seconds ?? updated[existingIndex].timeout_seconds,
              next_retry_at: event.next_retry_at ?? updated[existingIndex].next_retry_at,
              result: event.function_result ?? updated[existingIndex].result,
            }
            return updated
          } else {
            const newJob: JobInfo = {
              id: event.job_id,
              job_type: event.job_type as any,
              status: event.status as any,
              tenant: event.tenant,
              started_at: event.timestamp,
              completed_at: null,
              progress: event.progress,
              error: event.error,
              result: event.function_result ?? null,
              retry_count: event.retry_count ?? 0,
              max_retries: event.max_retries ?? 3,
              last_heartbeat: event.last_heartbeat ?? null,
              timeout_seconds: event.timeout_seconds ?? 300,
              next_retry_at: event.next_retry_at ?? null,
            }
            return [newJob, ...prevJobs]
          }
        })
        setConnected(true)
      },
      onOpen: () => setConnected(true),
      onError: () => setConnected(false),
    })
    return cleanup
  }, [])

  const handleDeleteJob = async (jobId: string) => {
    try {
      const response = await managementApi.deleteJob(jobId)
      if (response.success) {
        setJobs((prevJobs) => prevJobs.filter((j) => j.id !== jobId))
        if (expandedJobId === jobId) setExpandedJobId(null)
      } else {
        showError('Error', `Failed to delete: ${response.error}`)
      }
    } catch (err) {
      showError('Error', `Failed to delete: ${err instanceof Error ? err.message : 'Unknown error'}`)
    }
  }

  const handleClearCompleted = async () => {
    const completedJobs = filteredJobs.filter(
      (job) =>
        (typeof job.status === 'string' && (job.status === 'Completed' || job.status === 'Cancelled')) ||
        (typeof job.status === 'object' && 'Failed' in job.status)
    )

    if (completedJobs.length === 0) {
      showWarning('No Logs', 'No completed executions to clear')
      return
    }

    setClearConfirm({
      message: `Delete ${completedJobs.length} completed execution log(s)?`,
      onConfirm: async () => {
        for (const job of completedJobs) {
          await handleDeleteJob(job.id)
        }
      }
    })
  }

  const getJobTypeCategory = (jobType: JobInfo['job_type']): 'function' | 'flow' => {
    const jobTypeStr = typeof jobType === 'string' ? jobType : JSON.stringify(jobType)
    return jobTypeStr.startsWith('FlowExecution') ? 'flow' : 'function'
  }

  const getJobPath = (jobType: JobInfo['job_type']): string => {
    const jobTypeStr = typeof jobType === 'string' ? jobType : ''
    // Extract path from FunctionExecution(/path/to/func/trigger/exec-id) or FlowExecution(...)
    const match = jobTypeStr.match(/\(([^)]+)\)/)
    if (match) {
      const parts = match[1].split('/')
      // Remove execution ID (last part) and possibly trigger name
      return '/' + parts.slice(0, -1).join('/')
    }
    return jobTypeStr
  }

  const getStatusInfo = (status: JobInfo['status']) => {
    if (typeof status === 'string') {
      return STATUS_CONFIG[status as keyof typeof STATUS_CONFIG] || STATUS_CONFIG.Scheduled
    }
    return STATUS_CONFIG.Failed
  }

  const getStatusText = (status: JobInfo['status']): string => {
    if (typeof status === 'string') return status
    if (typeof status === 'object' && 'Failed' in status) return 'Failed'
    return 'Unknown'
  }

  const isJobRunning = (job: JobInfo): boolean => {
    return typeof job.status === 'string' && job.status === 'Running'
  }

  const isJobCompleted = (job: JobInfo): boolean => {
    return (
      (typeof job.status === 'string' && (job.status === 'Completed' || job.status === 'Cancelled')) ||
      (typeof job.status === 'object' && 'Failed' in job.status)
    )
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
            <Terminal className="w-7 h-7 text-primary-400" />
            Execution Logs
          </h1>
          <p className="text-zinc-400 text-sm mt-1">Function and flow execution history</p>
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
                ? 'bg-primary-500/20 text-primary-400'
                : 'bg-white/5 text-zinc-400 hover:text-white'
            }`}
          >
            <Filter className="w-5 h-5" />
          </button>

          {/* Clear Completed */}
          {filteredJobs.some(isJobCompleted) && (
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
            {/* Type Filter */}
            <div>
              <label className="block text-xs text-zinc-500 mb-1.5">Type</label>
              <div className="flex gap-1">
                {(['all', 'function', 'flow'] as FilterType[]).map((type) => (
                  <button
                    key={type}
                    onClick={() => setTypeFilter(type)}
                    className={`px-3 py-1.5 text-sm rounded-lg transition-colors ${
                      typeFilter === type
                        ? 'bg-primary-500 text-white'
                        : 'bg-white/5 text-zinc-400 hover:text-white'
                    }`}
                  >
                    {type === 'all' ? 'All' : type === 'function' ? 'Functions' : 'Flows'}
                  </button>
                ))}
              </div>
            </div>

            {/* Status Filter */}
            <div>
              <label className="block text-xs text-zinc-500 mb-1.5">Status</label>
              <div className="flex gap-1">
                {(['all', 'running', 'completed', 'failed'] as FilterStatus[]).map((status) => (
                  <button
                    key={status}
                    onClick={() => setStatusFilter(status)}
                    className={`px-3 py-1.5 text-sm rounded-lg transition-colors capitalize ${
                      statusFilter === status
                        ? 'bg-primary-500 text-white'
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
                  placeholder="Search by path or function name..."
                  className="w-full pl-10 pr-3 py-1.5 bg-white/5 border border-white/10 rounded-lg text-sm text-zinc-300 placeholder-zinc-500 focus:outline-none focus:border-primary-500"
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
        {filteredJobs.length} execution{filteredJobs.length !== 1 ? 's' : ''}
        {hasActiveFilters && ` (filtered from ${executionJobs.length})`}
      </div>

      {/* Table */}
      {filteredJobs.length === 0 ? (
        <div className="bg-white/5 border border-white/10 rounded-xl p-12 text-center">
          <Terminal className="w-12 h-12 text-zinc-600 mx-auto mb-4" />
          <p className="text-zinc-400">
            {executionJobs.length === 0 ? 'No executions yet' : 'No executions match filters'}
          </p>
          {hasActiveFilters && (
            <button onClick={clearFilters} className="mt-3 text-sm text-primary-400 hover:text-primary-300">
              Clear filters
            </button>
          )}
        </div>
      ) : (
        <div className="bg-white/5 border border-white/10 rounded-xl overflow-hidden">
          {/* Table Header */}
          <div className="grid grid-cols-[auto_1fr_120px_100px_100px_80px] gap-4 px-4 py-3 bg-white/5 border-b border-white/10 text-xs text-zinc-500 font-medium uppercase tracking-wider">
            <div className="w-6"></div>
            <div>Execution</div>
            <div>Status</div>
            <div>Duration</div>
            <div>Started</div>
            <div></div>
          </div>

          {/* Table Body */}
          <div className="divide-y divide-white/5">
            {filteredJobs.map((job) => {
              const isExpanded = expandedJobId === job.id
              const category = getJobTypeCategory(job.job_type)
              const path = getJobPath(job.job_type)
              const statusInfo = getStatusInfo(job.status)
              const StatusIcon = statusInfo.icon
              const isFailed = typeof job.status === 'object' && 'Failed' in job.status

              return (
                <div key={job.id}>
                  {/* Row */}
                  <div
                    className={`grid grid-cols-[auto_1fr_120px_100px_100px_80px] gap-4 px-4 py-3 items-center hover:bg-white/5 cursor-pointer transition-colors ${
                      isFailed ? 'bg-red-500/5' : ''
                    }`}
                    onClick={() => setExpandedJobId(isExpanded ? null : job.id)}
                  >
                    {/* Expand Icon */}
                    <div className="text-zinc-500">
                      {isExpanded ? (
                        <ChevronDown className="w-5 h-5" />
                      ) : (
                        <ChevronRight className="w-5 h-5" />
                      )}
                    </div>

                    {/* Execution Info */}
                    <div className="flex items-center gap-3 min-w-0">
                      <div className={`p-1.5 rounded-lg ${category === 'flow' ? 'bg-purple-500/20' : 'bg-primary-500/20'}`}>
                        {category === 'flow' ? (
                          <Workflow className="w-4 h-4 text-purple-400" />
                        ) : (
                          <Code className="w-4 h-4 text-primary-400" />
                        )}
                      </div>
                      <div className="min-w-0">
                        <div className="text-sm text-white font-medium truncate" title={path}>
                          {path}
                        </div>
                        <div className="text-xs text-zinc-500 truncate">
                          {category === 'flow' ? 'Flow' : 'Function'}
                          {job.retry_count > 0 && (
                            <span className="ml-2 text-yellow-400">Retry {job.retry_count}/{job.max_retries}</span>
                          )}
                        </div>
                      </div>
                    </div>

                    {/* Status */}
                    <div>
                      <span className={`inline-flex items-center gap-1.5 px-2 py-1 rounded-full text-xs font-medium ${statusInfo.bg} ${statusInfo.color}`}>
                        <StatusIcon className={`w-3 h-3 ${statusInfo.animate ? 'animate-spin' : ''}`} />
                        {getStatusText(job.status)}
                      </span>
                    </div>

                    {/* Duration */}
                    <div className="text-sm text-zinc-400">
                      {job.completed_at ? (
                        formatDuration(new Date(job.completed_at).getTime() - new Date(job.started_at).getTime())
                      ) : isJobRunning(job) ? (
                        <span className="text-blue-400">Running...</span>
                      ) : (
                        '-'
                      )}
                    </div>

                    {/* Started */}
                    <div className="text-xs text-zinc-500">
                      {new Date(job.started_at).toLocaleTimeString()}
                    </div>

                    {/* Actions */}
                    <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
                      {isJobCompleted(job) && (
                        <button
                          onClick={() => handleDeleteJob(job.id)}
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
                        {/* Error Preview for Failed Jobs */}
                        {isFailed && job.error && !job.result && (
                          <div className="mb-4 p-3 bg-red-500/10 border border-red-500/20 rounded-lg">
                            <div className="text-sm text-red-300">{job.error}</div>
                          </div>
                        )}

                        {/* Detailed Card */}
                        {category === 'function' ? (
                          <FunctionExecutionCard
                            jobType={job.job_type as any}
                            result={job.result as any}
                            isRunning={isJobRunning(job)}
                          />
                        ) : (
                          <FlowExecutionCard
                            jobType={job.job_type as any}
                            result={job.result as any}
                            isRunning={isJobRunning(job)}
                            jobError={job.error}
                          />
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
        title="Clear Execution Logs"
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
