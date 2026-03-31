import { useEffect, useState, useMemo, useCallback, useRef } from 'react'
import { XCircle, Clock, PlayCircle, Trash2, ChevronDown, ChevronRight, Eye, Filter, X, CheckCircle, AlertCircle, Loader2, Ban, Activity, AlertTriangle } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import FunctionExecutionCard from '../../components/management/FunctionExecutionCard'
import FlowExecutionCard from '../../components/management/FlowExecutionCard'
import TriggerEvaluationCard, { TriggerEvaluationReport } from '../../components/management/TriggerEvaluationCard'
import ErrorDetails from '../../components/management/ErrorDetails'
import { managementApi, JobInfo, JobEvent, JobLogEvent, JobQueueStats, sseManager, formatJobType, formatDuration } from '../../api/management'
import ConfirmDialog from '../../components/ConfirmDialog'
import { useToast, ToastContainer } from '../../components/Toast'

// Job type categories for filtering
const JOB_TYPE_CATEGORIES = [
  { value: 'all', label: 'All Jobs' },
  { value: 'function', label: 'Function Execution' },
  { value: 'flow', label: 'Flow Execution' },
  { value: 'trigger', label: 'Trigger Evaluation' },
  { value: 'index', label: 'Indexing' },
  { value: 'other', label: 'Other' },
]

const STATUS_OPTIONS = [
  { value: 'all', label: 'All Statuses' },
  { value: 'running', label: 'Running / Executing' },
  { value: 'scheduled', label: 'Scheduled' },
  { value: 'completed', label: 'Completed' },
  { value: 'failed', label: 'Failed' },
]

// Status icon component
function StatusIcon({ status }: { status: JobInfo['status'] }) {
  if (typeof status === 'string') {
    switch (status) {
      case 'Running':
      case 'Executing':
        return <Loader2 className="w-4 h-4 text-blue-400 animate-spin" />
      case 'Scheduled':
        return <Clock className="w-4 h-4 text-yellow-400" />
      case 'Completed':
        return <CheckCircle className="w-4 h-4 text-green-400" />
      case 'Cancelled':
        return <Ban className="w-4 h-4 text-zinc-400" />
      default:
        return <Clock className="w-4 h-4 text-zinc-400" />
    }
  }
  // Failed status is an object { Failed: "error message" }
  return <AlertCircle className="w-4 h-4 text-red-400" />
}

function getStatusText(status: JobInfo['status']): string {
  if (typeof status === 'string') return status
  if ('Failed' in status) return 'Failed'
  return 'Unknown'
}

function getStatusColor(status: JobInfo['status']): string {
  if (typeof status === 'string') {
    switch (status) {
      case 'Running': return 'text-blue-400'
      case 'Executing': return 'text-blue-400'
      case 'Scheduled': return 'text-yellow-400'
      case 'Completed': return 'text-green-400'
      case 'Cancelled': return 'text-zinc-400'
      default: return 'text-zinc-400'
    }
  }
  return 'text-red-400'
}

function formatRelativeTime(dateStr: string): string {
  const date = new Date(dateStr)
  const now = new Date()
  const diffMs = now.getTime() - date.getTime()
  const diffSec = Math.floor(diffMs / 1000)

  if (diffSec < 60) return `${diffSec}s ago`
  const diffMin = Math.floor(diffSec / 60)
  if (diffMin < 60) return `${diffMin}m ago`
  const diffHour = Math.floor(diffMin / 60)
  if (diffHour < 24) return `${diffHour}h ago`
  const diffDay = Math.floor(diffHour / 24)
  return `${diffDay}d ago`
}

