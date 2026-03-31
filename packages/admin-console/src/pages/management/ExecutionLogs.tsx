/**
 * Execution Logs Page
 *
 * Dedicated page for viewing function and flow execution logs with full-screen space.
 * Shows real-time updates via SSE with detailed log viewer and error display.
 */

import { useEffect, useState, useMemo } from 'react'
import { RefreshCw, Trash2, ChevronDown, ChevronRight, X, Terminal, Workflow, Code, Search } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import JobStatusBadge from '../../components/management/JobStatusBadge'
import FunctionExecutionCard from '../../components/management/FunctionExecutionCard'
import FlowExecutionCard from '../../components/management/FlowExecutionCard'
import ErrorDetails from '../../components/management/ErrorDetails'
import { managementApi, JobInfo, JobEvent, sseManager, formatJobType, formatDuration } from '../../api/management'
import ConfirmDialog from '../../components/ConfirmDialog'
import { useToast, ToastContainer } from '../../components/Toast'

// Job type categories for filtering
const JOB_TYPE_CATEGORIES = [
  { value: 'all', label: 'All Executions' },
  { value: 'function', label: 'Functions' },
  { value: 'flow', label: 'Flows' },
]

const STATUS_OPTIONS = [
  { value: 'all', label: 'All Statuses' },
  { value: 'running', label: 'Running' },
  { value: 'completed', label: 'Completed' },
  { value: 'failed', label: 'Failed' },
]

