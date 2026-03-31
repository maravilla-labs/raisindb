import { useState, useEffect, useRef } from 'react'
import { Database, Search, Sparkles, AlertCircle, RefreshCw, Zap, Trash2, CheckCircle, XCircle, Clock, Activity, Loader2 } from 'lucide-react'
import GlassCard from '../components/GlassCard'
import ConfirmDialog from '../components/ConfirmDialog'
import { repositoriesApi, type Repository } from '../api/repositories'
import {
  databaseManagementApi,
  sseManager,
  formatBytes,
  formatDuration,
  type FulltextHealth,
  type JobInfo,
  type JobEvent,
  formatJobStatus,
} from '../api/management'
import { useToast, ToastContainer } from '../components/Toast'

export default function DatabaseManagement() {
  // State
  const [tenant, setTenant] = useState('default')
  const [repositories, setRepositories] = useState<Repository[]>([])
  const [selectedRepo, setSelectedRepo] = useState<string>('')
  const [branch, setBranch] = useState('main')

  const [fulltextHealth, setFulltextHealth] = useState<FulltextHealth | null>(null)
  const [healthLoading, setHealthLoading] = useState(false)
  const [healthError, setHealthError] = useState<string | null>(null)

  const [activeJobs, setActiveJobs] = useState<JobInfo[]>([])
  const [operationLoading, setOperationLoading] = useState<string | null>(null)

  const [showConfirmDialog, setShowConfirmDialog] = useState(false)
  const [confirmAction, setConfirmAction] = useState<{
    title: string
    message: string
    confirmText: string
    variant: 'danger' | 'warning'
    action: () => Promise<void>
    requireRepoName?: boolean
  } | null>(null)
  const [confirmInput, setConfirmInput] = useState('')
  const { toasts, error: showError, closeToast } = useToast()

  const jobsEndRef = useRef<HTMLDivElement>(null)

  // Load repositories on mount
  useEffect(() => {
    loadRepositories()
  }, [])

  // Load fulltext health when repo changes
  useEffect(() => {
    if (selectedRepo) {
      loadFulltextHealth()
    }
  }, [selectedRepo, tenant])

  // SSE connection for job monitoring
  useEffect(() => {
    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        // Update job list with new event
        setActiveJobs(prev => {
          const existingIndex = prev.findIndex(j => j.id === event.job_id)

          if (existingIndex >= 0) {
            // Update existing job
            const updated = [...prev]
            updated[existingIndex] = {
              ...updated[existingIndex],
              status: event.status as any,
              progress: event.progress,
              error: event.error || null,
              retry_count: event.retry_count ?? updated[existingIndex].retry_count,
              max_retries: event.max_retries ?? updated[existingIndex].max_retries,
              last_heartbeat: event.last_heartbeat ?? updated[existingIndex].last_heartbeat,
              timeout_seconds: event.timeout_seconds ?? updated[existingIndex].timeout_seconds,
              next_retry_at: event.next_retry_at ?? updated[existingIndex].next_retry_at,
            }
            return updated
          } else {
            // Add new job (fetch full info from API if needed)
            // For now, create a basic JobInfo from the event
            const newJob: JobInfo = {
              id: event.job_id,
              job_type: event.job_type as any,
              status: event.status as any,
              tenant: event.tenant,
              started_at: event.timestamp,
              completed_at: null,
              progress: event.progress,
              error: event.error,
              result: null,
              retry_count: event.retry_count ?? 0,
              max_retries: event.max_retries ?? 3,
              last_heartbeat: event.last_heartbeat ?? null,
              timeout_seconds: event.timeout_seconds ?? 300,
              next_retry_at: event.next_retry_at ?? null,
            }
            return [...prev, newJob]
          }
        })

        // Auto-scroll to new jobs
        setTimeout(() => jobsEndRef.current?.scrollIntoView({ behavior: 'smooth' }), 100)
      },
      onError: (error) => {
        console.error('SSE connection error:', error)
      },
    })

    return cleanup
  }, [])

  async function loadRepositories() {
    try {
      const repos = await repositoriesApi.list()
      setRepositories(repos)
      if (repos.length > 0 && !selectedRepo) {
        setSelectedRepo(repos[0].repo_id)
      }
    } catch (error) {
      console.error('Failed to load repositories:', error)
      showErrorMsg('Failed to load repositories')
    }
  }

  async function loadFulltextHealth() {
    if (!selectedRepo) return

    setHealthLoading(true)
    setHealthError(null)

    try {
      const health = await databaseManagementApi.fulltextHealth(tenant, selectedRepo)
      setFulltextHealth(health)
    } catch (error: any) {
      console.error('Failed to load fulltext health:', error)
      setHealthError(error.message || 'Failed to load health metrics')
    } finally {
      setHealthLoading(false)
    }
  }

  function showErrorMsg(message: string) {
    showError('Error', message)
  }

  function showSuccess(message: string) {
    // Simple success notification - could be replaced with a toast library
    console.log('Success:', message)
  }

  function openConfirmDialog(config: typeof confirmAction) {
    setConfirmAction(config)
    setConfirmInput('')
    setShowConfirmDialog(true)
  }

  async function handleConfirm() {
    if (!confirmAction) return

    // Check repo name confirmation if required
    if (confirmAction.requireRepoName && confirmInput !== selectedRepo) {
      return // Don't proceed if repo name doesn't match
    }

    setShowConfirmDialog(false)

    try {
      await confirmAction.action()
    } catch (error: any) {
      showErrorMsg(error.message || 'Operation failed')
    } finally {
      setConfirmInput('')
      setConfirmAction(null)
    }
  }

  // Fulltext operations
  async function verifyFulltext() {
    if (!selectedRepo) return

    setOperationLoading('verify')
    try {
      const response = await databaseManagementApi.fulltextVerify(tenant, selectedRepo)
      showSuccess(`Verification started: ${response.message}`)
    } catch (error: any) {
      showErrorMsg(error.message || 'Failed to start verification')
    } finally {
      setOperationLoading(null)
    }
  }

  async function rebuildFulltext() {
    if (!selectedRepo) return

    openConfirmDialog({
      title: 'Rebuild Fulltext Index',
      message: `This will rebuild the entire fulltext index for ${selectedRepo}. The index will be unavailable during this operation. This may take several minutes depending on the size of your data.`,
      confirmText: 'Rebuild Index',
      variant: 'warning',
      action: async () => {
        setOperationLoading('rebuild')
        try {
          const response = await databaseManagementApi.fulltextRebuild(tenant, selectedRepo)
          showSuccess(`Rebuild started: ${response.message}`)
        } finally {
          setOperationLoading(null)
        }
      },
    })
  }

  async function optimizeFulltext() {
    if (!selectedRepo) return

    setOperationLoading('optimize')
    try {
      const response = await databaseManagementApi.fulltextOptimize(tenant, selectedRepo)
      showSuccess(`Optimization started: ${response.message}`)
    } catch (error: any) {
      showErrorMsg(error.message || 'Failed to start optimization')
    } finally {
      setOperationLoading(null)
    }
  }

  async function purgeFulltext() {
    if (!selectedRepo) return

    openConfirmDialog({
      title: 'Purge Fulltext Index',
      message: `This will permanently delete the entire fulltext index for ${selectedRepo}. All search functionality will be unavailable until the index is rebuilt. Type the repository name "${selectedRepo}" to confirm.`,
      confirmText: 'Purge Index',
      variant: 'danger',
      requireRepoName: true,
      action: async () => {
        setOperationLoading('purge')
        try {
          const response = await databaseManagementApi.fulltextPurge(tenant, selectedRepo)
          showSuccess(`Purge started: ${response.message}`)
          setFulltextHealth(null)
        } finally {
          setOperationLoading(null)
        }
      },
    })
  }

  // Filter jobs for current tenant/repo
  const filteredJobs = activeJobs.filter(job => {
    if (!selectedRepo) return false
    return job.tenant === tenant
    // Note: Jobs don't currently include repo info, so we show all jobs for the tenant
    // This could be enhanced if the backend adds repo information to JobInfo
  })

  function getStatusBadgeColor(status: string): string {
    const statusLower = status.toLowerCase()
    if (statusLower.includes('complete')) return 'bg-green-500/20 border-green-500/30 text-green-300'
    if (statusLower.includes('fail')) return 'bg-red-500/20 border-red-500/30 text-red-300'
    if (statusLower.includes('running')) return 'bg-yellow-500/20 border-yellow-500/30 text-yellow-300'
    return 'bg-blue-500/20 border-blue-500/30 text-blue-300'
  }

  function getStatusIcon(status: string) {
    const statusLower = status.toLowerCase()
    if (statusLower.includes('complete')) return <CheckCircle className="w-4 h-4" />
    if (statusLower.includes('fail')) return <XCircle className="w-4 h-4" />
    if (statusLower.includes('running')) return <Loader2 className="w-4 h-4 animate-spin" />
    return <Clock className="w-4 h-4" />
  }

  return (
    <div className="animate-fade-in max-w-6xl mx-auto">
      {/* Page Header */}
      <div className="mb-8">
        <div className="flex items-center gap-3 mb-2">
          <Database className="w-10 h-10 text-primary-400" />
          <h1 className="text-4xl font-bold text-white">Database Management</h1>
        </div>
        <p className="text-zinc-400">
          Manage indexes and operations for{' '}
          <span className="text-primary-400">{tenant}/{selectedRepo || '(select repository)'}</span>
        </p>
      </div>

      {/* Repository Selector Section */}
      <GlassCard className="mb-6">
        <h2 className="text-xl font-semibold text-white mb-4">Repository Selection</h2>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Tenant</label>
            <input
              type="text"
              value={tenant}
              onChange={(e) => setTenant(e.target.value)}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
              placeholder="default"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Repository</label>
            <select
              value={selectedRepo}
              onChange={(e) => setSelectedRepo(e.target.value)}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
            >
              <option value="">Select repository...</option>
              {repositories.map(repo => (
                <option key={repo.repo_id} value={repo.repo_id}>
                  {repo.repo_id}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Branch</label>
            <input
              type="text"
              value={branch}
              onChange={(e) => setBranch(e.target.value)}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
              placeholder="main"
            />
          </div>
        </div>
      </GlassCard>

      {/* Fulltext Index Section */}
      <GlassCard className="mb-6">
        <div className="flex items-center justify-between mb-6">
          <div className="flex items-center gap-3">
            <Search className="w-6 h-6 text-primary-400" />
            <div>
              <h2 className="text-xl font-semibold text-white">Full-Text Search Index</h2>
              <p className="text-sm text-gray-400">Tantivy-based search indexing</p>
            </div>
          </div>

          <button
            onClick={loadFulltextHealth}
            disabled={!selectedRepo || healthLoading}
            className="flex items-center gap-2 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <RefreshCw className={`w-4 h-4 ${healthLoading ? 'animate-spin' : ''}`} />
            Refresh Health
          </button>
        </div>

        {/* Health Metrics */}
        {healthLoading ? (
          <div className="text-center py-8">
            <Loader2 className="w-8 h-8 text-primary-400 animate-spin mx-auto mb-2" />
            <p className="text-gray-400">Loading health metrics...</p>
          </div>
        ) : healthError ? (
          <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4 mb-6">
            <p className="text-red-300">{healthError}</p>
          </div>
        ) : fulltextHealth ? (
          <div className="grid grid-cols-2 md:grid-cols-5 gap-4 mb-6 p-4 bg-white/5 rounded-lg border border-white/10">
            <div>
              <p className="text-xs text-gray-400 mb-1">Memory Usage</p>
              <p className="text-lg font-semibold text-white">{formatBytes(fulltextHealth.memory_usage_bytes)}</p>
            </div>
            <div>
              <p className="text-xs text-gray-400 mb-1">Disk Usage</p>
              <p className="text-lg font-semibold text-white">{formatBytes(fulltextHealth.disk_usage_bytes)}</p>
            </div>
            <div>
              <p className="text-xs text-gray-400 mb-1">Entry Count</p>
              <p className="text-lg font-semibold text-white">{fulltextHealth.entry_count.toLocaleString()}</p>
            </div>
            <div>
              <p className="text-xs text-gray-400 mb-1">Cache Hit Rate</p>
              <p className="text-lg font-semibold text-white">{(fulltextHealth.cache_hit_rate * 100).toFixed(1)}%</p>
            </div>
            <div>
              <p className="text-xs text-gray-400 mb-1">Last Optimized</p>
              <p className="text-lg font-semibold text-white">
                {fulltextHealth.last_optimized ? new Date(fulltextHealth.last_optimized).toLocaleDateString() : 'Never'}
              </p>
            </div>
          </div>
        ) : selectedRepo ? (
          <div className="bg-blue-500/10 border border-blue-500/20 rounded-lg p-4 mb-6">
            <p className="text-blue-300">Click "Refresh Health" to load index metrics</p>
          </div>
        ) : (
          <div className="bg-gray-500/10 border border-gray-500/20 rounded-lg p-4 mb-6">
            <p className="text-gray-400">Select a repository to view health metrics</p>
          </div>
        )}

        {/* Action Buttons */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          <button
            onClick={verifyFulltext}
            disabled={!selectedRepo || operationLoading === 'verify'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-blue-500/20 hover:bg-blue-500/30 border border-blue-500/30 text-blue-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <AlertCircle className="w-5 h-5" />
            <span className="font-medium">Verify Index</span>
          </button>

          <button
            onClick={rebuildFulltext}
            disabled={!selectedRepo || operationLoading === 'rebuild'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-yellow-500/20 hover:bg-yellow-500/30 border border-yellow-500/30 text-yellow-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <RefreshCw className="w-5 h-5" />
            <span className="font-medium">Rebuild Index</span>
          </button>

          <button
            onClick={optimizeFulltext}
            disabled={!selectedRepo || operationLoading === 'optimize'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-green-500/20 hover:bg-green-500/30 border border-green-500/30 text-green-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Zap className="w-5 h-5" />
            <span className="font-medium">Optimize Index</span>
          </button>

          <button
            onClick={purgeFulltext}
            disabled={!selectedRepo || operationLoading === 'purge'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 text-red-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Trash2 className="w-5 h-5" />
            <span className="font-medium">Purge Index</span>
          </button>
        </div>
      </GlassCard>

      {/* Vector Index Section */}
      <GlassCard className="mb-6 opacity-60">
        <div className="flex items-center justify-between mb-6">
          <div className="flex items-center gap-3">
            <Sparkles className="w-6 h-6 text-purple-400" />
            <div>
              <h2 className="text-xl font-semibold text-white">Vector Embeddings Index</h2>
              <p className="text-sm text-gray-400">HNSW-based vector similarity search</p>
            </div>
          </div>

          <div className="px-3 py-1 bg-purple-500/20 border border-purple-500/30 rounded-lg text-purple-300 text-sm font-medium">
            Coming Soon
          </div>
        </div>

        <div className="bg-purple-500/10 border border-purple-500/20 rounded-lg p-4 mb-6">
          <p className="text-purple-300">
            Vector indexing will be available in Phase 3 of the embeddings implementation.
          </p>
        </div>

        {/* Disabled Action Buttons */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
          <button
            disabled
            className="flex items-center justify-center gap-2 px-4 py-3 bg-white/5 border border-white/10 text-gray-500 rounded-lg cursor-not-allowed"
          >
            <AlertCircle className="w-5 h-5" />
            <span className="font-medium">Verify Index</span>
          </button>

          <button
            disabled
            className="flex items-center justify-center gap-2 px-4 py-3 bg-white/5 border border-white/10 text-gray-500 rounded-lg cursor-not-allowed"
          >
            <RefreshCw className="w-5 h-5" />
            <span className="font-medium">Rebuild Index</span>
          </button>

          <button
            disabled
            className="flex items-center justify-center gap-2 px-4 py-3 bg-white/5 border border-white/10 text-gray-500 rounded-lg cursor-not-allowed"
          >
            <Zap className="w-5 h-5" />
            <span className="font-medium">Optimize Index</span>
          </button>

          <button
            disabled
            className="flex items-center justify-center gap-2 px-4 py-3 bg-white/5 border border-white/10 text-gray-500 rounded-lg cursor-not-allowed"
          >
            <Activity className="w-5 h-5" />
            <span className="font-medium">Restore Index</span>
          </button>
        </div>
      </GlassCard>

      {/* Active Jobs Section */}
      <GlassCard>
        <div className="flex items-center gap-3 mb-6">
          <Activity className="w-6 h-6 text-primary-400" />
          <div>
            <h2 className="text-xl font-semibold text-white">Active Jobs</h2>
            <p className="text-sm text-gray-400">Real-time job monitoring</p>
          </div>
        </div>

        {filteredJobs.length === 0 ? (
          <div className="text-center py-12 text-gray-400">
            <Activity className="w-12 h-12 text-gray-600 mx-auto mb-3" />
            <p>No active jobs</p>
            <p className="text-sm mt-1">Jobs will appear here when operations are started</p>
          </div>
        ) : (
          <div className="space-y-3 max-h-96 overflow-y-auto">
            {filteredJobs
              .sort((a, b) => new Date(b.started_at).getTime() - new Date(a.started_at).getTime())
              .map((job) => {
              const status = formatJobStatus(job.status)
              const duration = job.completed_at
                ? new Date(job.completed_at).getTime() - new Date(job.started_at).getTime()
                : Date.now() - new Date(job.started_at).getTime()

              return (
                <div
                  key={job.id}
                  className="p-4 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 transition-colors"
                >
                  <div className="flex items-start justify-between mb-3">
                    <div className="flex items-center gap-3">
                      {getStatusIcon(status)}
                      <div>
                        <h3 className="font-semibold text-white">{typeof job.job_type === 'string' ? job.job_type : 'Custom Job'}</h3>
                        <p className="text-xs text-gray-400">ID: {job.id}</p>
                      </div>
                    </div>

                    <div className="flex items-center gap-2">
                      {job.retry_count > 0 && (
                        <span className="px-2 py-1 bg-yellow-500/20 border border-yellow-500/30 rounded text-xs text-yellow-300 font-medium">
                          Retry {job.retry_count}/{job.max_retries}
                        </span>
                      )}
                      <div className={`px-3 py-1 border rounded-lg text-sm font-medium ${getStatusBadgeColor(status)}`}>
                        {status}
                      </div>
                    </div>
                  </div>

                  <div className="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm">
                    <div>
                      <p className="text-gray-400">Started</p>
                      <p className="text-white">{new Date(job.started_at).toLocaleTimeString()}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Duration</p>
                      <p className="text-white">{formatDuration(duration)}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Progress</p>
                      <p className="text-white">{job.progress !== null ? `${job.progress}%` : 'N/A'}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Tenant</p>
                      <p className="text-white">{job.tenant || 'All'}</p>
                    </div>
                  </div>

                  {job.error && (
                    <div className="mt-3 p-2 bg-red-500/10 border border-red-500/20 rounded text-sm text-red-300 break-words">
                      <strong>Error:</strong> {(() => {
                        try {
                          const errorObj = JSON.parse(job.error)
                          return errorObj.message || errorObj.error?.message || job.error
                        } catch {
                          return job.error
                        }
                      })()}
                    </div>
                  )}

                  {job.progress !== null && (
                    <div className="mt-3">
                      <div className="w-full h-2 bg-white/10 rounded-full overflow-hidden">
                        <div
                          className="h-full bg-primary-400 transition-all duration-300"
                          style={{ width: `${job.progress}%` }}
                        />
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
            <div ref={jobsEndRef} />
          </div>
        )}
      </GlassCard>

      {/* Confirmation Dialog */}
      {showConfirmDialog && confirmAction && (
        <ConfirmDialog
          open={true}
          title={confirmAction.title}
          message={confirmAction.message}
          confirmText={confirmAction.confirmText}
          variant={confirmAction.variant}
          onConfirm={handleConfirm}
          onCancel={() => {
            setShowConfirmDialog(false)
            setConfirmAction(null)
            setConfirmInput('')
          }}
        />
      )}

      {/* Enhanced Confirmation Modal with Input */}
      {showConfirmDialog && confirmAction?.requireRepoName && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
          <div className="glass-dark rounded-xl max-w-md w-full p-6 animate-slide-in">
            <div className="flex items-center gap-3 mb-4">
              <AlertCircle className="w-6 h-6 text-red-400" />
              <h2 className="text-xl font-bold text-white">{confirmAction.title}</h2>
            </div>

            <p className="text-gray-300 mb-4">{confirmAction.message}</p>

            <div className="mb-6">
              <label className="block text-sm font-medium text-gray-300 mb-2">
                Type repository name to confirm:
              </label>
              <input
                type="text"
                value={confirmInput}
                onChange={(e) => setConfirmInput(e.target.value)}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-red-400 focus:ring-2 focus:ring-red-400/20 transition-all"
                placeholder={selectedRepo}
                autoFocus
              />
            </div>

            <div className="flex gap-3 justify-end">
              <button
                onClick={() => {
                  setShowConfirmDialog(false)
                  setConfirmAction(null)
                  setConfirmInput('')
                }}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleConfirm}
                disabled={confirmInput !== selectedRepo}
                className="px-4 py-2 bg-red-500 hover:bg-red-600 disabled:bg-red-500/50 disabled:cursor-not-allowed text-white rounded-lg transition-colors"
              >
                {confirmAction.confirmText}
              </button>
            </div>
          </div>
        </div>
      )}
      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