export default function JobsManagement() {
  const [jobs, setJobs] = useState<JobInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [connected, setConnected] = useState(false)
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null)
  const [showDetailsModal, setShowDetailsModal] = useState<{ job: JobInfo } | null>(null)
  const [, setTick] = useState(0)
  const [clearConfirm, setClearConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)
  const { toasts, error: showError, warning: showWarning, success: showSuccess, closeToast } = useToast()

  // Batch delete state
  const [isDeleting, setIsDeleting] = useState(false)
  const [deleteProgress, setDeleteProgress] = useState({ deleted: 0, total: 0 })

  // Cancellation state
  const [cancellingJobs, setCancellingJobs] = useState<Set<string>>(new Set())

  // Queue stats state
  const [queueStats, setQueueStats] = useState<JobQueueStats | null>(null)
  const [purgeConfirm, setPurgeConfirm] = useState<{ type: 'all' | 'orphaned' | 'force-fail'; message: string } | null>(null)

  // Real-time streaming logs per job
  const [jobLogs, setJobLogs] = useState<Map<string, JobLogEvent[]>>(new Map())
  const logContainerRef = useRef<HTMLDivElement>(null)

  // Filter state
  const [typeFilter, setTypeFilter] = useState('all')
  const [statusFilter, setStatusFilter] = useState('all')
  const [pathFilter, setPathFilter] = useState('')
  const [showFilters, setShowFilters] = useState(false)

  // Filter jobs based on criteria
  const filteredJobs = useMemo(() => {
    return jobs.filter(job => {
      // Type filter
      if (typeFilter !== 'all') {
        const jobTypeStr = typeof job.job_type === 'string' ? job.job_type : JSON.stringify(job.job_type)
        switch (typeFilter) {
          case 'function':
            if (!jobTypeStr.startsWith('FunctionExecution')) return false
            break
          case 'flow':
            if (!jobTypeStr.startsWith('FlowExecution')) return false
            break
          case 'trigger':
            if (!jobTypeStr.startsWith('TriggerEvaluation')) return false
            break
          case 'index':
            if (!jobTypeStr.includes('Index') && !jobTypeStr.includes('Fulltext') && !jobTypeStr.includes('Vector')) return false
            break
          case 'other':
            if (jobTypeStr.startsWith('FunctionExecution') ||
                jobTypeStr.startsWith('FlowExecution') ||
                jobTypeStr.startsWith('TriggerEvaluation') ||
                jobTypeStr.includes('Index') ||
                jobTypeStr.includes('Fulltext') ||
                jobTypeStr.includes('Vector')) return false
            break
        }
      }

      // Status filter
      if (statusFilter !== 'all') {
        const statusStr = typeof job.status === 'string' ? job.status.toLowerCase() : 'failed'
        if (statusFilter === 'running' && statusStr !== 'running' && statusStr !== 'executing') return false
        if (statusFilter === 'scheduled' && statusStr !== 'scheduled') return false
        if (statusFilter === 'completed' && statusStr !== 'completed') return false
        if (statusFilter === 'failed' && typeof job.status !== 'object') return false
      }

      // Path filter
      if (pathFilter) {
        const jobTypeStr = typeof job.job_type === 'string' ? job.job_type : JSON.stringify(job.job_type)
        const searchLower = pathFilter.toLowerCase()
        if (!jobTypeStr.toLowerCase().includes(searchLower)) return false
      }

      return true
    })
  }, [jobs, typeFilter, statusFilter, pathFilter])

  const hasActiveFilters = typeFilter !== 'all' || statusFilter !== 'all' || pathFilter !== ''

  const clearFilters = () => {
    setTypeFilter('all')
    setStatusFilter('all')
    setPathFilter('')
  }

  // Auto-scroll log container when new logs arrive for expanded job
  useEffect(() => {
    if (expandedJobId && logContainerRef.current) {
      logContainerRef.current.scrollTop = logContainerRef.current.scrollHeight
    }
  }, [expandedJobId, jobLogs])

  // Update countdown timers every second
  useEffect(() => {
    const interval = setInterval(() => {
      setTick(prev => prev + 1)
    }, 1000)
    return () => clearInterval(interval)
  }, [])

  // Poll queue stats every 5 seconds
  useEffect(() => {
    const fetchStats = async () => {
      try {
        const response = await managementApi.getJobQueueStats()
        if (response.success && response.data) {
          setQueueStats(response.data)
        }
      } catch {
        // Silently ignore stats fetch errors
      }
    }
    fetchStats()
    const interval = setInterval(fetchStats, 5000)
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

  // Connect to SSE for live job updates
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
      onJobLog: (event: JobLogEvent) => {
        setJobLogs((prev) => {
          const updated = new Map(prev)
          const existing = updated.get(event.job_id) || []
          updated.set(event.job_id, [...existing, event])
          return updated
        })
      },
      onOpen: () => {
        setConnected(true)
      },
      onError: () => {
        setConnected(false)
      },
    })

    return cleanup
  }, [])

  const handleCancelJob = async (jobId: string) => {
    setCancellingJobs(prev => new Set(prev).add(jobId))
    try {
      const response = await managementApi.cancelJob(jobId)
      if (!response.success) {
        showError('Error', `Failed to cancel job: ${response.error}`)
      }
    } catch (err) {
      showError('Error', `Failed to cancel job: ${err instanceof Error ? err.message : 'Unknown error'}`)
    } finally {
      // Remove from cancelling state after a brief delay to show feedback
      setTimeout(() => {
        setCancellingJobs(prev => {
          const next = new Set(prev)
          next.delete(jobId)
          return next
        })
      }, 1000)
    }
  }

  const handleDeleteJob = async (jobId: string) => {
    try {
      const response = await managementApi.deleteJob(jobId)
      if (response.success) {
        setJobs((prevJobs) => prevJobs.filter((j) => j.id !== jobId))
        if (expandedJobId === jobId) {
          setExpandedJobId(null)
        }
      } else {
        showError('Error', `Failed to delete job: ${response.error}`)
      }
    } catch (err) {
      showError('Error', `Failed to delete job: ${err instanceof Error ? err.message : 'Unknown error'}`)
    }
  }

  const getCompletedJobs = useCallback(() => {
    return jobs.filter(
      (job) =>
        (typeof job.status === 'string' && (job.status === 'Completed' || job.status === 'Cancelled')) ||
        (typeof job.status === 'object' && 'Failed' in job.status)
    )
  }, [jobs])

  const handleClearCompleted = async () => {
    const completedJobs = getCompletedJobs()

    if (completedJobs.length === 0) {
      showWarning('No Jobs', 'No completed jobs to clear')
      return
    }

    setClearConfirm({
      message: `Delete ${completedJobs.length} completed job(s)?`,
      onConfirm: async () => {
        setIsDeleting(true)
        setDeleteProgress({ deleted: 0, total: completedJobs.length })

        try {
          const jobIds = completedJobs.map(j => j.id)
          const response = await managementApi.batchDeleteJobs(jobIds)

          if (response.success && response.data) {
            const { deleted, skipped } = response.data
            setDeleteProgress({ deleted, total: completedJobs.length })

            // Remove deleted jobs from state
            setJobs(prevJobs => prevJobs.filter(j => !jobIds.includes(j.id) ||
              // Keep jobs that were skipped (running/executing/scheduled)
              (typeof j.status === 'string' && (j.status === 'Running' || j.status === 'Executing' || j.status === 'Scheduled'))
            ))

            if (skipped > 0) {
              showSuccess('Cleared', `Deleted ${deleted} jobs (${skipped} skipped - still running)`)
            } else {
              showSuccess('Cleared', `Deleted ${deleted} jobs`)
            }
          } else {
            showError('Error', response.error || 'Failed to delete jobs')
          }
        } catch (err) {
          showError('Error', `Failed to delete jobs: ${err instanceof Error ? err.message : 'Unknown error'}`)
        } finally {
          setIsDeleting(false)
          setDeleteProgress({ deleted: 0, total: 0 })
        }
      }
    })
  }

  const toggleExpand = (jobId: string) => {
    setExpandedJobId(expandedJobId === jobId ? null : jobId)
  }

  const viewDetails = (job: JobInfo) => {
    setShowDetailsModal({ job })
  }

  const getJobTypeCategory = (jobType: JobInfo['job_type']): string => {
    const jobTypeStr = typeof jobType === 'string' ? jobType : JSON.stringify(jobType)
    if (jobTypeStr.startsWith('FunctionExecution')) return 'function'
    if (jobTypeStr.startsWith('FlowExecution')) return 'flow'
    if (jobTypeStr.startsWith('TriggerEvaluation')) return 'trigger'
    return 'other'
  }

  const isJobRunning = (job: JobInfo): boolean => {
    return typeof job.status === 'string' && (job.status === 'Running' || job.status === 'Executing')
  }

  const isJobComplete = (job: JobInfo): boolean => {
    return (typeof job.status === 'string' && (job.status === 'Completed' || job.status === 'Cancelled')) ||
           (typeof job.status === 'object' && 'Failed' in job.status)
  }

  const renderExpandedContent = (job: JobInfo) => {
    const jobTypeCategory = getJobTypeCategory(job.job_type)

    if (jobTypeCategory === 'function') {
      return (
        <FunctionExecutionCard
          jobType={job.job_type as any}
          result={job.result as any}
          isRunning={isJobRunning(job)}
        />
      )
    }

    if (jobTypeCategory === 'flow') {
      return (
        <FlowExecutionCard
          jobType={job.job_type as any}
          result={job.result as any}
          isRunning={isJobRunning(job)}
          jobError={job.error}
        />
      )
    }

    if (jobTypeCategory === 'trigger' && job.result) {
      return <TriggerEvaluationCard result={job.result as TriggerEvaluationReport} />
    }

    if (typeof job.job_type === 'string' && job.job_type === 'IntegrityScan' && job.result) {
      const report = job.result as any
      return (
        <div className="p-3 bg-white/5 rounded-lg border border-white/10">
          <div className="flex items-center justify-between mb-2">
            <h4 className="text-sm font-semibold text-white">Integrity Report</h4>
            <button
              onClick={() => viewDetails(job)}
              className="text-xs text-primary-400 hover:text-primary-300 flex items-center gap-1"
            >
              <Eye className="w-3 h-3" />
              View Full Report
            </button>
          </div>
          <div className="grid grid-cols-2 gap-2 text-xs text-zinc-300">
            <div>
              <span className="text-zinc-500">Nodes checked:</span> {report.nodes_checked}
            </div>
            <div>
              <span className="text-zinc-500">Issues found:</span>{' '}
              <span className={report.issues_found?.length > 0 ? 'text-yellow-400' : 'text-green-400'}>
                {report.issues_found?.length || 0}
              </span>
            </div>
            <div>
              <span className="text-zinc-500">Health score:</span>{' '}
              <span className={report.health_score >= 0.9 ? 'text-green-400' : report.health_score >= 0.7 ? 'text-yellow-400' : 'text-red-400'}>
                {(report.health_score * 100).toFixed(1)}%
              </span>
            </div>
            <div>
              <span className="text-zinc-500">Duration:</span> {formatDuration(report.duration_ms)}
            </div>
          </div>
        </div>
      )
    }

    if (!job.result && job.error) {
      return <ErrorDetails error={job.error} />
    }

    if (!job.result) {
      return <p className="text-sm text-zinc-500 italic">No result data available</p>
    }

    return (
      <div className="p-3 bg-white/5 rounded-lg border border-white/10">
        <div className="flex items-center justify-between mb-2">
          <h4 className="text-sm font-semibold text-white">Job Result</h4>
          <button
            onClick={() => viewDetails(job)}
            className="text-xs text-primary-400 hover:text-primary-300 flex items-center gap-1"
          >
            <Eye className="w-3 h-3" />
            View Details
          </button>
        </div>
        <pre className="text-xs text-zinc-300 overflow-x-auto max-h-40 overflow-y-auto">
          {JSON.stringify(job.result, null, 2)}
        </pre>
      </div>
    )
  }

  if (loading) {
    return (
      <div className="pt-8">
        <div className="animate-pulse">
          <div className="h-8 bg-white/10 rounded w-48 mb-8"></div>
          <div className="space-y-4">
            {[1, 2, 3].map((i) => (
              <div key={i} className="h-12 bg-white/5 rounded-xl"></div>
            ))}
          </div>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="pt-8">
        <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4 text-red-300">
          {error}
        </div>
      </div>
    )
  }

  const handlePurge = async (type: 'all' | 'orphaned' | 'force-fail') => {
    try {
      if (type === 'force-fail') {
        const response = await managementApi.forceFailStuckJobs(10)
        if (response.success && response.data) {
          showSuccess('Force Fail Complete', `Force-failed ${response.data.failed_count} stuck job(s)`)
          // Refresh job list
          const jobsResponse = await managementApi.listJobs()
          if (jobsResponse.success && jobsResponse.data) {
            setJobs(jobsResponse.data)
          }
        } else {
          showError('Force Fail Failed', response.error || 'Unknown error')
        }
      } else {
        const response = type === 'all'
          ? await managementApi.purgeAllJobs()
          : await managementApi.purgeOrphanedJobs()
        if (response.success && response.data) {
          showSuccess('Purge Complete', `Purged ${response.data.purged} job entries`)
          // Refresh stats
          const statsResponse = await managementApi.getJobQueueStats()
          if (statsResponse.success && statsResponse.data) {
            setQueueStats(statsResponse.data)
          }
          // Refresh job list
          const jobsResponse = await managementApi.listJobs()
          if (jobsResponse.success && jobsResponse.data) {
            setJobs(jobsResponse.data)
          }
        } else {
          showError('Purge Failed', response.error || 'Unknown error')
        }
      }
    } catch (err) {
      showError('Operation Failed', err instanceof Error ? err.message : 'Unknown error')
    }
    setPurgeConfirm(null)
  }

  const completedCount = getCompletedJobs().length

  return (
    <div className="pt-8">
      {/* Header */}
      <div className="flex items-center justify-between mb-4">
        <div>
          <h1 className="text-3xl font-bold text-white mb-2">Background Jobs</h1>
          <p className="text-zinc-400">Monitor and manage background tasks</p>
        </div>
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${connected ? 'bg-green-400' : 'bg-red-400'} animate-pulse`}></div>
            <span className="text-sm text-zinc-400">{connected ? 'Live Updates' : 'Disconnected'}</span>
          </div>
        </div>
      </div>

      {/* Queue Status Panel */}
      {queueStats && (
        <GlassCard className="mb-4">
          <div className="flex items-center gap-2 mb-3">
            <Activity className="w-4 h-4 text-primary-400" />
            <h3 className="text-sm font-medium text-white">Queue Status</h3>
          </div>

          {/* Per-category breakdown (when available from three-pool system) */}
          {queueStats.categories && queueStats.categories.length > 0 ? (
            <div className="space-y-3">
              <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                {queueStats.categories.map((cat) => {
                  const totalQueued = cat.high_queue_len + cat.normal_queue_len + cat.low_queue_len
                  const catColor = cat.category === 'Realtime' ? 'primary' : cat.category === 'Background' ? 'blue' : 'emerald'
                  return (
                    <div key={cat.category} className="p-3 bg-white/5 rounded-lg border border-white/10">
                      <div className="flex items-center justify-between mb-2">
                        <span className={`text-xs font-medium text-${catColor}-400`}>{cat.category}</span>
                        <span className="text-xs text-zinc-500">{cat.total_dispatched.toLocaleString()} dispatched</span>
                      </div>
                      <div className="space-y-1.5">
                        <div className="flex items-center gap-2">
                          <span className="text-[10px] text-zinc-500 w-6">H</span>
                          <div className="flex-1 bg-white/10 rounded-full h-1">
                            <div
                              className={`h-1 rounded-full transition-all ${cat.high_queue_len > 8000 ? 'bg-red-400' : `bg-${catColor}-400`}`}
                              style={{ width: `${Math.min((cat.high_queue_len / 10000) * 100, 100)}%` }}
                            />
                          </div>
                          <span className="text-[10px] font-mono text-zinc-400 w-12 text-right">{cat.high_queue_len.toLocaleString()}</span>
                        </div>
                        <div className="flex items-center gap-2">
                          <span className="text-[10px] text-zinc-500 w-6">N</span>
                          <div className="flex-1 bg-white/10 rounded-full h-1">
                            <div
                              className={`h-1 rounded-full transition-all ${cat.normal_queue_len > 40000 ? 'bg-red-400' : `bg-${catColor}-400`}`}
                              style={{ width: `${Math.min((cat.normal_queue_len / 50000) * 100, 100)}%` }}
                            />
                          </div>
                          <span className="text-[10px] font-mono text-zinc-400 w-12 text-right">{cat.normal_queue_len.toLocaleString()}</span>
                        </div>
                        <div className="flex items-center gap-2">
                          <span className="text-[10px] text-zinc-500 w-6">L</span>
                          <div className="flex-1 bg-white/10 rounded-full h-1">
                            <div
                              className={`h-1 rounded-full transition-all ${cat.low_queue_len > 80000 ? 'bg-red-400' : `bg-${catColor}-400`}`}
                              style={{ width: `${Math.min((cat.low_queue_len / 100000) * 100, 100)}%` }}
                            />
                          </div>
                          <span className="text-[10px] font-mono text-zinc-400 w-12 text-right">{cat.low_queue_len.toLocaleString()}</span>
                        </div>
                      </div>
                      <div className="mt-2 text-[10px] text-zinc-500">
                        {totalQueued.toLocaleString()} queued
                      </div>
                    </div>
                  )
                })}
              </div>
              {/* Aggregate totals + persisted */}
              <div className="flex items-center gap-4 text-xs text-zinc-400">
                <span>
                  Total queued: <span className="font-mono text-white">{(queueStats.queue.high_queue_len + queueStats.queue.normal_queue_len + queueStats.queue.low_queue_len).toLocaleString()}</span>
                </span>
                <span>
                  Workers: <span className="font-mono text-white">{queueStats.workers.pool_size}</span>
                </span>
                <span>
                  Persisted: <span className="font-mono text-white">{queueStats.persisted.total_entries}</span>
                </span>
                {queueStats.persisted.orphaned_entries > 0 && (
                  <span className="text-yellow-400 flex items-center gap-1">
                    <AlertTriangle className="w-3 h-3" />
                    {queueStats.persisted.orphaned_entries} orphaned
                  </span>
                )}
              </div>
            </div>
          ) : (
            /* Fallback: aggregate-only view (single-pool mode) */
            <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
              {/* High Priority Queue */}
              <div>
                <div className="text-xs text-zinc-400 mb-1">High Priority</div>
                <div className="flex items-baseline gap-1">
                  <span className="text-lg font-mono text-white">{queueStats.queue.high_queue_len.toLocaleString()}</span>
                  <span className="text-xs text-zinc-500">/ {queueStats.queue.high_queue_capacity.toLocaleString()}</span>
                </div>
                <div className="w-full bg-white/10 rounded-full h-1 mt-1">
                  <div
                    className={`h-1 rounded-full transition-all ${queueStats.queue.high_queue_len / queueStats.queue.high_queue_capacity > 0.8 ? 'bg-red-400' : 'bg-primary-400'}`}
                    style={{ width: `${Math.min((queueStats.queue.high_queue_len / queueStats.queue.high_queue_capacity) * 100, 100)}%` }}
                  />
                </div>
              </div>
              {/* Normal Priority Queue */}
              <div>
                <div className="text-xs text-zinc-400 mb-1">Normal Priority</div>
                <div className="flex items-baseline gap-1">
                  <span className="text-lg font-mono text-white">{queueStats.queue.normal_queue_len.toLocaleString()}</span>
                  <span className="text-xs text-zinc-500">/ {queueStats.queue.normal_queue_capacity.toLocaleString()}</span>
                </div>
                <div className="w-full bg-white/10 rounded-full h-1 mt-1">
                  <div
                    className={`h-1 rounded-full transition-all ${queueStats.queue.normal_queue_len / queueStats.queue.normal_queue_capacity > 0.8 ? 'bg-red-400' : 'bg-blue-400'}`}
                    style={{ width: `${Math.min((queueStats.queue.normal_queue_len / queueStats.queue.normal_queue_capacity) * 100, 100)}%` }}
                  />
                </div>
              </div>
              {/* Low Priority Queue */}
              <div>
                <div className="text-xs text-zinc-400 mb-1">Low Priority</div>
                <div className="flex items-baseline gap-1">
                  <span className="text-lg font-mono text-white">{queueStats.queue.low_queue_len.toLocaleString()}</span>
                  <span className="text-xs text-zinc-500">/ {queueStats.queue.low_queue_capacity.toLocaleString()}</span>
                </div>
                <div className="w-full bg-white/10 rounded-full h-1 mt-1">
                  <div
                    className={`h-1 rounded-full transition-all ${queueStats.queue.low_queue_len / queueStats.queue.low_queue_capacity > 0.8 ? 'bg-red-400' : 'bg-green-400'}`}
                    style={{ width: `${Math.min((queueStats.queue.low_queue_len / queueStats.queue.low_queue_capacity) * 100, 100)}%` }}
                  />
                </div>
              </div>
              {/* Workers & Persisted */}
              <div>
                <div className="text-xs text-zinc-400 mb-1">Workers / Persisted</div>
                <div className="flex items-baseline gap-2">
                  <span className="text-lg font-mono text-white">{queueStats.workers.pool_size}</span>
                  <span className="text-xs text-zinc-500">workers</span>
                </div>
                <div className="flex items-center gap-2 mt-1">
                  <span className="text-xs text-zinc-400">{queueStats.persisted.total_entries} stored</span>
                  {queueStats.persisted.orphaned_entries > 0 && (
                    <span className="text-xs text-yellow-400 flex items-center gap-1">
                      <AlertTriangle className="w-3 h-3" />
                      {queueStats.persisted.orphaned_entries} orphaned
                    </span>
                  )}
                </div>
              </div>
            </div>
          )}
          {/* Purge Actions */}
          {queueStats.persisted.total_entries > 0 && (
            <div className="mt-3 pt-3 border-t border-white/10 flex items-center gap-2">
              <button
                onClick={() => setPurgeConfirm({ type: 'force-fail', message: 'Force-fail all jobs stuck in Running/Executing state for more than 10 minutes?' })}
                className="px-3 py-1.5 text-xs bg-orange-500/20 text-orange-300 border border-orange-500/30 rounded hover:bg-orange-500/30 transition-colors"
              >
                Force Fail Stuck
              </button>
              {queueStats.persisted.orphaned_entries > 0 && (
                <button
                  onClick={() => setPurgeConfirm({ type: 'orphaned', message: `Purge ${queueStats.persisted.orphaned_entries} orphaned (undeserializable) job entries?` })}
                  className="px-3 py-1.5 text-xs bg-yellow-500/20 text-yellow-300 border border-yellow-500/30 rounded hover:bg-yellow-500/30 transition-colors"
                >
                  Purge Orphaned
                </button>
              )}
              <button
                onClick={() => setPurgeConfirm({ type: 'all', message: `Purge ALL ${queueStats.persisted.total_entries} job entries from persistent storage? This cannot be undone.` })}
                className="px-3 py-1.5 text-xs bg-red-500/20 text-red-300 border border-red-500/30 rounded hover:bg-red-500/30 transition-colors"
              >
                Purge All Jobs
              </button>
            </div>
          )}
        </GlassCard>
      )}

      {/* Purge Confirmation Dialog */}
      <ConfirmDialog
        open={purgeConfirm !== null}
        title={purgeConfirm?.type === 'force-fail' ? 'Force Fail Stuck Jobs' : purgeConfirm?.type === 'all' ? 'Purge All Jobs' : 'Purge Orphaned Jobs'}
        message={purgeConfirm?.message || ''}
        variant="danger"
        confirmText={purgeConfirm?.type === 'force-fail' ? 'Force Fail' : purgeConfirm?.type === 'all' ? 'Purge All' : 'Purge Orphaned'}
        onConfirm={() => {
          if (purgeConfirm) handlePurge(purgeConfirm.type)
        }}
        onCancel={() => setPurgeConfirm(null)}
      />

      {/* Toolbar */}
      <div className="mb-4 flex items-center gap-3">
        {isDeleting ? (
          // Inline progress bar during deletion
          <div className="flex-1 flex items-center gap-3 px-4 py-2 bg-white/5 border border-white/10 rounded-lg">
            <Loader2 className="w-4 h-4 text-primary-400 animate-spin" />
            <div className="flex-1">
              <div className="flex items-center justify-between text-sm mb-1">
                <span className="text-zinc-300">Deleting jobs...</span>
                <span className="text-zinc-400">{deleteProgress.deleted}/{deleteProgress.total}</span>
              </div>
              <div className="w-full bg-white/10 rounded-full h-1.5">
                <div
                  className="bg-primary-500 h-1.5 rounded-full transition-all duration-300"
                  style={{ width: `${deleteProgress.total > 0 ? (deleteProgress.deleted / deleteProgress.total) * 100 : 0}%` }}
                ></div>
              </div>
            </div>
          </div>
        ) : (
          // Normal toolbar
          <>
            <button
              onClick={() => setShowFilters(!showFilters)}
              className={`px-3 py-2 border rounded-lg text-sm flex items-center gap-2 transition-colors ${
                hasActiveFilters
                  ? 'bg-primary-500/20 border-primary-500/50 text-primary-300'
                  : 'bg-zinc-800 border-zinc-700 text-zinc-300 hover:bg-zinc-700'
              }`}
            >
              <Filter className="w-4 h-4" />
              Filters
              {hasActiveFilters && (
                <span className="w-5 h-5 rounded-full bg-primary-500 text-white text-xs flex items-center justify-center">
                  {[typeFilter !== 'all', statusFilter !== 'all', pathFilter !== ''].filter(Boolean).length}
                </span>
              )}
            </button>

            {showFilters && (
              <>
                <select
                  value={typeFilter}
                  onChange={(e) => setTypeFilter(e.target.value)}
                  className="px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 focus:outline-none focus:border-primary-500"
                >
                  {JOB_TYPE_CATEGORIES.map(opt => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>

                <select
                  value={statusFilter}
                  onChange={(e) => setStatusFilter(e.target.value)}
                  className="px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 focus:outline-none focus:border-primary-500"
                >
                  {STATUS_OPTIONS.map(opt => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>

                <input
                  type="text"
                  value={pathFilter}
                  onChange={(e) => setPathFilter(e.target.value)}
                  placeholder="Filter by path..."
                  className="px-3 py-2 bg-zinc-800 border border-zinc-700 rounded-lg text-sm text-zinc-300 placeholder-zinc-500 focus:outline-none focus:border-primary-500 min-w-[200px]"
                />

                {hasActiveFilters && (
                  <button
                    onClick={clearFilters}
                    className="px-2 py-2 text-sm text-zinc-400 hover:text-white"
                  >
                    <X className="w-4 h-4" />
                  </button>
                )}
              </>
            )}

            <div className="flex-1" />

            <span className="text-sm text-zinc-500">
              {filteredJobs.length} of {jobs.length} jobs
            </span>

            {completedCount > 0 && (
              <button
                onClick={handleClearCompleted}
                className="px-3 py-2 bg-zinc-800 hover:bg-zinc-700 border border-zinc-700 rounded-lg text-zinc-300 text-sm flex items-center gap-2 transition-colors"
              >
                <Trash2 className="w-4 h-4" />
                Clear Completed ({completedCount})
              </button>
            )}
          </>
        )}
      </div>

      {/* Jobs Table */}
      {filteredJobs.length === 0 ? (
        <GlassCard>
          <div className="text-center py-12">
            <Clock className="w-16 h-16 text-zinc-500 mx-auto mb-4" />
            <p className="text-zinc-400">
              {jobs.length === 0 ? 'No background jobs running' : 'No jobs match the current filters'}
            </p>
            {hasActiveFilters && jobs.length > 0 && (
              <button
                onClick={clearFilters}
                className="mt-4 px-4 py-2 text-sm text-primary-400 hover:text-primary-300"
              >
                Clear filters
              </button>
            )}
          </div>
        </GlassCard>
      ) : (
        <div className="bg-white/5 border border-white/10 rounded-lg overflow-hidden">
          <table className="w-full">
            <thead className="sticky top-0 bg-zinc-900/95 backdrop-blur z-10">
              <tr className="border-b border-white/10">
                <th className="w-8 px-3 py-3"></th>
                <th className="px-3 py-3 text-left text-xs font-medium text-zinc-400 uppercase tracking-wider">Status</th>
                <th className="px-3 py-3 text-left text-xs font-medium text-zinc-400 uppercase tracking-wider">Type</th>
                <th className="px-3 py-3 text-left text-xs font-medium text-zinc-400 uppercase tracking-wider">Started</th>
                <th className="px-3 py-3 text-left text-xs font-medium text-zinc-400 uppercase tracking-wider w-32">Progress</th>
                <th className="px-3 py-3 text-right text-xs font-medium text-zinc-400 uppercase tracking-wider w-24">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/5">
              {filteredJobs.map((job) => {
                const isExpanded = expandedJobId === job.id
                const complete = isJobComplete(job)
                const running = isJobRunning(job)

                return (
                  <>
                    <tr
                      key={job.id}
                      className={`hover:bg-white/5 transition-colors cursor-pointer ${isExpanded ? 'bg-white/5' : ''}`}
                      onClick={() => toggleExpand(job.id)}
                    >
                      <td className="px-3 py-3">
                        <button className="text-zinc-400 hover:text-white transition-colors">
                          {isExpanded ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
                        </button>
                      </td>
                      <td className="px-3 py-3">
                        <div className="flex items-center gap-2">
                          <StatusIcon status={job.status} />
                          <span className={`text-sm ${getStatusColor(job.status)}`}>
                            {getStatusText(job.status)}
                          </span>
                          {job.retry_count > 0 && (
                            <span className="px-1.5 py-0.5 bg-yellow-500/20 rounded text-[10px] text-yellow-300">
                              {job.retry_count}/{job.max_retries}
                            </span>
                          )}
                          {job.next_retry_at && typeof job.status === 'string' && job.status === 'Scheduled' && (
                            (() => {
                              const now = new Date().getTime()
                              const retryTime = new Date(job.next_retry_at).getTime()
                              const secondsRemaining = Math.max(0, Math.floor((retryTime - now) / 1000))
                              if (secondsRemaining > 0) {
                                return (
                                  <span className="px-1.5 py-0.5 bg-blue-500/20 rounded text-[10px] text-blue-300">
                                    {secondsRemaining}s
                                  </span>
                                )
                              }
                              return null
                            })()
                          )}
                          {/* Log count indicator */}
                          {(jobLogs.get(job.id)?.length ?? 0) > 0 && (
                            <span className="px-1.5 py-0.5 bg-primary-500/20 rounded text-[10px] text-primary-300" title="Log entries received">
                              {jobLogs.get(job.id)!.length} logs
                            </span>
                          )}
                          {/* Last activity for running jobs */}
                          {running && job.last_heartbeat && (
                            <span className="text-[10px] text-zinc-500" title={`Last heartbeat: ${new Date(job.last_heartbeat).toLocaleString()}`}>
                              • {formatRelativeTime(job.last_heartbeat)}
                            </span>
                          )}
                        </div>
                      </td>
                      <td className="px-3 py-3">
                        <div className="flex flex-col">
                          <span className="text-sm text-white truncate max-w-[300px]" title={formatJobType(job.job_type)}>
                            {formatJobType(job.job_type)}
                          </span>
                          {job.tenant && (
                            <span className="text-[10px] text-zinc-500">{job.tenant}</span>
                          )}
                        </div>
                      </td>
                      <td className="px-3 py-3">
                        <span className="text-sm text-zinc-400" title={new Date(job.started_at).toLocaleString()}>
                          {formatRelativeTime(job.started_at)}
                        </span>
                      </td>
                      <td className="px-3 py-3">
                        {running && job.progress !== null ? (
                          <div className="flex items-center gap-2">
                            <div className="flex-1 bg-white/10 rounded-full h-1.5">
                              <div
                                className="bg-primary-500 h-1.5 rounded-full transition-all"
                                style={{ width: `${Math.round(job.progress * 100)}%` }}
                              ></div>
                            </div>
                            <span className="text-xs text-zinc-400 w-9 text-right">
                              {Math.round(job.progress * 100)}%
                            </span>
                          </div>
                        ) : (
                          <span className="text-sm text-zinc-500">—</span>
                        )}
                      </td>
                      <td className="px-3 py-3" onClick={(e) => e.stopPropagation()}>
                        <div className="flex items-center justify-end gap-1">
                          {(running || (typeof job.status === 'string' && (job.status === 'Scheduled'))) && (
                            cancellingJobs.has(job.id) ? (
                              <span className="px-2 py-1 text-[10px] text-orange-300 bg-orange-500/20 rounded flex items-center gap-1">
                                <Loader2 className="w-3 h-3 animate-spin" />
                                Cancelling...
                              </span>
                            ) : (
                              <button
                                onClick={() => handleCancelJob(job.id)}
                                className="p-1.5 hover:bg-red-500/20 rounded transition-colors"
                                title="Cancel job (some jobs may take time to stop)"
                              >
                                <XCircle className="w-4 h-4 text-red-400" />
                              </button>
                            )
                          )}
                          {complete && (
                            <button
                              onClick={() => handleDeleteJob(job.id)}
                              className="p-1.5 hover:bg-zinc-700 rounded transition-colors"
                              title="Delete job"
                            >
                              <Trash2 className="w-4 h-4 text-zinc-400" />
                            </button>
                          )}
                        </div>
                      </td>
                    </tr>
                    {isExpanded && (
                      <tr key={`${job.id}-expanded`}>
                        <td colSpan={6} className="px-4 py-4 bg-black/20">
                          <div className="space-y-3">
                            {/* Job metadata */}
                            <div className="flex items-center gap-4 text-xs text-zinc-400">
                              <span className="flex items-center gap-1">
                                <PlayCircle className="w-3 h-3" />
                                {new Date(job.started_at).toLocaleString()}
                              </span>
                              {job.completed_at && (
                                <span>Completed: {new Date(job.completed_at).toLocaleString()}</span>
                              )}
                              <span className="font-mono text-zinc-500">ID: {job.id}</span>
                            </div>

                            {/* Error message */}
                            {job.error && (
                              <div className="text-sm text-red-300 bg-red-500/10 border border-red-500/20 rounded p-2">
                                {job.error}
                              </div>
                            )}

                            {/* Heartbeat warning */}
                            {running && job.last_heartbeat && (
                              (() => {
                                const heartbeatAge = Math.floor((new Date().getTime() - new Date(job.last_heartbeat).getTime()) / 1000)
                                if (heartbeatAge > 60) {
                                  return (
                                    <div className="text-sm text-yellow-300 bg-yellow-500/10 border border-yellow-500/20 rounded p-2 flex items-center gap-2">
                                      <Clock className="w-4 h-4" />
                                      Warning: No heartbeat for {heartbeatAge}s (timeout: {job.timeout_seconds}s)
                                    </div>
                                  )
                                }
                                return null
                              })()
                            )}

                            {/* Streaming logs */}
                            {running && (
                              <div className="p-3 bg-black/30 rounded-lg border border-white/10">
                                <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-2 flex items-center gap-2">
                                  <span className="w-2 h-2 bg-green-400 rounded-full animate-pulse"></span>
                                  Live Logs
                                </h4>
                                {(jobLogs.get(job.id)?.length ?? 0) > 0 ? (
                                  <div
                                    ref={expandedJobId === job.id ? logContainerRef : undefined}
                                    className="max-h-48 overflow-y-auto font-mono text-xs space-y-0.5"
                                  >
                                    {jobLogs.get(job.id)!.map((log, idx) => (
                                      <div key={idx} className="flex gap-2">
                                        <span className="text-zinc-600 shrink-0">
                                          {new Date(log.timestamp).toLocaleTimeString()}
                                        </span>
                                        <span className={`shrink-0 w-12 ${
                                          log.level === 'error' ? 'text-red-400' :
                                          log.level === 'warn' ? 'text-yellow-400' :
                                          log.level === 'debug' ? 'text-zinc-500' :
                                          'text-zinc-400'
                                        }`}>
                                          [{log.level}]
                                        </span>
                                        <span className={
                                          log.level === 'error' ? 'text-red-300' :
                                          log.level === 'warn' ? 'text-yellow-300' :
                                          'text-zinc-300'
                                        }>
                                          {log.message}
                                        </span>
                                      </div>
                                    ))}
                                  </div>
                                ) : (
                                  <p className="text-xs text-zinc-500 italic">
                                    Waiting for console output... (use console.log or print statements)
                                  </p>
                                )}
                              </div>
                            )}
                            {/* Show logs for completed jobs if any were captured */}
                            {!running && (jobLogs.get(job.id)?.length ?? 0) > 0 && (
                              <div className="p-3 bg-black/30 rounded-lg border border-white/10">
                                <h4 className="text-xs font-semibold text-zinc-400 uppercase tracking-wider mb-2">Captured Logs</h4>
                                <div
                                  ref={expandedJobId === job.id ? logContainerRef : undefined}
                                  className="max-h-48 overflow-y-auto font-mono text-xs space-y-0.5"
                                >
                                  {jobLogs.get(job.id)!.map((log, idx) => (
                                    <div key={idx} className="flex gap-2">
                                      <span className="text-zinc-600 shrink-0">
                                        {new Date(log.timestamp).toLocaleTimeString()}
                                      </span>
                                      <span className={`shrink-0 w-12 ${
                                        log.level === 'error' ? 'text-red-400' :
                                        log.level === 'warn' ? 'text-yellow-400' :
                                        log.level === 'debug' ? 'text-zinc-500' :
                                        'text-zinc-400'
                                      }`}>
                                        [{log.level}]
                                      </span>
                                      <span className={
                                        log.level === 'error' ? 'text-red-300' :
                                        log.level === 'warn' ? 'text-yellow-300' :
                                        'text-zinc-300'
                                      }>
                                        {log.message}
                                      </span>
                                    </div>
                                  ))}
                                </div>
                              </div>
                            )}

                            {/* Result content */}
                            {renderExpandedContent(job)}
                          </div>
                        </td>
                      </tr>
                    )}
                  </>
                )
              })}
            </tbody>
          </table>
        </div>
      )}

      {/* Details Modal */}
      {showDetailsModal && (
        <div
          className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4"
          onClick={() => setShowDetailsModal(null)}
        >
          <div
            className="bg-gradient-to-br from-zinc-900 to-black border border-white/20 rounded-xl max-w-4xl w-full max-h-[80vh] overflow-hidden shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="p-6 border-b border-white/10">
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-2xl font-bold text-white mb-1">
                    {formatJobType(showDetailsModal.job.job_type)} - Details
                  </h2>
                  <p className="text-sm text-zinc-400">Job ID: {showDetailsModal.job.id}</p>
                </div>
                <button
                  onClick={() => setShowDetailsModal(null)}
                  className="p-2 hover:bg-white/10 rounded-lg transition-colors"
                >
                  <XCircle className="w-6 h-6 text-zinc-400" />
                </button>
              </div>
            </div>
            <div className="p-6 overflow-y-auto max-h-[calc(80vh-120px)]">
              <pre className="text-sm text-zinc-300 bg-black/50 p-4 rounded-lg overflow-x-auto">
                {JSON.stringify(showDetailsModal.job.result || showDetailsModal.job, null, 2)}
              </pre>
            </div>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={clearConfirm !== null}
        title="Clear Completed Jobs"
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