export default function ExecutionLogs() {
  const [jobs, setJobs] = useState<JobInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null)
  const [, setTick] = useState(0)
  const [clearConfirm, setClearConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, warning: showWarning, closeToast } = useToast()

  // Filter state
  const [typeFilter, setTypeFilter] = useState('all')
  const [statusFilter, setStatusFilter] = useState('all')
  const [pathFilter, setPathFilter] = useState('')

  // Filter to only show FunctionExecution and FlowExecution jobs
  const executionJobs = useMemo(() => {
    return jobs.filter(job => {
      const jobTypeStr = typeof job.job_type === 'string' ? job.job_type : JSON.stringify(job.job_type)
      return jobTypeStr.startsWith('FunctionExecution') || jobTypeStr.startsWith('FlowExecution')
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

      // Path filter
      if (pathFilter) {
        const searchLower = pathFilter.toLowerCase()
        if (!jobTypeStr.toLowerCase().includes(searchLower)) return false
      }

      return true
    })
  }, [executionJobs, typeFilter, statusFilter, pathFilter])

  const hasActiveFilters = typeFilter !== 'all' || statusFilter !== 'all' || pathFilter !== ''

  const clearFilters = () => {
    setTypeFilter('all')
    setStatusFilter('all')
    setPathFilter('')
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

  const toggleExpand = (jobId: string) => {
    setExpandedJobId(expandedJobId === jobId ? null : jobId)
  }

  const getJobTypeCategory = (jobType: JobInfo['job_type']): string => {
    const jobTypeStr = typeof jobType === 'string' ? jobType : JSON.stringify(jobType)
    if (jobTypeStr.startsWith('FunctionExecution')) return 'function'
    if (jobTypeStr.startsWith('FlowExecution')) return 'flow'
    return 'other'
  }

  const isJobRunning = (job: JobInfo): boolean => {
    return typeof job.status === 'string' && job.status === 'Running'
  }

  const renderJobDetails = (job: JobInfo) => {
    const category = getJobTypeCategory(job.job_type)

    if (category === 'function') {
      return (
        <FunctionExecutionCard
          jobType={job.job_type as any}
          result={job.result as any}
          isRunning={isJobRunning(job)}
        />
      )
    }

    if (category === 'flow') {
      return (
        <FlowExecutionCard
          jobType={job.job_type as any}
          result={job.result as any}
          isRunning={isJobRunning(job)}
          jobError={job.error}
        />
      )
    }

    if (!job.result && job.error) {
      return <div className="mt-3"><ErrorDetails error={job.error} /></div>
    }

    return null
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

  return (
    <div className="p-8 max-w-7xl mx-auto">
      {/* Header */}
      <div className="flex items-center justify-between mb-6">
        <div>
          <h1 className="text-3xl font-bold text-white mb-2 flex items-center gap-3">
            <Terminal className="w-8 h-8 text-primary-400" />
            Execution Logs
          </h1>
          <p className="text-zinc-400">Monitor function and flow executions in real-time</p>
        </div>
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'} animate-pulse`}></div>
            <span className="text-sm text-zinc-400">{connected ? 'Live' : 'Disconnected'}</span>
          </div>
          {filteredJobs.some((j) =>
            (typeof j.status === 'string' && (j.status === 'Completed' || j.status === 'Cancelled')) ||
            (typeof j.status === 'object' && 'Failed' in j.status)
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
          {/* Type Filter */}
          <div className="flex flex-col gap-1">
            <label className="text-xs text-zinc-500">Type</label>
            <select
              value={typeFilter}
              onChange={(e) => setTypeFilter(e.target.value)}
              className="px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 focus:outline-none focus:border-primary-500"
            >
              {JOB_TYPE_CATEGORIES.map(opt => (
                <option key={opt.value} value={opt.value}>{opt.label}</option>
              ))}
            </select>
          </div>

          {/* Status Filter */}
          <div className="flex flex-col gap-1">
            <label className="text-xs text-zinc-500">Status</label>
            <select
              value={statusFilter}
              onChange={(e) => setStatusFilter(e.target.value)}
              className="px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 focus:outline-none focus:border-primary-500"
            >
              {STATUS_OPTIONS.map(opt => (
                <option key={opt.value} value={opt.value}>{opt.label}</option>
              ))}
            </select>
          </div>

          {/* Path Filter */}
          <div className="flex flex-col gap-1 flex-1 min-w-[200px]">
            <label className="text-xs text-zinc-500">Search Path / Function</label>
            <div className="relative">
              <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-zinc-500" />
              <input
                type="text"
                value={pathFilter}
                onChange={(e) => setPathFilter(e.target.value)}
                placeholder="e.g., /lib/agent-handler"
                className="w-full pl-10 pr-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 placeholder-zinc-500 focus:outline-none focus:border-primary-500"
              />
            </div>
          </div>

          {/* Clear Filters */}
          {hasActiveFilters && (
            <button
              onClick={clearFilters}
              className="px-3 py-2 mt-5 text-sm text-zinc-400 hover:text-white flex items-center gap-1"
            >
              <X className="w-4 h-4" />
              Clear
            </button>
          )}
        </div>

        {/* Results count */}
        <div className="mt-3 text-xs text-zinc-500">
          Showing {filteredJobs.length} of {executionJobs.length} executions
        </div>
      </div>

      {/* Execution List */}
      {filteredJobs.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <Terminal className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <p className="text-zinc-400">
              {executionJobs.length === 0 ? 'No function or flow executions yet' : 'No executions match the current filters'}
            </p>
            {hasActiveFilters && executionJobs.length > 0 && (
              <button onClick={clearFilters} className="mt-4 px-4 py-2 text-sm text-primary-400 hover:text-primary-300">
                Clear filters
              </button>
            )}
          </div>
        </GlassCard>
      ) : (
        <div className="space-y-3">
          {filteredJobs.map((job) => {
            const isExpanded = expandedJobId === job.id
            const isCompleted =
              (typeof job.status === 'string' && (job.status === 'Completed' || job.status === 'Cancelled')) ||
              (typeof job.status === 'object' && 'Failed' in job.status)
            const category = getJobTypeCategory(job.job_type)
            const isFailed = typeof job.status === 'object' && 'Failed' in job.status

            return (
              <div
                key={job.id}
                className={`bg-white/5 border rounded-xl overflow-hidden transition-all ${
                  isFailed ? 'border-red-500/30' : 'border-white/10'
                } ${isExpanded ? 'ring-1 ring-primary-500/30' : ''}`}
              >
                {/* Header Row */}
                <div
                  className="flex items-center gap-3 p-4 cursor-pointer hover:bg-white/5"
                  onClick={() => toggleExpand(job.id)}
                >
                  <button className="text-zinc-400 hover:text-white">
                    {isExpanded ? <ChevronDown className="w-5 h-5" /> : <ChevronRight className="w-5 h-5" />}
                  </button>

                  {/* Icon based on type */}
                  {category === 'function' ? (
                    <Code className="w-5 h-5 text-primary-400" />
                  ) : (
                    <Workflow className="w-5 h-5 text-purple-400" />
                  )}

                  {/* Job Type / Path */}
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-medium text-white truncate">
                        {formatJobType(job.job_type)}
                      </span>
                      <JobStatusBadge status={job.status} />
                      {job.retry_count > 0 && (
                        <span className="px-2 py-0.5 bg-yellow-500/20 text-yellow-300 text-xs rounded">
                          Retry {job.retry_count}/{job.max_retries}
                        </span>
                      )}
                    </div>
                    <div className="text-xs text-zinc-500 mt-1">
                      {new Date(job.started_at).toLocaleString()}
                      {job.completed_at && (
                        <span className="ml-2">
                          Duration: {formatDuration(new Date(job.completed_at).getTime() - new Date(job.started_at).getTime())}
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                    {isJobRunning(job) && (
                      <RefreshCw className="w-4 h-4 text-blue-400 animate-spin" />
                    )}
                    {isCompleted && (
                      <button
                        onClick={() => handleDeleteJob(job.id)}
                        className="p-2 text-zinc-400 hover:text-red-400 hover:bg-red-500/10 rounded-lg transition-colors"
                        title="Delete log"
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    )}
                  </div>
                </div>

                {/* Error Preview (always visible for failed jobs) */}
                {isFailed && !isExpanded && job.error && (
                  <div className="px-4 pb-3 -mt-1">
                    <div className="text-sm text-red-300 bg-red-500/10 rounded p-2 truncate">
                      {job.error}
                    </div>
                  </div>
                )}

                {/* Expanded Details */}
                {isExpanded && (
                  <div className="px-4 pb-4 border-t border-white/5">
                    {renderJobDetails(job)}
                  </div>
                )}
              </div>
            )
          })}
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
