import { useState, useEffect, useRef } from 'react'
import { Database, Search, Sparkles, AlertCircle, RefreshCw, Zap, Trash2, CheckCircle, XCircle, Clock, Activity, Loader2, Package, Link2, Wrench, FileText } from 'lucide-react'
import GlassCard from './GlassCard'
import BranchDropdown from './BranchDropdown'
import ConfirmDialog from './ConfirmDialog'
import { repositoriesApi, type Repository } from '../api/repositories'
import {
  databaseManagementApi,
  managementApi,
  sseManager,
  formatBytes,
  formatDuration,
  type FulltextHealth,
  type VectorHealth,
  type JobInfo,
  type JobEvent,
  formatJobStatus,
} from '../api/management'
import { useToast, ToastContainer } from './Toast'
import { workspacesApi, type Workspace } from '../api/workspaces'

interface DatabaseManagementSharedProps {
  /** If provided, locks to this repository (disables selector) */
  fixedRepository?: string

  /** If true, shows custom branch dropdown instead of text input */
  showBranchSelector?: boolean

  /** Context for filtering and display */
  context: 'repository' | 'tenant'
}

export default function DatabaseManagementShared({
  fixedRepository,
  showBranchSelector = false,
  context,
}: DatabaseManagementSharedProps) {
  // State
  const [tenant, setTenant] = useState('default')
  const [repositories, setRepositories] = useState<Repository[]>([])
  const [selectedRepo, setSelectedRepo] = useState<string>(fixedRepository || '')
  const [branch, setBranch] = useState('main')

  const [fulltextHealth, setFulltextHealth] = useState<FulltextHealth | null>(null)
  const [healthLoading, setHealthLoading] = useState(false)
  const [healthError, setHealthError] = useState<string | null>(null)

  const [vectorHealth, setVectorHealth] = useState<VectorHealth | null>(null)
  const [vectorHealthLoading, setVectorHealthLoading] = useState(false)
  const [vectorHealthError, setVectorHealthError] = useState<string | null>(null)

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
  const [showRegenerateDialog, setShowRegenerateDialog] = useState(false)
  const [forceRegenerate, setForceRegenerate] = useState(false)
  const { toasts, error: showError, closeToast } = useToast()

  // Reindex state
  const [workspaces, setWorkspaces] = useState<Workspace[]>([])
  const [reindexWorkspace, setReindexWorkspace] = useState('')
  const [reindexTypes, setReindexTypes] = useState<string[]>([])
  const [reindexJobId, setReindexJobId] = useState<string | null>(null)
  const [reindexProgress, setReindexProgress] = useState(0)
  const [reindexing, setReindexing] = useState(false)
  const [showReindexConfirm, setShowReindexConfirm] = useState(false)
  const [reindexResult, setReindexResult] = useState<any>(null)

  // Relation integrity state
  const [relationVerifyLoading, setRelationVerifyLoading] = useState(false)
  const [relationVerifyJobId, setRelationVerifyJobId] = useState<string | null>(null)
  const [relationVerifyProgress, setRelationVerifyProgress] = useState(0)
  const [relationVerifyResult, setRelationVerifyResult] = useState<any>(null)
  const [relationRepairLoading, setRelationRepairLoading] = useState(false)
  const [relationRepairJobId, setRelationRepairJobId] = useState<string | null>(null)
  const [relationRepairProgress, setRelationRepairProgress] = useState(0)
  const [relationRepairResult, setRelationRepairResult] = useState<any>(null)
  const [showRelationRepairConfirm, setShowRelationRepairConfirm] = useState(false)

  // Property index cleanup state
  const [propertyIndexCleanupLoading, setPropertyIndexCleanupLoading] = useState(false)
  const [propertyIndexCleanupResult, setPropertyIndexCleanupResult] = useState<any>(null)

  const jobsEndRef = useRef<HTMLDivElement>(null)
  const jobsContainerRef = useRef<HTMLDivElement>(null)

  // Load repositories on mount (only if not fixed)
  useEffect(() => {
    if (!fixedRepository) {
      loadRepositories()
    }
  }, [fixedRepository])

  // Lock repository if fixed
  useEffect(() => {
    if (fixedRepository) {
      setSelectedRepo(fixedRepository)
    }
  }, [fixedRepository])

  // Load fulltext health when repo changes
  useEffect(() => {
    if (selectedRepo) {
      loadFulltextHealth()
    }
  }, [selectedRepo, tenant])

  // Load vector health when repo changes
  useEffect(() => {
    if (selectedRepo) {
      loadVectorHealth()
    }
  }, [selectedRepo, tenant])

  // Load workspaces when repo changes
  useEffect(() => {
    if (selectedRepo) {
      loadWorkspaces()
    }
  }, [selectedRepo])

  // SSE connection for job monitoring
  useEffect(() => {
    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        setActiveJobs(prev => {
          const existingIndex = prev.findIndex(j => j.id === event.job_id)

          if (existingIndex >= 0) {
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

        // Scroll only the jobs container, not the entire page
        setTimeout(() => {
          if (jobsContainerRef.current) {
            jobsContainerRef.current.scrollTop = jobsContainerRef.current.scrollHeight
          }
        }, 100)
      },
      onError: (error) => {
        console.error('SSE connection error:', error)
      },
    })

    return cleanup
  }, [])

  // SSE hook for reindex job progress
  useEffect(() => {
    if (!reindexJobId) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        if (event.job_id !== reindexJobId) return

        if (event.progress !== null) {
          setReindexProgress(event.progress)
        }

        if (event.status === 'Completed') {
          // Fetch final result
          managementApi.getJobInfo(reindexJobId).then((response) => {
            if (response.success && response.data) {
              setReindexResult(response.data.result)
              setReindexing(false)
              showSuccess('Reindex completed successfully')
            }
          }).catch((error) => {
            console.error('Failed to fetch job result:', error)
            setReindexing(false)
          })
        }

        if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
          setReindexing(false)
          showErrorMsg(event.error || 'Reindex job failed')
        }
      }
    })

    return cleanup
  }, [reindexJobId])

  // SSE hook for relation verify job progress
  useEffect(() => {
    if (!relationVerifyJobId) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        if (event.job_id !== relationVerifyJobId) return

        if (event.progress !== null) {
          setRelationVerifyProgress(event.progress)
        }

        if (event.status === 'Completed') {
          managementApi.getJobInfo(relationVerifyJobId).then((response) => {
            if (response.success && response.data?.result) {
              setRelationVerifyResult({ type: 'success', data: response.data.result })
            } else {
              setRelationVerifyResult({ type: 'success', message: 'Relation verification completed' })
            }
            setRelationVerifyLoading(false)
            setRelationVerifyJobId(null)
            setRelationVerifyProgress(0)
          }).catch((error) => {
            console.error('Failed to fetch job result:', error)
            setRelationVerifyLoading(false)
          })
        }

        if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
          setRelationVerifyResult({ type: 'error', message: event.error || 'Relation verification failed' })
          setRelationVerifyLoading(false)
          setRelationVerifyJobId(null)
          setRelationVerifyProgress(0)
        }
      }
    })

    return cleanup
  }, [relationVerifyJobId])

  // SSE hook for relation repair job progress
  useEffect(() => {
    if (!relationRepairJobId) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        if (event.job_id !== relationRepairJobId) return

        if (event.progress !== null) {
          setRelationRepairProgress(event.progress)
        }

        if (event.status === 'Completed') {
          managementApi.getJobInfo(relationRepairJobId).then((response) => {
            if (response.success && response.data?.result) {
              setRelationRepairResult({ type: 'success', data: response.data.result })
            } else {
              setRelationRepairResult({ type: 'success', message: 'Relation repair completed' })
            }
            setRelationRepairLoading(false)
            setRelationRepairJobId(null)
            setRelationRepairProgress(0)
          }).catch((error) => {
            console.error('Failed to fetch job result:', error)
            setRelationRepairLoading(false)
          })
        }

        if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
          setRelationRepairResult({ type: 'error', message: event.error || 'Relation repair failed' })
          setRelationRepairLoading(false)
          setRelationRepairJobId(null)
          setRelationRepairProgress(0)
        }
      }
    })

    return cleanup
  }, [relationRepairJobId])

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

  async function loadVectorHealth() {
    if (!selectedRepo) return

    setVectorHealthLoading(true)
    setVectorHealthError(null)
    try {
      const health = await databaseManagementApi.vectorHealth(tenant, selectedRepo)
      setVectorHealth(health)
    } catch (error: any) {
      console.error('Failed to load vector health:', error)
      setVectorHealthError(error.message || 'Failed to load health metrics')
    } finally {
      setVectorHealthLoading(false)
    }
  }

  function showErrorMsg(message: string) {
    showError('Error', message)
  }

  function showSuccess(message: string) {
    console.log('Success:', message)
  }

  function openConfirmDialog(config: typeof confirmAction) {
    setConfirmAction(config)
    setConfirmInput('')
    setShowConfirmDialog(true)
  }

  async function handleConfirm() {
    if (!confirmAction) return

    if (confirmAction.requireRepoName && confirmInput !== selectedRepo) {
      return
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

  // Vector operations
  async function verifyVector() {
    if (!selectedRepo) return

    setOperationLoading('vector-verify')
    try {
      const response = await databaseManagementApi.vectorVerify(tenant, selectedRepo)
      showSuccess(`Vector verification started: ${response.message}`)
    } catch (error: any) {
      showErrorMsg(error.message || 'Failed to start vector verification')
    } finally {
      setOperationLoading(null)
    }
  }

  async function rebuildVector() {
    if (!selectedRepo) return

    openConfirmDialog({
      title: 'Rebuild Vector Index',
      message: `This will rebuild the entire HNSW vector index for ${selectedRepo}. This operation will:\n\n• Purge the existing HNSW index\n• Re-index all embeddings from RocksDB\n• Use the current dimensions from tenant config\n\nThis is useful when:\n• You changed the embedding model dimensions\n• The vector index is corrupted\n• You want to rebuild with new HNSW parameters\n\nThis may take several minutes depending on the number of embeddings.`,
      confirmText: 'Rebuild Vector Index',
      variant: 'warning',
      action: async () => {
        setOperationLoading('vector-rebuild')
        try {
          const response = await databaseManagementApi.vectorRebuild(tenant, selectedRepo)
          showSuccess(`Vector rebuild started: ${response.message}`)
        } finally {
          setOperationLoading(null)
        }
      },
    })
  }

  async function regenerateVector() {
    if (!selectedRepo) return
    setForceRegenerate(false)
    setShowRegenerateDialog(true)
  }

  async function handleRegenerateConfirm() {
    if (!selectedRepo) return

    setShowRegenerateDialog(false)
    setOperationLoading('vector-regenerate')
    try {
      const response = await databaseManagementApi.vectorRegenerate(tenant, selectedRepo, forceRegenerate)
      showSuccess(`Embedding regeneration started: ${response.message}`)
    } catch (error: any) {
      showErrorMsg(error.message || 'Failed to start embedding regeneration')
    } finally {
      setOperationLoading(null)
      setForceRegenerate(false)
    }
  }

  async function optimizeVector() {
    if (!selectedRepo) return

    setOperationLoading('vector-optimize')
    try {
      const response = await databaseManagementApi.vectorOptimize(tenant, selectedRepo)
      showSuccess(`Vector optimization started: ${response.message}`)
    } catch (error: any) {
      showErrorMsg(error.message || 'Failed to start vector optimization')
    } finally {
      setOperationLoading(null)
    }
  }

  async function restoreVector() {
    if (!selectedRepo) return

    setOperationLoading('vector-restore')
    try {
      const response = await databaseManagementApi.vectorRestore(tenant, selectedRepo)
      showSuccess(`Vector restore started: ${response.message}`)
    } catch (error: any) {
      showErrorMsg(error.message || 'Failed to start vector restore')
    } finally {
      setOperationLoading(null)
    }
  }

  // Reindex operations
  async function loadWorkspaces() {
    if (!selectedRepo) return

    try {
      const workspaceList = await workspacesApi.list(selectedRepo)
      setWorkspaces(workspaceList)
      if (workspaceList.length > 0 && !reindexWorkspace) {
        setReindexWorkspace(workspaceList[0].name)
      }
    } catch (error: any) {
      console.error('Failed to load workspaces:', error)
      showErrorMsg('Failed to load workspaces')
    }
  }

  function handleReindexTypeToggle(type: string) {
    if (type === 'All') {
      if (reindexTypes.includes('All')) {
        setReindexTypes([])
      } else {
        setReindexTypes(['All'])
      }
    } else {
      if (reindexTypes.includes('All')) {
        setReindexTypes([type])
      } else {
        if (reindexTypes.includes(type)) {
          setReindexTypes(reindexTypes.filter(t => t !== type))
        } else {
          setReindexTypes([...reindexTypes, type])
        }
      }
    }
  }

  async function handleStartReindex() {
    if (!selectedRepo || !reindexWorkspace) {
      showErrorMsg('Please select a workspace')
      return
    }

    if (reindexTypes.length === 0) {
      showErrorMsg('Please select at least one index type')
      return
    }

    setReindexing(true)
    setReindexProgress(0)
    setReindexResult(null)

    try {
      const response = await managementApi.startReindex(
        tenant,
        selectedRepo,
        reindexWorkspace,
        reindexTypes,
        branch !== 'main' ? branch : undefined
      )

      if (response.success && response.data) {
        setReindexJobId(response.data.job_id)
        showSuccess('Reindex started')
      } else {
        throw new Error(response.error || 'Failed to start reindex')
      }
    } catch (error: any) {
      showErrorMsg(error.message || 'Failed to start reindex')
      setReindexing(false)
    } finally {
      setShowReindexConfirm(false)
    }
  }

  function openReindexConfirmDialog() {
    if (!reindexWorkspace) {
      showErrorMsg('Please select a workspace')
      return
    }

    if (reindexTypes.length === 0) {
      showErrorMsg('Please select at least one index type')
      return
    }

    setShowReindexConfirm(true)
  }

  // Relation integrity handlers
  async function handleVerifyRelations() {
    if (!selectedRepo) return

    setRelationVerifyLoading(true)
    setRelationVerifyResult(null)
    setRelationVerifyProgress(0)

    try {
      const response = await databaseManagementApi.relationsVerify(tenant, selectedRepo, branch !== 'main' ? branch : undefined)
      if (response.job_id) {
        setRelationVerifyJobId(response.job_id)
      } else {
        setRelationVerifyResult({ type: 'error', message: 'Failed to start relation verification' })
        setRelationVerifyLoading(false)
      }
    } catch (error: any) {
      setRelationVerifyResult({ type: 'error', message: error.message || 'Unknown error' })
      setRelationVerifyLoading(false)
    }
  }

  async function handleRepairRelations() {
    if (!selectedRepo) return

    setRelationRepairLoading(true)
    setRelationRepairResult(null)
    setRelationRepairProgress(0)

    try {
      const response = await databaseManagementApi.relationsRepair(tenant, selectedRepo, branch !== 'main' ? branch : undefined)
      if (response.job_id) {
        setRelationRepairJobId(response.job_id)
      } else {
        setRelationRepairResult({ type: 'error', message: 'Failed to start relation repair' })
        setRelationRepairLoading(false)
      }
    } catch (error: any) {
      setRelationRepairResult({ type: 'error', message: error.message || 'Unknown error' })
      setRelationRepairLoading(false)
    }
  }

  function openRelationRepairConfirmDialog() {
    setShowRelationRepairConfirm(true)
  }

  // Property index cleanup handler
  async function handlePropertyIndexCleanup() {
    if (!reindexWorkspace) {
      showErrorMsg('Please select a workspace')
      return
    }

    setPropertyIndexCleanupLoading(true)
    setPropertyIndexCleanupResult(null)

    try {
      const response = await managementApi.cleanupPropertyIndexOrphans(reindexWorkspace)
      if (response.success && response.data) {
        const stats = response.data
        setPropertyIndexCleanupResult({
          type: 'success',
          data: stats,
          message: `Scanned ${stats.entries_scanned} entries, found ${stats.orphaned_found} orphaned, deleted ${stats.orphaned_deleted}`
        })
      } else {
        setPropertyIndexCleanupResult({
          type: 'error',
          message: response.error || 'Failed to cleanup property index orphans'
        })
      }
    } catch (error: any) {
      setPropertyIndexCleanupResult({
        type: 'error',
        message: error.message || 'Unknown error'
      })
    } finally {
      setPropertyIndexCleanupLoading(false)
    }
  }

  // Filter jobs
  const filteredJobs = activeJobs.filter(job => {
    if (!selectedRepo) return false
    return job.tenant === tenant
  })

  // Count pending/running embedding generation jobs for the current tenant/repo
  const pendingEmbeddingJobs = filteredJobs.filter(job =>
    (typeof job.job_type === 'string' && job.job_type.startsWith('EmbeddingGenerate') ||
     typeof job.job_type === 'object' && 'EmbeddingGenerate' in job.job_type) &&
    (job.status === 'Scheduled' || job.status === 'Running')
  ).length

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

  // Disable tenant and repo selectors in tenant context or when repository is fixed
  const isRepoSelectorDisabled = !!fixedRepository || context === 'tenant'
  const isTenantSelectorDisabled = context === 'tenant'

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
              disabled={isTenantSelectorDisabled}
              className={`w-full px-3 py-2 border rounded-lg transition-all ${
                isTenantSelectorDisabled
                  ? 'bg-white/5 border-white/5 text-white/30 cursor-not-allowed'
                  : 'bg-white/5 border-white/10 text-white focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20'
              }`}
              placeholder="default"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Repository</label>
            {isRepoSelectorDisabled ? (
              <div className="w-full px-3 py-2 bg-white/5 border border-white/5 rounded-lg text-white/30 cursor-not-allowed">
                {selectedRepo}
              </div>
            ) : (
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
            )}
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Branch</label>
            {showBranchSelector ? (
              <BranchDropdown
                repo={selectedRepo}
                currentBranch={branch}
                onBranchChange={setBranch}
                disabled={!selectedRepo}
              />
            ) : (
              <input
                type="text"
                value={branch}
                onChange={(e) => setBranch(e.target.value)}
                className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-primary-400 focus:ring-2 focus:ring-primary-400/20 transition-all"
                placeholder="main"
              />
            )}
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
      <GlassCard className="mb-6">
        <div className="flex items-center justify-between mb-6">
          <div className="flex items-center gap-3">
            <Sparkles className="w-6 h-6 text-purple-400" />
            <div>
              <h2 className="text-xl font-semibold text-white">Vector Embeddings Index</h2>
              <p className="text-sm text-gray-400">HNSW-based vector similarity search</p>
            </div>
          </div>

          <button
            onClick={loadVectorHealth}
            disabled={!selectedRepo || vectorHealthLoading}
            className="flex items-center gap-2 px-4 py-2 bg-white/5 hover:bg-white/10 border border-white/10 text-white rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <RefreshCw className={`w-4 h-4 ${vectorHealthLoading ? 'animate-spin' : ''}`} />
            Refresh Health
          </button>
        </div>

        {/* Health Metrics */}
        {vectorHealthLoading ? (
          <div className="text-center py-8">
            <Loader2 className="w-8 h-8 text-purple-400 animate-spin mx-auto mb-2" />
            <p className="text-gray-400">Loading health metrics...</p>
          </div>
        ) : vectorHealthError ? (
          <div className="bg-red-500/10 border border-red-500/20 rounded-lg p-4 mb-6">
            <p className="text-red-300">{vectorHealthError}</p>
          </div>
        ) : vectorHealth ? (
          <div className="grid grid-cols-2 md:grid-cols-4 gap-4 mb-6 p-4 bg-white/5 rounded-lg border border-white/10">
            <div>
              <p className="text-xs text-gray-400 mb-1">Memory Usage</p>
              <p className="text-lg font-semibold text-white">{formatBytes(vectorHealth.memory_usage_bytes)}</p>
            </div>
            <div>
              <p className="text-xs text-gray-400 mb-1">Disk Usage</p>
              <p className="text-lg font-semibold text-white">{formatBytes(vectorHealth.disk_usage_bytes)}</p>
            </div>
            <div>
              <p className="text-xs text-gray-400 mb-1">Entry Count</p>
              <p className="text-lg font-semibold text-white">{vectorHealth.entry_count.toLocaleString()}</p>
            </div>
            <div>
              <p className="text-xs text-gray-400 mb-1">Index Type / Dimensions</p>
              <p className="text-lg font-semibold text-white">{vectorHealth.index_type} / {vectorHealth.dimensions}D</p>
            </div>
          </div>
        ) : selectedRepo ? (
          <div className="bg-purple-500/10 border border-purple-500/20 rounded-lg p-4 mb-6">
            <p className="text-purple-300">Click "Refresh Health" to load index metrics</p>
          </div>
        ) : (
          <div className="bg-gray-500/10 border border-gray-500/20 rounded-lg p-4 mb-6">
            <p className="text-gray-400">Select a repository to view health metrics</p>
          </div>
        )}

        {/* Action Buttons */}
        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
          <button
            onClick={verifyVector}
            disabled={!selectedRepo || operationLoading === 'vector-verify'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-purple-500/20 hover:bg-purple-500/30 border border-purple-500/30 text-purple-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <AlertCircle className="w-5 h-5" />
            <span className="font-medium">Verify Index</span>
          </button>

          <button
            onClick={rebuildVector}
            disabled={!selectedRepo || operationLoading === 'vector-rebuild'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-yellow-500/20 hover:bg-yellow-500/30 border border-yellow-500/30 text-yellow-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <RefreshCw className="w-5 h-5" />
            <span className="font-medium">Rebuild Index</span>
          </button>

          <button
            onClick={regenerateVector}
            disabled={!selectedRepo || operationLoading === 'vector-regenerate' || pendingEmbeddingJobs > 0}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-orange-500/20 hover:bg-orange-500/30 border border-orange-500/30 text-orange-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed relative"
          >
            <RefreshCw className="w-5 h-5" />
            <span className="font-medium">Regenerate Embeddings</span>
            {pendingEmbeddingJobs > 0 && (
              <span className="absolute -top-1 -right-1 bg-orange-500 text-white text-xs font-bold rounded-full w-5 h-5 flex items-center justify-center">
                {pendingEmbeddingJobs}
              </span>
            )}
          </button>

          <button
            onClick={optimizeVector}
            disabled={!selectedRepo || operationLoading === 'vector-optimize'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-green-500/20 hover:bg-green-500/30 border border-green-500/30 text-green-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Zap className="w-5 h-5" />
            <span className="font-medium">Optimize Index</span>
          </button>

          <button
            onClick={restoreVector}
            disabled={!selectedRepo || operationLoading === 'vector-restore'}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-blue-500/20 hover:bg-blue-500/30 border border-blue-500/30 text-blue-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            <Activity className="w-5 h-5" />
            <span className="font-medium">Restore Index</span>
          </button>
        </div>
      </GlassCard>

      {/* Reindex Database Section */}
      <GlassCard className="mb-6">
        <div className="flex items-center gap-3 mb-6">
          <Package className="w-6 h-6 text-cyan-400" />
          <div>
            <h2 className="text-xl font-semibold text-white">Reindex Database</h2>
            <p className="text-sm text-gray-400">Rebuild indexes for a specific workspace</p>
          </div>
        </div>

        {/* Workspace and Index Type Selection */}
        <div className="space-y-4 mb-6">
          <div>
            <label className="block text-sm font-medium text-gray-300 mb-2">Workspace</label>
            <select
              value={reindexWorkspace}
              onChange={(e) => setReindexWorkspace(e.target.value)}
              disabled={!selectedRepo || reindexing}
              className="w-full px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white focus:border-cyan-400 focus:ring-2 focus:ring-cyan-400/20 transition-all disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <option value="">Select workspace...</option>
              {workspaces.map(workspace => (
                <option key={workspace.name} value={workspace.name}>
                  {workspace.name}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-300 mb-3">Index Types</label>
            <div className="space-y-2">
              <label className="flex items-center gap-3 p-3 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 transition-colors cursor-pointer">
                <input
                  type="checkbox"
                  checked={reindexTypes.includes('child_order')}
                  onChange={() => handleReindexTypeToggle('child_order')}
                  disabled={reindexing}
                  className="w-4 h-4 rounded border-cyan-500/30 bg-white/5 text-cyan-500 focus:ring-2 focus:ring-cyan-400/20 transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                />
                <div className="flex-1">
                  <p className="text-white font-medium">Child Order</p>
                  <p className="text-xs text-gray-400">Rebuild child order indexes</p>
                </div>
              </label>

              <label className="flex items-center gap-3 p-3 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 transition-colors cursor-pointer">
                <input
                  type="checkbox"
                  checked={reindexTypes.includes('property')}
                  onChange={() => handleReindexTypeToggle('property')}
                  disabled={reindexing}
                  className="w-4 h-4 rounded border-cyan-500/30 bg-white/5 text-cyan-500 focus:ring-2 focus:ring-cyan-400/20 transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                />
                <div className="flex-1">
                  <p className="text-white font-medium">Property Indexes</p>
                  <p className="text-xs text-gray-400">Rebuild property value indexes</p>
                </div>
              </label>

              <label className="flex items-center gap-3 p-3 bg-white/5 border border-white/10 rounded-lg hover:bg-white/10 transition-colors cursor-pointer">
                <input
                  type="checkbox"
                  checked={reindexTypes.includes('reference')}
                  onChange={() => handleReindexTypeToggle('reference')}
                  disabled={reindexing}
                  className="w-4 h-4 rounded border-cyan-500/30 bg-white/5 text-cyan-500 focus:ring-2 focus:ring-cyan-400/20 transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                />
                <div className="flex-1">
                  <p className="text-white font-medium">References</p>
                  <p className="text-xs text-gray-400">Rebuild relation and reference indexes</p>
                </div>
              </label>

              <label className="flex items-center gap-3 p-3 bg-cyan-500/10 border border-cyan-500/20 rounded-lg hover:bg-cyan-500/20 transition-colors cursor-pointer">
                <input
                  type="checkbox"
                  checked={reindexTypes.includes('all')}
                  onChange={() => handleReindexTypeToggle('all')}
                  disabled={reindexing}
                  className="w-4 h-4 rounded border-cyan-500/30 bg-white/5 text-cyan-500 focus:ring-2 focus:ring-cyan-400/20 transition-all disabled:opacity-50 disabled:cursor-not-allowed"
                />
                <div className="flex-1">
                  <p className="text-cyan-300 font-medium">All</p>
                  <p className="text-xs text-cyan-400/80">Rebuild all indexes (recommended)</p>
                </div>
              </label>
            </div>
          </div>
        </div>

        {/* Reindex Button */}
        <button
          onClick={openReindexConfirmDialog}
          disabled={!selectedRepo || !reindexWorkspace || reindexTypes.length === 0 || reindexing}
          className="w-full flex items-center justify-center gap-2 px-6 py-3 bg-cyan-500/20 hover:bg-cyan-500/30 border border-cyan-500/30 text-cyan-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed font-medium"
        >
          {reindexing ? (
            <>
              <Loader2 className="w-5 h-5 animate-spin" />
              <span>Reindexing...</span>
            </>
          ) : (
            <>
              <RefreshCw className="w-5 h-5" />
              <span>Start Reindex</span>
            </>
          )}
        </button>

        {/* Progress Display */}
        {reindexing && (
          <div className="mt-6 p-4 bg-cyan-500/10 border border-cyan-500/20 rounded-lg space-y-3">
            <div className="flex items-center gap-3">
              <Loader2 className="w-5 h-5 text-cyan-400 animate-spin" />
              <div className="flex-1">
                <p className="text-white font-medium">Reindexing in progress...</p>
                <p className="text-sm text-cyan-300">{reindexProgress}% complete</p>
              </div>
            </div>
            <div className="w-full h-2 bg-white/10 rounded-full overflow-hidden">
              <div
                className="h-full bg-cyan-400 transition-all duration-300"
                style={{ width: `${reindexProgress}%` }}
              />
            </div>
          </div>
        )}

        {/* Result Display */}
        {reindexResult && !reindexing && (
          <div className="mt-6 p-4 bg-green-500/10 border border-green-500/20 rounded-lg">
            <div className="flex items-center gap-3 mb-3">
              <CheckCircle className="w-5 h-5 text-green-400" />
              <p className="text-white font-medium">Reindex Complete!</p>
            </div>
            {reindexResult.items_processed !== undefined && (
              <div className="grid grid-cols-2 gap-3 text-sm">
                <div>
                  <p className="text-gray-400">Items Processed</p>
                  <p className="text-white font-semibold">{reindexResult.items_processed?.toLocaleString() || 'N/A'}</p>
                </div>
                <div>
                  <p className="text-gray-400">Duration</p>
                  <p className="text-white font-semibold">{reindexResult.duration_ms ? formatDuration(reindexResult.duration_ms) : 'N/A'}</p>
                </div>
              </div>
            )}
          </div>
        )}
      </GlassCard>

      {/* Relation Index Integrity Section */}
      <GlassCard className="mb-6">
        <div className="flex items-center gap-3 mb-6">
          <Link2 className="w-6 h-6 text-amber-400" />
          <div>
            <h2 className="text-xl font-semibold text-white">Relation Index Integrity</h2>
            <p className="text-sm text-gray-400">Check and repair orphaned relations in the global index</p>
          </div>
        </div>

        <div className="space-y-6">
          {/* Verify Relations */}
          <div>
            <h3 className="text-lg font-medium text-white mb-2">Verify Relations</h3>
            <p className="text-sm text-gray-400 mb-4">
              Scan the global relation index for orphaned relations (relations pointing to deleted or tombstoned nodes).
              This helps diagnose "Node not found" errors in GRAPH_TABLE queries.
            </p>

            <button
              onClick={handleVerifyRelations}
              disabled={!selectedRepo || relationVerifyLoading}
              className="flex items-center justify-center gap-2 px-4 py-3 bg-amber-500/20 hover:bg-amber-500/30 border border-amber-500/30 text-amber-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {relationVerifyLoading ? (
                <>
                  <Loader2 className="w-5 h-5 animate-spin" />
                  <span className="font-medium">Verifying...</span>
                </>
              ) : (
                <>
                  <FileText className="w-5 h-5" />
                  <span className="font-medium">Verify Relations</span>
                </>
              )}
            </button>

            {/* Verify Progress */}
            {relationVerifyLoading && relationVerifyProgress > 0 && (
              <div className="mt-4 p-4 bg-amber-500/10 border border-amber-500/20 rounded-lg">
                <div className="flex items-center gap-3 mb-2">
                  <Loader2 className="w-5 h-5 text-amber-400 animate-spin" />
                  <span className="text-white font-medium">Verifying relations...</span>
                  <span className="text-amber-300 ml-auto">{relationVerifyProgress}%</span>
                </div>
                <div className="w-full h-2 bg-white/10 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-amber-400 transition-all duration-300"
                    style={{ width: `${relationVerifyProgress}%` }}
                  />
                </div>
              </div>
            )}

            {/* Verify Result */}
            {relationVerifyResult && (
              <div className={`mt-4 p-4 rounded-lg border ${
                relationVerifyResult.type === 'success'
                  ? 'bg-green-500/10 border-green-500/20'
                  : 'bg-red-500/10 border-red-500/20'
              }`}>
                <div className="flex items-center gap-3 mb-2">
                  {relationVerifyResult.type === 'success' ? (
                    <CheckCircle className="w-5 h-5 text-green-400" />
                  ) : (
                    <XCircle className="w-5 h-5 text-red-400" />
                  )}
                  <span className={`font-medium ${
                    relationVerifyResult.type === 'success' ? 'text-green-300' : 'text-red-300'
                  }`}>
                    {relationVerifyResult.type === 'success' ? 'Verification Complete' : 'Verification Failed'}
                  </span>
                  <button
                    onClick={() => setRelationVerifyResult(null)}
                    className="ml-auto text-gray-400 hover:text-white"
                  >
                    <XCircle className="w-4 h-4" />
                  </button>
                </div>
                {relationVerifyResult.data ? (
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm">
                    <div>
                      <p className="text-gray-400">Relations Scanned</p>
                      <p className="text-white font-semibold">{relationVerifyResult.data.relations_scanned?.toLocaleString() || 0}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Orphaned (Source)</p>
                      <p className="text-white font-semibold">{relationVerifyResult.data.orphaned_source?.toLocaleString() || 0}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Orphaned (Target)</p>
                      <p className="text-white font-semibold">{relationVerifyResult.data.orphaned_target?.toLocaleString() || 0}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Errors</p>
                      <p className="text-white font-semibold">{relationVerifyResult.data.errors?.toLocaleString() || 0}</p>
                    </div>
                  </div>
                ) : (
                  <p className={relationVerifyResult.type === 'success' ? 'text-green-300' : 'text-red-300'}>
                    {relationVerifyResult.message}
                  </p>
                )}
              </div>
            )}
          </div>

          {/* Repair Relations */}
          <div className="pt-6 border-t border-white/10">
            <h3 className="text-lg font-medium text-white mb-2">Repair Orphaned Relations</h3>
            <p className="text-sm text-gray-400 mb-4">
              Write tombstones for orphaned relations in the global index. This fixes "Node not found" errors
              in GRAPH_TABLE queries by marking stale relations as deleted.
            </p>

            <button
              onClick={openRelationRepairConfirmDialog}
              disabled={!selectedRepo || relationRepairLoading}
              className="flex items-center justify-center gap-2 px-4 py-3 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 text-red-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {relationRepairLoading ? (
                <>
                  <Loader2 className="w-5 h-5 animate-spin" />
                  <span className="font-medium">Repairing...</span>
                </>
              ) : (
                <>
                  <Wrench className="w-5 h-5" />
                  <span className="font-medium">Repair Relations</span>
                </>
              )}
            </button>

            {/* Repair Progress */}
            {relationRepairLoading && relationRepairProgress > 0 && (
              <div className="mt-4 p-4 bg-red-500/10 border border-red-500/20 rounded-lg">
                <div className="flex items-center gap-3 mb-2">
                  <Loader2 className="w-5 h-5 text-red-400 animate-spin" />
                  <span className="text-white font-medium">Repairing relations...</span>
                  <span className="text-red-300 ml-auto">{relationRepairProgress}%</span>
                </div>
                <div className="w-full h-2 bg-white/10 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-red-400 transition-all duration-300"
                    style={{ width: `${relationRepairProgress}%` }}
                  />
                </div>
              </div>
            )}

            {/* Repair Result */}
            {relationRepairResult && (
              <div className={`mt-4 p-4 rounded-lg border ${
                relationRepairResult.type === 'success'
                  ? 'bg-green-500/10 border-green-500/20'
                  : 'bg-red-500/10 border-red-500/20'
              }`}>
                <div className="flex items-center gap-3 mb-2">
                  {relationRepairResult.type === 'success' ? (
                    <CheckCircle className="w-5 h-5 text-green-400" />
                  ) : (
                    <XCircle className="w-5 h-5 text-red-400" />
                  )}
                  <span className={`font-medium ${
                    relationRepairResult.type === 'success' ? 'text-green-300' : 'text-red-300'
                  }`}>
                    {relationRepairResult.type === 'success' ? 'Repair Complete' : 'Repair Failed'}
                  </span>
                  <button
                    onClick={() => setRelationRepairResult(null)}
                    className="ml-auto text-gray-400 hover:text-white"
                  >
                    <XCircle className="w-4 h-4" />
                  </button>
                </div>
                {relationRepairResult.data ? (
                  <div className="grid grid-cols-2 md:grid-cols-3 gap-3 text-sm">
                    <div>
                      <p className="text-gray-400">Relations Scanned</p>
                      <p className="text-white font-semibold">{relationRepairResult.data.relations_scanned?.toLocaleString() || 0}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Tombstones Written</p>
                      <p className="text-white font-semibold">{relationRepairResult.data.tombstones_written?.toLocaleString() || 0}</p>
                    </div>
                    <div>
                      <p className="text-gray-400">Errors</p>
                      <p className="text-white font-semibold">{relationRepairResult.data.errors?.toLocaleString() || 0}</p>
                    </div>
                  </div>
                ) : (
                  <p className={relationRepairResult.type === 'success' ? 'text-green-300' : 'text-red-300'}>
                    {relationRepairResult.message}
                  </p>
                )}
              </div>
            )}
          </div>
        </div>
      </GlassCard>

      {/* Property Index Cleanup Section */}
      <GlassCard className="mb-6">
        <div className="flex items-center gap-3 mb-6">
          <Database className="w-6 h-6 text-emerald-400" />
          <div>
            <h2 className="text-xl font-semibold text-white">Property Index Cleanup</h2>
            <p className="text-sm text-gray-400">Remove orphaned property index entries</p>
          </div>
        </div>

        <div className="space-y-4">
          <p className="text-sm text-gray-400">
            Scan property indexes and remove entries that point to non-existent nodes.
            This fixes issues where SQL queries with LIMIT return fewer rows than expected
            due to orphaned index entries.
          </p>

          <div className="p-3 bg-yellow-500/10 border border-yellow-500/20 rounded-lg">
            <p className="text-sm text-yellow-300">
              <strong>Note:</strong> Select a workspace above in the "Reindex Database" section first.
              {reindexWorkspace && (
                <span className="ml-1">Currently selected: <span className="font-semibold">{reindexWorkspace}</span></span>
              )}
            </p>
          </div>

          <button
            onClick={handlePropertyIndexCleanup}
            disabled={!selectedRepo || !reindexWorkspace || propertyIndexCleanupLoading}
            className="flex items-center justify-center gap-2 px-4 py-3 bg-emerald-500/20 hover:bg-emerald-500/30 border border-emerald-500/30 text-emerald-300 rounded-lg transition-all disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {propertyIndexCleanupLoading ? (
              <>
                <Loader2 className="w-5 h-5 animate-spin" />
                <span className="font-medium">Cleaning up...</span>
              </>
            ) : (
              <>
                <Trash2 className="w-5 h-5" />
                <span className="font-medium">Cleanup Property Indexes</span>
              </>
            )}
          </button>

          {/* Result */}
          {propertyIndexCleanupResult && (
            <div className={`p-4 rounded-lg border ${
              propertyIndexCleanupResult.type === 'success'
                ? 'bg-green-500/10 border-green-500/20'
                : 'bg-red-500/10 border-red-500/20'
            }`}>
              <div className="flex items-center gap-3 mb-2">
                {propertyIndexCleanupResult.type === 'success' ? (
                  <CheckCircle className="w-5 h-5 text-green-400" />
                ) : (
                  <XCircle className="w-5 h-5 text-red-400" />
                )}
                <span className={`font-medium ${
                  propertyIndexCleanupResult.type === 'success' ? 'text-green-300' : 'text-red-300'
                }`}>
                  {propertyIndexCleanupResult.type === 'success' ? 'Cleanup Complete' : 'Cleanup Failed'}
                </span>
                <button
                  onClick={() => setPropertyIndexCleanupResult(null)}
                  className="ml-auto text-gray-400 hover:text-white"
                >
                  <XCircle className="w-4 h-4" />
                </button>
              </div>
              {propertyIndexCleanupResult.data ? (
                <div className="grid grid-cols-2 md:grid-cols-5 gap-3 text-sm">
                  <div>
                    <p className="text-gray-400">Entries Scanned</p>
                    <p className="text-white font-semibold">{propertyIndexCleanupResult.data.entries_scanned?.toLocaleString() || 0}</p>
                  </div>
                  <div>
                    <p className="text-gray-400">Orphaned Found</p>
                    <p className="text-yellow-400 font-semibold">{propertyIndexCleanupResult.data.orphaned_found?.toLocaleString() || 0}</p>
                  </div>
                  <div>
                    <p className="text-gray-400">Orphaned Deleted</p>
                    <p className="text-green-400 font-semibold">{propertyIndexCleanupResult.data.orphaned_deleted?.toLocaleString() || 0}</p>
                  </div>
                  <div>
                    <p className="text-gray-400">Duration</p>
                    <p className="text-white font-semibold">{propertyIndexCleanupResult.data.duration_ms}ms</p>
                  </div>
                  <div>
                    <p className="text-gray-400">Workspaces</p>
                    <p className="text-white font-semibold">{propertyIndexCleanupResult.data.workspaces_processed || 1}</p>
                  </div>
                </div>
              ) : (
                <p className={propertyIndexCleanupResult.type === 'success' ? 'text-green-300' : 'text-red-300'}>
                  {propertyIndexCleanupResult.message}
                </p>
              )}
            </div>
          )}
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
          <div ref={jobsContainerRef} className="space-y-3 max-h-96 overflow-y-auto">
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

      {/* Regenerate Embeddings Dialog with Force Checkbox */}
      {showRegenerateDialog && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
          <div className="glass-dark rounded-xl max-w-lg w-full p-6 animate-slide-in">
            <div className="flex items-center gap-3 mb-4">
              <AlertCircle className="w-6 h-6 text-orange-400" />
              <h2 className="text-xl font-bold text-white">Regenerate Embeddings</h2>
            </div>

            <div className="text-gray-300 mb-4 space-y-2">
              <p>This will scan all embeddings in <span className="text-white font-semibold">{selectedRepo}</span> and queue jobs to regenerate embeddings.</p>

              <p className="text-sm">This operation will:</p>
              <ul className="text-sm list-disc list-inside space-y-1 ml-2">
                <li>Scan all embeddings in RocksDB</li>
                <li>Check dimensions against tenant config</li>
                <li>Queue embedding jobs for matching nodes</li>
                <li>Call your embedding provider API (OpenAI, etc.)</li>
              </ul>

              <p className="text-sm mt-3">This is useful when:</p>
              <ul className="text-sm list-disc list-inside space-y-1 ml-2">
                <li>You changed the embedding model dimensions</li>
                <li>You want to regenerate embeddings with a new model</li>
                <li>You need to re-normalize existing embeddings</li>
              </ul>

              <p className="text-sm text-yellow-400 mt-3">
                Note: This operation cannot run concurrently. Only one regeneration can run at a time per tenant.
              </p>
            </div>

            <div className="mb-6 p-3 bg-orange-500/10 border border-orange-500/20 rounded-lg">
              <label className="flex items-start gap-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={forceRegenerate}
                  onChange={(e) => setForceRegenerate(e.target.checked)}
                  className="mt-1 w-4 h-4 rounded border-orange-500/30 bg-white/5 text-orange-500 focus:ring-2 focus:ring-orange-400/20 transition-all"
                />
                <div className="flex-1">
                  <p className="text-sm font-medium text-orange-300">
                    Force regenerate all embeddings
                  </p>
                  <p className="text-xs text-orange-400/80 mt-1">
                    Regenerate ALL embeddings, even if dimensions already match. Use this to re-normalize existing embeddings or regenerate with updated models.
                  </p>
                </div>
              </label>
            </div>

            <div className="flex gap-3 justify-end">
              <button
                onClick={() => {
                  setShowRegenerateDialog(false)
                  setForceRegenerate(false)
                }}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleRegenerateConfirm}
                className="px-4 py-2 bg-orange-500 hover:bg-orange-600 text-white rounded-lg transition-colors"
              >
                Regenerate Embeddings
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Reindex Confirmation Dialog */}
      {showReindexConfirm && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
          <div className="glass-dark rounded-xl max-w-md w-full p-6 animate-slide-in">
            <div className="flex items-center gap-3 mb-4">
              <AlertCircle className="w-6 h-6 text-yellow-400" />
              <h2 className="text-xl font-bold text-white">Confirm Reindex</h2>
            </div>

            <p className="text-gray-300 mb-6">
              This will rebuild all selected indexes for the workspace <span className="text-white font-semibold">{reindexWorkspace}</span>.
              Database operations may be slower during reindexing. This operation cannot be cancelled once started. Continue?
            </p>

            <div className="flex gap-3 justify-end">
              <button
                onClick={() => setShowReindexConfirm(false)}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={handleStartReindex}
                className="px-4 py-2 bg-yellow-500 hover:bg-yellow-600 text-white rounded-lg transition-colors"
              >
                Start Reindex
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Relation Repair Confirmation Dialog */}
      {showRelationRepairConfirm && (
        <div className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center p-8 z-50">
          <div className="glass-dark rounded-xl max-w-md w-full p-6 animate-slide-in">
            <div className="flex items-center gap-3 mb-4">
              <AlertCircle className="w-6 h-6 text-red-400" />
              <h2 className="text-xl font-bold text-white">Confirm Relation Repair</h2>
            </div>

            <div className="text-gray-300 mb-6 space-y-3">
              <p>
                This will scan the global relation index and write tombstones for any relations pointing to deleted or tombstoned nodes.
              </p>
              <p className="text-sm text-yellow-400">
                This operation cannot be undone. Orphaned relations will be permanently marked as deleted.
              </p>
            </div>

            <div className="flex gap-3 justify-end">
              <button
                onClick={() => setShowRelationRepairConfirm(false)}
                className="px-4 py-2 bg-white/10 hover:bg-white/20 text-white rounded-lg transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={() => {
                  setShowRelationRepairConfirm(false)
                  handleRepairRelations()
                }}
                className="px-4 py-2 bg-red-500 hover:bg-red-600 text-white rounded-lg transition-colors"
              >
                Repair Relations
              </button>
            </div>
          </div>
        </div>
      )}

      <ToastContainer toasts={toasts} onClose={closeToast} />
    </div>
  )
}
