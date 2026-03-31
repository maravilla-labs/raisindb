import { useState, useEffect } from 'react'
import { Database, FileText, Wrench, Trash2, HardDrive, Package, RefreshCw, XCircle, Link2 } from 'lucide-react'
import GlassCard from '../../components/GlassCard'
import WorkspaceSelector from '../../components/management/WorkspaceSelector'
import ActionButton from '../../components/management/ActionButton'
import ActionResult from '../../components/management/ActionResult'
import { managementApi, formatBytes, formatDuration, sseManager, JobEvent, databaseManagementApi } from '../../api/management'
import ConfirmDialog from '../../components/ConfirmDialog'

export default function Actions() {
  const [selectedWorkspace, setSelectedWorkspace] = useState('')
  const repo = 'main' // TODO: Get from route params or context

  // Loading states
  const [compactLoading, setCompactLoading] = useState(false)
  const [integrityLoading, setIntegrityLoading] = useState(false)
  const [verifyLoading, setVerifyLoading] = useState(false)
  const [rebuildLoading, setRebuildLoading] = useState(false)
  const [cleanupLoading, setCleanupLoading] = useState(false)
  const [backupLoading, setBackupLoading] = useState(false)

  // Results
  const [compactResult, setCompactResult] = useState<any>(null)
  const [integrityResult, setIntegrityResult] = useState<any>(null)
  const [verifyResult, setVerifyResult] = useState<any>(null)
  const [rebuildResult, setRebuildResult] = useState<any>(null)
  const [cleanupResult, setCleanupResult] = useState<any>(null)
  const [backupResult, setBackupResult] = useState<any>(null)

  // Form inputs
  const [indexType, setIndexType] = useState('all')
  const [backupPath, setBackupPath] = useState('./backup')

  // Integrity check job tracking
  const [integrityJobId, setIntegrityJobId] = useState<string | null>(null)
  const [integrityProgress, setIntegrityProgress] = useState<number>(0)

  // Repair functionality
  const [selectedIssues, setSelectedIssues] = useState<Set<number>>(new Set())
  const [repairLoading, setRepairLoading] = useState(false)
  const [repairResult, setRepairResult] = useState<any>(null)

  // Job tracking for all operations
  const [compactJobId, setCompactJobId] = useState<string | null>(null)
  const [compactProgress, setCompactProgress] = useState<number>(0)

  const [verifyJobId, setVerifyJobId] = useState<string | null>(null)
  const [verifyProgress, setVerifyProgress] = useState<number>(0)

  const [rebuildJobIds, setRebuildJobIds] = useState<Map<string, string>>(new Map())
  const [rebuildProgress, setRebuildProgress] = useState<Map<string, number>>(new Map())

  const [cleanupJobId, setCleanupJobId] = useState<string | null>(null)
  const [cleanupProgress, setCleanupProgress] = useState<number>(0)

  const [backupJobId, setBackupJobId] = useState<string | null>(null)
  const [backupProgress, setBackupProgress] = useState<number>(0)

  const [repairJobId, setRepairJobId] = useState<string | null>(null)
  const [repairProgress, setRepairProgress] = useState<number>(0)
  const [repairAllConfirm, setRepairAllConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)

  // Relation integrity states
  const [relationVerifyLoading, setRelationVerifyLoading] = useState(false)
  const [relationVerifyJobId, setRelationVerifyJobId] = useState<string | null>(null)
  const [relationVerifyProgress, setRelationVerifyProgress] = useState<number>(0)
  const [relationVerifyResult, setRelationVerifyResult] = useState<any>(null)

  const [relationRepairLoading, setRelationRepairLoading] = useState(false)
  const [relationRepairJobId, setRelationRepairJobId] = useState<string | null>(null)
  const [relationRepairProgress, setRelationRepairProgress] = useState<number>(0)
  const [relationRepairResult, setRelationRepairResult] = useState<any>(null)
  const [relationRepairConfirm, setRelationRepairConfirm] = useState<{ message: string; onConfirm: () => void } | null>(null)

  // Property index orphan cleanup states
  const [propertyIndexCleanupLoading, setPropertyIndexCleanupLoading] = useState(false)
  const [propertyIndexCleanupResult, setPropertyIndexCleanupResult] = useState<any>(null)

  // Load last report and check for running jobs when workspace changes
  useEffect(() => {
    if (!selectedWorkspace) {
      setIntegrityResult(null)
      return
    }

    const loadLastReport = async () => {
      try {
        // Try to fetch the last report
        const reportResponse = await managementApi.getLastIntegrityReport(selectedWorkspace)
        if (reportResponse.success && reportResponse.data) {
          setIntegrityResult({ type: 'success', data: reportResponse.data })
        }
      } catch (error) {
        // No last report found - this is OK
        console.log('No previous integrity report found')
      }

      // Check for running integrity jobs for this workspace
      try {
        const jobsResponse = await managementApi.listJobs()
        if (jobsResponse.success && jobsResponse.data) {
          const runningJob = jobsResponse.data.find(
            (job) =>
              job.job_type === 'IntegrityScan' &&
              job.tenant === selectedWorkspace &&
              (job.status === 'Running' || job.status === 'Scheduled')
          )

          if (runningJob) {
            setIntegrityJobId(runningJob.id)
            setIntegrityLoading(true)
            setIntegrityProgress(runningJob.progress || 0)
          }
        }
      } catch (error) {
        console.error('Failed to check for running jobs:', error)
      }
    }

    loadLastReport()
  }, [selectedWorkspace])

  const handleTriggerCompaction = async () => {
    setCompactLoading(true)
    setCompactResult(null)
    setCompactProgress(0)

    try {
      // Start the background job
      const response = await managementApi.startCompaction()
      if (response.success && response.data) {
        setCompactJobId(response.data)
        // Job started successfully, now SSE will handle updates
      } else {
        setCompactResult({ type: 'error', message: response.error || 'Failed to start compaction' })
        setCompactLoading(false)
      }
    } catch (error) {
      setCompactResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setCompactLoading(false)
    }
  }

  const handleCancelCompaction = async () => {
    if (!compactJobId) return

    try {
      await managementApi.cancelJob(compactJobId)
      setCompactLoading(false)
      setCompactJobId(null)
      setCompactProgress(0)
      setCompactResult({ type: 'warning', message: 'Compaction cancelled' })
    } catch (error) {
      console.error('Failed to cancel compaction:', error)
    }
  }

  // Connect to SSE for integrity check job updates
  useEffect(() => {
    if (!integrityJobId) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        // Only process events for our job
        if (event.job_id !== integrityJobId) return

        // Update progress
        if (event.progress !== null && event.progress !== undefined) {
          setIntegrityProgress(event.progress)
        }

        // Handle completion
        if (event.status === 'Completed') {
          // Fetch job info to get the result
          managementApi.getJobInfo(integrityJobId).then(response => {
            if (response.success && response.data) {
              const jobInfo = response.data
              if (jobInfo.result) {
                setIntegrityResult({ type: 'success', data: jobInfo.result })
              }
            }
            setIntegrityLoading(false)
            setIntegrityJobId(null)
            setIntegrityProgress(0)
          })
        }

        // Handle failure
        if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
          setIntegrityResult({ type: 'error', message: event.error || 'Integrity check failed' })
          setIntegrityLoading(false)
          setIntegrityJobId(null)
          setIntegrityProgress(0)
        }
      },
      onError: () => {
        // SSE connection error, but don't stop the job
        console.error('SSE connection error for integrity check')
      }
    })

    return cleanup
  }, [integrityJobId])

  // Unified SSE handler for all other operations
  useEffect(() => {
    // Collect all active job IDs
    const activeJobIds = [
      compactJobId,
      verifyJobId,
      cleanupJobId,
      backupJobId,
      repairJobId,
      relationVerifyJobId,
      relationRepairJobId,
      ...Array.from(rebuildJobIds.values())
    ].filter(Boolean) as string[]

    if (activeJobIds.length === 0) return

    const cleanup = sseManager.connect('jobs', {
      onJobUpdate: (event: JobEvent) => {
        // Handle compaction jobs
        if (event.job_id === compactJobId) {
          if (event.progress !== null && event.progress !== undefined) {
            setCompactProgress(event.progress)
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(compactJobId).then(response => {
              if (response.success && response.data?.result) {
                setCompactResult({ type: 'success', data: response.data.result })
              }
              setCompactLoading(false)
              setCompactJobId(null)
              setCompactProgress(0)
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setCompactResult({ type: 'error', message: event.error || 'Compaction failed' })
            setCompactLoading(false)
            setCompactJobId(null)
            setCompactProgress(0)
          }
        }

        // Handle verify jobs
        if (event.job_id === verifyJobId) {
          if (event.progress !== null && event.progress !== undefined) {
            setVerifyProgress(event.progress)
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(verifyJobId).then(response => {
              if (response.success && response.data?.result) {
                setVerifyResult({ type: 'success', data: response.data.result })
              }
              setVerifyLoading(false)
              setVerifyJobId(null)
              setVerifyProgress(0)
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setVerifyResult({ type: 'error', message: event.error || 'Index verification failed' })
            setVerifyLoading(false)
            setVerifyJobId(null)
            setVerifyProgress(0)
          }
        }

        // Handle cleanup jobs
        if (event.job_id === cleanupJobId) {
          if (event.progress !== null && event.progress !== undefined) {
            setCleanupProgress(event.progress)
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(cleanupJobId).then(response => {
              if (response.success && response.data?.result) {
                setCleanupResult({ type: 'success', data: response.data.result })
              }
              setCleanupLoading(false)
              setCleanupJobId(null)
              setCleanupProgress(0)
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setCleanupResult({ type: 'error', message: event.error || 'Cleanup failed' })
            setCleanupLoading(false)
            setCleanupJobId(null)
            setCleanupProgress(0)
          }
        }

        // Handle backup jobs
        if (event.job_id === backupJobId) {
          if (event.progress !== null && event.progress !== undefined) {
            setBackupProgress(event.progress)
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(backupJobId).then(response => {
              if (response.success && response.data?.result) {
                setBackupResult({ type: 'success', data: response.data.result })
              }
              setBackupLoading(false)
              setBackupJobId(null)
              setBackupProgress(0)
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setBackupResult({ type: 'error', message: event.error || 'Backup failed' })
            setBackupLoading(false)
            setBackupJobId(null)
            setBackupProgress(0)
          }
        }

        // Handle repair jobs
        if (event.job_id === repairJobId) {
          if (event.progress !== null && event.progress !== undefined) {
            setRepairProgress(event.progress)
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(repairJobId).then(response => {
              if (response.success && response.data?.result) {
                setRepairResult({ type: 'success', data: response.data.result })
                setSelectedIssues(new Set())
                // Reload integrity check to see results
                handleCheckIntegrity()
              }
              setRepairLoading(false)
              setRepairJobId(null)
              setRepairProgress(0)
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setRepairResult({ type: 'error', message: event.error || 'Repair failed' })
            setRepairLoading(false)
            setRepairJobId(null)
            setRepairProgress(0)
          }
        }

        // Handle rebuild jobs
        const rebuildJobId = Array.from(rebuildJobIds.entries()).find(([_, jid]) => jid === event.job_id)
        if (rebuildJobId) {
          const [indexType, jobId] = rebuildJobId

          if (event.progress !== null && event.progress !== undefined) {
            setRebuildProgress(prev => new Map(prev).set(indexType, event.progress!))
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(jobId).then(response => {
              if (response.success && response.data?.result) {
                setRebuildResult({ type: 'success', data: response.data.result })
              }
              setRebuildLoading(false)
              setRebuildJobIds(prev => {
                const next = new Map(prev)
                next.delete(indexType)
                return next
              })
              setRebuildProgress(prev => {
                const next = new Map(prev)
                next.delete(indexType)
                return next
              })
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setRebuildResult({ type: 'error', message: event.error || 'Index rebuild failed' })
            setRebuildLoading(false)
            setRebuildJobIds(prev => {
              const next = new Map(prev)
              next.delete(indexType)
              return next
            })
            setRebuildProgress(prev => {
              const next = new Map(prev)
              next.delete(indexType)
              return next
            })
          }
        }

        // Handle relation verify jobs
        if (event.job_id === relationVerifyJobId) {
          if (event.progress !== null && event.progress !== undefined) {
            setRelationVerifyProgress(event.progress)
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(relationVerifyJobId).then(response => {
              if (response.success && response.data?.result) {
                setRelationVerifyResult({ type: 'success', data: response.data.result })
              } else {
                setRelationVerifyResult({ type: 'success', message: 'Relation verification completed' })
              }
              setRelationVerifyLoading(false)
              setRelationVerifyJobId(null)
              setRelationVerifyProgress(0)
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setRelationVerifyResult({ type: 'error', message: event.error || 'Relation verification failed' })
            setRelationVerifyLoading(false)
            setRelationVerifyJobId(null)
            setRelationVerifyProgress(0)
          }
        }

        // Handle relation repair jobs
        if (event.job_id === relationRepairJobId) {
          if (event.progress !== null && event.progress !== undefined) {
            setRelationRepairProgress(event.progress)
          }

          if (event.status === 'Completed') {
            managementApi.getJobInfo(relationRepairJobId).then(response => {
              if (response.success && response.data?.result) {
                setRelationRepairResult({ type: 'success', data: response.data.result })
              } else {
                setRelationRepairResult({ type: 'success', message: 'Relation repair completed' })
              }
              setRelationRepairLoading(false)
              setRelationRepairJobId(null)
              setRelationRepairProgress(0)
            })
          }

          if (event.status === 'Failed' || (typeof event.status === 'object' && 'Failed' in event.status)) {
            setRelationRepairResult({ type: 'error', message: event.error || 'Relation repair failed' })
            setRelationRepairLoading(false)
            setRelationRepairJobId(null)
            setRelationRepairProgress(0)
          }
        }
      },
      onError: () => {
        console.error('SSE connection error for job updates')
      }
    })

    return cleanup
  }, [compactJobId, verifyJobId, cleanupJobId, backupJobId, repairJobId, rebuildJobIds, relationVerifyJobId, relationRepairJobId])

  const handleCheckIntegrity = async () => {
    if (!selectedWorkspace) {
      setIntegrityResult({ type: 'warning', message: 'Please select a workspace' })
      return
    }

    setIntegrityLoading(true)
    setIntegrityResult(null)
    setIntegrityProgress(0)

    try {
      // Start the background job
      const response = await managementApi.startIntegrityCheck(selectedWorkspace)
      if (response.success && response.data) {
        setIntegrityJobId(response.data)
        // Job started successfully, now SSE will handle updates
      } else {
        setIntegrityResult({ type: 'error', message: response.error || 'Failed to start integrity check' })
        setIntegrityLoading(false)
      }
    } catch (error) {
      setIntegrityResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setIntegrityLoading(false)
    }
  }

  const handleCancelIntegrityCheck = async () => {
    if (!integrityJobId) return

    try {
      await managementApi.cancelJob(integrityJobId)
      setIntegrityLoading(false)
      setIntegrityJobId(null)
      setIntegrityProgress(0)
      setIntegrityResult({ type: 'warning', message: 'Integrity check cancelled' })
    } catch (error) {
      console.error('Failed to cancel integrity check:', error)
    }
  }

  const handleRepairSelected = async () => {
    if (!selectedWorkspace || !integrityResult || selectedIssues.size === 0) return

    const issuesToRepair = integrityResult.data.issues_found.filter((_: any, idx: number) => selectedIssues.has(idx))

    setRepairLoading(true)
    setRepairResult(null)
    setRepairProgress(0)

    try {
      const response = await managementApi.startRepair(selectedWorkspace, issuesToRepair)
      if (response.success && response.data) {
        setRepairJobId(response.data)
        // Job started successfully, now SSE will handle updates
      } else {
        setRepairResult({ type: 'error', message: response.error || 'Failed to start repair' })
        setRepairLoading(false)
      }
    } catch (error) {
      setRepairResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setRepairLoading(false)
    }
  }

  const handleRepairAll = async () => {
    if (!selectedWorkspace || !integrityResult) return

    setRepairAllConfirm({
      message: `Are you sure you want to auto-repair all ${integrityResult.data.issues_found.length} issues?\n\nThis will:\n- Delete orphaned nodes\n- Rebuild child order indexes\n- Remove broken references\n- Remove duplicate children\n\nThis operation cannot be undone.`,
      onConfirm: async () => {
        setRepairLoading(true)
        setRepairResult(null)
        setRepairProgress(0)

        try {
          const response = await managementApi.startRepair(selectedWorkspace, integrityResult.data.issues_found)
          if (response.success && response.data) {
            setRepairJobId(response.data)
            // Job started successfully, now SSE will handle updates
          } else {
            setRepairResult({ type: 'error', message: response.error || 'Failed to start repair' })
            setRepairLoading(false)
          }
        } catch (error) {
          setRepairResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
          setRepairLoading(false)
        }
      }
    })
  }

  const handleCancelRepair = async () => {
    if (!repairJobId) return

    try {
      await managementApi.cancelJob(repairJobId)
      setRepairLoading(false)
      setRepairJobId(null)
      setRepairProgress(0)
      setRepairResult({ type: 'warning', message: 'Repair cancelled' })
    } catch (error) {
      console.error('Failed to cancel repair:', error)
    }
  }

  const toggleIssueSelection = (idx: number) => {
    const newSelection = new Set(selectedIssues)
    if (newSelection.has(idx)) {
      newSelection.delete(idx)
    } else {
      newSelection.add(idx)
    }
    setSelectedIssues(newSelection)
  }

  const toggleSelectAll = () => {
    if (!integrityResult) return

    if (selectedIssues.size === integrityResult.data.issues_found.length) {
      setSelectedIssues(new Set<number>())
    } else {
      const allIndices = new Set<number>(integrityResult.data.issues_found.map((_: any, idx: number) => idx))
      setSelectedIssues(allIndices)
    }
  }

  const handleVerifyIndexes = async () => {
    if (!selectedWorkspace) {
      setVerifyResult({ type: 'warning', message: 'Please select a workspace' })
      return
    }

    setVerifyLoading(true)
    setVerifyResult(null)
    setVerifyProgress(0)

    try {
      const response = await managementApi.startVerifyIndexes(selectedWorkspace)
      if (response.success && response.data) {
        setVerifyJobId(response.data)
        // Job started successfully, now SSE will handle updates
      } else {
        setVerifyResult({ type: 'error', message: response.error || 'Failed to start index verification' })
        setVerifyLoading(false)
      }
    } catch (error) {
      setVerifyResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setVerifyLoading(false)
    }
  }

  const handleCancelVerify = async () => {
    if (!verifyJobId) return

    try {
      await managementApi.cancelJob(verifyJobId)
      setVerifyLoading(false)
      setVerifyJobId(null)
      setVerifyProgress(0)
      setVerifyResult({ type: 'warning', message: 'Index verification cancelled' })
    } catch (error) {
      console.error('Failed to cancel index verification:', error)
    }
  }

  // Helper to check if rebuild can start (concurrency management)
  const canStartRebuild = (requestedType: string): { canStart: boolean, reason?: string } => {
    if (rebuildJobIds.size === 0) return { canStart: true }

    if (requestedType === 'all') {
      // Can't start 'all' if ANY rebuild is running
      if (rebuildJobIds.size > 0) {
        const runningTypes = Array.from(rebuildJobIds.keys()).join(', ')
        return {
          canStart: false,
          reason: `Cannot rebuild all indexes while ${runningTypes} rebuild is in progress`
        }
      }
    }

    // Check if 'all' is running
    if (rebuildJobIds.has('all')) {
      return {
        canStart: false,
        reason: 'Cannot start rebuild while all indexes are being rebuilt'
      }
    }

    // Check if this specific type is running
    if (rebuildJobIds.has(requestedType)) {
      return {
        canStart: false,
        reason: `${requestedType} index rebuild is already in progress`
      }
    }

    // Different specific type - OK!
    return { canStart: true }
  }

  const handleRebuildIndexes = async () => {
    if (!selectedWorkspace) {
      setRebuildResult({ type: 'warning', message: 'Please select a workspace' })
      return
    }

    // Check concurrency
    const { canStart, reason } = canStartRebuild(indexType)
    if (!canStart) {
      setRebuildResult({ type: 'warning', message: reason || 'Cannot start rebuild' })
      return
    }

    setRebuildLoading(true)
    setRebuildResult(null)

    try {
      const response = await managementApi.startRebuildIndexes(selectedWorkspace, indexType)
      if (response.success && response.data) {
        setRebuildJobIds(prev => new Map(prev).set(indexType, response.data!))
        setRebuildProgress(prev => new Map(prev).set(indexType, 0))
        // Job started successfully, now SSE will handle updates
      } else {
        setRebuildResult({ type: 'error', message: response.error || 'Failed to start index rebuild' })
        setRebuildLoading(false)
      }
    } catch (error) {
      setRebuildResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setRebuildLoading(false)
    }
  }

  const handleCancelRebuild = async (type: string) => {
    const jobId = rebuildJobIds.get(type)
    if (!jobId) return

    try {
      await managementApi.cancelJob(jobId)
      setRebuildLoading(false)
      setRebuildJobIds(prev => {
        const next = new Map(prev)
        next.delete(type)
        return next
      })
      setRebuildProgress(prev => {
        const next = new Map(prev)
        next.delete(type)
        return next
      })
      setRebuildResult({ type: 'warning', message: `${type} index rebuild cancelled` })
    } catch (error) {
      console.error('Failed to cancel rebuild:', error)
    }
  }

  const handleCleanupOrphans = async () => {
    if (!selectedWorkspace) {
      setCleanupResult({ type: 'warning', message: 'Please select a workspace' })
      return
    }

    setCleanupLoading(true)
    setCleanupResult(null)
    setCleanupProgress(0)

    try {
      const response = await managementApi.startCleanupOrphans(selectedWorkspace)
      if (response.success && response.data) {
        setCleanupJobId(response.data)
        // Job started successfully, now SSE will handle updates
      } else {
        setCleanupResult({ type: 'error', message: response.error || 'Failed to start orphan cleanup' })
        setCleanupLoading(false)
      }
    } catch (error) {
      setCleanupResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setCleanupLoading(false)
    }
  }

  const handleCancelCleanup = async () => {
    if (!cleanupJobId) return

    try {
      await managementApi.cancelJob(cleanupJobId)
      setCleanupLoading(false)
      setCleanupJobId(null)
      setCleanupProgress(0)
      setCleanupResult({ type: 'warning', message: 'Cleanup cancelled' })
    } catch (error) {
      console.error('Failed to cancel cleanup:', error)
    }
  }

  // Property Index Orphan Cleanup - removes index entries pointing to non-existent nodes
  // This fixes issues where LIMIT queries return 0 rows due to orphaned index entries
  const handlePropertyIndexCleanup = async () => {
    if (!selectedWorkspace) {
      setPropertyIndexCleanupResult({ type: 'warning', message: 'Please select a workspace' })
      return
    }

    setPropertyIndexCleanupLoading(true)
    setPropertyIndexCleanupResult(null)

    try {
      const response = await managementApi.cleanupPropertyIndexOrphans(selectedWorkspace)
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
    } catch (error) {
      setPropertyIndexCleanupResult({
        type: 'error',
        message: error instanceof Error ? error.message : 'Unknown error'
      })
    } finally {
      setPropertyIndexCleanupLoading(false)
    }
  }

  const handleBackupAll = async () => {
    if (!backupPath) {
      setBackupResult({ type: 'warning', message: 'Please enter a backup path' })
      return
    }

    setBackupLoading(true)
    setBackupResult(null)
    setBackupProgress(0)

    try {
      const response = await managementApi.startBackup(backupPath)
      if (response.success && response.data) {
        setBackupJobId(response.data)
        // Job started successfully, now SSE will handle updates
      } else {
        setBackupResult({ type: 'error', message: response.error || 'Failed to start backup' })
        setBackupLoading(false)
      }
    } catch (error) {
      setBackupResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setBackupLoading(false)
    }
  }

  const handleCancelBackup = async () => {
    if (!backupJobId) return

    try {
      await managementApi.cancelJob(backupJobId)
      setBackupLoading(false)
      setBackupJobId(null)
      setBackupProgress(0)
      setBackupResult({ type: 'warning', message: 'Backup cancelled' })
    } catch (error) {
      console.error('Failed to cancel backup:', error)
    }
  }

  // Relation integrity handlers
  const handleVerifyRelations = async () => {
    if (!selectedWorkspace) {
      setRelationVerifyResult({ type: 'warning', message: 'Please select a workspace' })
      return
    }

    setRelationVerifyLoading(true)
    setRelationVerifyResult(null)
    setRelationVerifyProgress(0)

    try {
      const response = await databaseManagementApi.relationsVerify(selectedWorkspace, repo)
      if (response.job_id) {
        setRelationVerifyJobId(response.job_id)
      } else {
        setRelationVerifyResult({ type: 'error', message: 'Failed to start relation verification' })
        setRelationVerifyLoading(false)
      }
    } catch (error) {
      setRelationVerifyResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
      setRelationVerifyLoading(false)
    }
  }

  const handleCancelRelationVerify = async () => {
    if (!relationVerifyJobId) return

    try {
      await managementApi.cancelJob(relationVerifyJobId)
      setRelationVerifyLoading(false)
      setRelationVerifyJobId(null)
      setRelationVerifyProgress(0)
      setRelationVerifyResult({ type: 'warning', message: 'Relation verification cancelled' })
    } catch (error) {
      console.error('Failed to cancel relation verification:', error)
    }
  }

  const handleRepairRelations = async () => {
    if (!selectedWorkspace) {
      setRelationRepairResult({ type: 'warning', message: 'Please select a workspace' })
      return
    }

    setRelationRepairConfirm({
      message: `Are you sure you want to repair orphaned relations?\n\nThis will scan the global relation index and write tombstones for any relations pointing to deleted/tombstoned nodes.\n\nThis operation cannot be undone.`,
      onConfirm: async () => {
        setRelationRepairLoading(true)
        setRelationRepairResult(null)
        setRelationRepairProgress(0)

        try {
          const response = await databaseManagementApi.relationsRepair(selectedWorkspace, repo)
          if (response.job_id) {
            setRelationRepairJobId(response.job_id)
          } else {
            setRelationRepairResult({ type: 'error', message: 'Failed to start relation repair' })
            setRelationRepairLoading(false)
          }
        } catch (error) {
          setRelationRepairResult({ type: 'error', message: error instanceof Error ? error.message : 'Unknown error' })
          setRelationRepairLoading(false)
        }
      }
    })
  }

  const handleCancelRelationRepair = async () => {
    if (!relationRepairJobId) return

    try {
      await managementApi.cancelJob(relationRepairJobId)
      setRelationRepairLoading(false)
      setRelationRepairJobId(null)
      setRelationRepairProgress(0)
      setRelationRepairResult({ type: 'warning', message: 'Relation repair cancelled' })
    } catch (error) {
      console.error('Failed to cancel relation repair:', error)
    }
  }

  return (
    <div className="pt-8">
      {/* Header */}
      <div className="mb-8">
        <h1 className="text-3xl font-bold text-white mb-2">Management Actions</h1>
        <p className="text-zinc-400">Trigger manual maintenance and administrative operations</p>
      </div>

      {/* Workspace Selector */}
      <div className="mb-8">
        <WorkspaceSelector
          value={selectedWorkspace}
          onChange={setSelectedWorkspace}
          repo={repo}
          label="Select Workspace for Operations"
        />
      </div>

      {/* Maintenance Section */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4 flex items-center gap-2">
          <Wrench className="w-5 h-5 text-primary-400" />
          Maintenance
        </h2>
        <GlassCard>
          <h3 className="text-lg font-semibold text-white mb-2">Database Compaction</h3>
          <p className="text-zinc-400 text-sm mb-4">
            Trigger manual compaction to optimize disk usage and improve performance
          </p>

          <div className="flex gap-2">
            <ActionButton
              onClick={handleTriggerCompaction}
              loading={compactLoading}
              icon={HardDrive}
            >
              Trigger Compaction
            </ActionButton>

            {compactLoading && compactJobId && (
              <button
                onClick={handleCancelCompaction}
                className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
              >
                <XCircle className="w-4 h-4" />
                Cancel
              </button>
            )}
          </div>

          {compactLoading && compactProgress > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between text-sm mb-2">
                <div className="flex items-center gap-2 text-zinc-300">
                  <RefreshCw className="w-4 h-4 animate-spin" />
                  <span>Compacting database...</span>
                </div>
                <span className="text-white font-medium">{Math.round(compactProgress * 100)}%</span>
              </div>
              <div className="w-full bg-white/10 rounded-full h-2">
                <div
                  className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${Math.round(compactProgress * 100)}%` }}
                ></div>
              </div>
            </div>
          )}

          {compactResult && (
            <div className="mt-4">
              {compactResult.type === 'success' ? (
                <ActionResult
                  type="success"
                  title="Compaction Completed"
                  details={
                    <div className="text-sm text-green-300 space-y-1">
                      <p>Before: {formatBytes(compactResult.data.bytes_before)}</p>
                      <p>After: {formatBytes(compactResult.data.bytes_after)}</p>
                      <p>Saved: {formatBytes(compactResult.data.bytes_before - compactResult.data.bytes_after)}</p>
                      <p>Duration: {formatDuration(compactResult.data.duration_ms)}</p>
                      <p>Files compacted: {compactResult.data.files_compacted}</p>
                    </div>
                  }
                  onDismiss={() => setCompactResult(null)}
                />
              ) : (
                <ActionResult
                  type={compactResult.type}
                  title="Compaction Failed"
                  message={compactResult.message}
                  onDismiss={() => setCompactResult(null)}
                />
              )}
            </div>
          )}
        </GlassCard>
      </div>

      {/* Integrity Section */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4 flex items-center gap-2">
          <FileText className="w-5 h-5 text-primary-400" />
          Data Integrity
        </h2>
        <GlassCard>
          <h3 className="text-lg font-semibold text-white mb-2">Integrity Check</h3>
          <p className="text-zinc-400 text-sm mb-4">
            Scan workspace data for integrity issues like orphaned nodes, broken references, and corrupted data
          </p>

          <div className="flex gap-2">
            <ActionButton
              onClick={handleCheckIntegrity}
              loading={integrityLoading}
              icon={FileText}
              disabled={!selectedWorkspace}
            >
              Check Integrity
            </ActionButton>

            {integrityLoading && integrityJobId && (
              <button
                onClick={handleCancelIntegrityCheck}
                className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
              >
                <XCircle className="w-4 h-4" />
                Cancel
              </button>
            )}
          </div>

          {integrityLoading && integrityProgress > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between text-sm mb-2">
                <div className="flex items-center gap-2 text-zinc-300">
                  <RefreshCw className="w-4 h-4 animate-spin" />
                  <span>Checking integrity...</span>
                </div>
                <span className="text-white font-medium">{Math.round(integrityProgress * 100)}%</span>
              </div>
              <div className="w-full bg-white/10 rounded-full h-2">
                <div
                  className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${Math.round(integrityProgress * 100)}%` }}
                ></div>
              </div>
            </div>
          )}

          {integrityResult && (
            <div className="mt-4">
              {integrityResult.type === 'success' ? (
                <ActionResult
                  type={integrityResult.data.issues_found.length > 0 ? 'warning' : 'success'}
                  title={`Integrity Check Complete - ${integrityResult.data.issues_found.length} issue(s) found`}
                  details={
                    <div className="text-sm space-y-2 mt-2">
                      {integrityResult.data.scan_time && (
                        <div className="text-xs text-zinc-400 mb-2">
                          Report from: {new Date(integrityResult.data.scan_time).toLocaleString()}
                        </div>
                      )}
                      <div className="flex gap-4 text-zinc-300">
                        <span>Nodes checked: {integrityResult.data.nodes_checked}</span>
                        <span>Health score: {(integrityResult.data.health_score * 100).toFixed(1)}%</span>
                        <span>Duration: {formatDuration(integrityResult.data.duration_ms)}</span>
                      </div>
                      {integrityResult.data.issues_found.length > 0 && (
                        <div className="mt-4">
                          <div className="flex items-center justify-between mb-3">
                            <p className="text-yellow-300 font-medium">Issues found:</p>
                            <label className="flex items-center gap-2 text-sm text-zinc-300 cursor-pointer">
                              <input
                                type="checkbox"
                                checked={selectedIssues.size === integrityResult.data.issues_found.length}
                                onChange={toggleSelectAll}
                                className="w-4 h-4 rounded bg-white/10 border-white/20 text-primary-500 focus:ring-2 focus:ring-primary-500"
                              />
                              <span>Select All</span>
                            </label>
                          </div>
                          <div className="space-y-2 max-h-60 overflow-y-auto">
                            {integrityResult.data.issues_found.slice(0, 20).map((issue: any, idx: number) => {
                              // Format issue display based on type
                              let issueType = '';
                              let issueText = '';
                              let nodeId = '';

                              if (issue.MissingIndex) {
                                issueType = issue.MissingIndex.index_type;
                                nodeId = issue.MissingIndex.node_id;
                                if (issueType === 'Property') {
                                  issueText = `Missing ${issueType} index: ${nodeId}`;
                                } else {
                                  issueText = `Missing ${issueType} index: ${nodeId}`;
                                }
                              } else if (issue.OrphanedNode) {
                                issueType = 'Orphan';
                                nodeId = issue.OrphanedNode.id;
                                const parentId = issue.OrphanedNode.parent_id;
                                issueText = `Orphaned node: ${nodeId}${parentId ? ` (parent: ${parentId})` : ''}`;
                              } else if (issue.CorruptedData) {
                                issueType = 'Corrupt';
                                nodeId = issue.CorruptedData.node_id;
                                issueText = `Corrupted data: ${nodeId} - ${issue.CorruptedData.error}`;
                              } else if (issue.BrokenReference) {
                                issueType = 'Reference';
                                nodeId = issue.BrokenReference.from_id;
                                issueText = `Broken reference: ${nodeId} → ${issue.BrokenReference.to_id}`;
                              } else if (issue.DuplicateChild) {
                                issueType = 'Duplicate';
                                nodeId = issue.DuplicateChild.parent_id;
                                issueText = `Duplicate child: ${issue.DuplicateChild.child_id} in ${nodeId}`;
                              } else {
                                issueType = 'Unknown';
                                issueText = JSON.stringify(issue);
                              }

                              return (
                                <div key={idx} className="flex items-start gap-2 text-xs bg-white/5 p-2 rounded hover:bg-white/10 transition-colors">
                                  <input
                                    type="checkbox"
                                    checked={selectedIssues.has(idx)}
                                    onChange={() => toggleIssueSelection(idx)}
                                    className="mt-0.5 w-4 h-4 rounded bg-white/10 border-white/20 text-primary-500 focus:ring-2 focus:ring-primary-500 cursor-pointer"
                                  />
                                  <span className={`flex-shrink-0 px-2 py-0.5 rounded text-xs font-medium ${
                                    issueType === 'Property' ? 'bg-blue-500/20 text-blue-300' :
                                    issueType === 'Reference' ? 'bg-purple-500/20 text-purple-300' :
                                    issueType === 'Orphan' ? 'bg-yellow-500/20 text-yellow-300' :
                                    issueType === 'Corrupt' ? 'bg-red-500/20 text-red-300' :
                                    'bg-zinc-500/20 text-zinc-300'
                                  }`}>
                                    {issueType}
                                  </span>
                                  <span className="text-zinc-300 font-mono flex-1">{issueText}</span>
                                </div>
                              );
                            })}
                            {integrityResult.data.issues_found.length > 20 && (
                              <div className="text-xs text-zinc-400 text-center py-1">
                                ... and {integrityResult.data.issues_found.length - 20} more issues
                              </div>
                            )}
                          </div>

                          {/* Repair Action Buttons */}
                          <div className="mt-4 flex gap-3">
                            <ActionButton
                              onClick={handleRepairSelected}
                              loading={repairLoading}
                              icon={Wrench}
                              variant="secondary"
                              disabled={selectedIssues.size === 0}
                            >
                              Repair Selected ({selectedIssues.size})
                            </ActionButton>

                            <ActionButton
                              onClick={handleRepairAll}
                              loading={repairLoading}
                              icon={Wrench}
                              variant="danger"
                            >
                              Auto-Repair All
                            </ActionButton>

                            {repairLoading && repairJobId && (
                              <button
                                onClick={handleCancelRepair}
                                className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
                              >
                                <XCircle className="w-4 h-4" />
                                Cancel
                              </button>
                            )}
                          </div>

                          {repairLoading && repairProgress > 0 && (
                            <div className="mt-4">
                              <div className="flex items-center justify-between text-sm mb-2">
                                <div className="flex items-center gap-2 text-zinc-300">
                                  <RefreshCw className="w-4 h-4 animate-spin" />
                                  <span>Repairing issues...</span>
                                </div>
                                <span className="text-white font-medium">{Math.round(repairProgress * 100)}%</span>
                              </div>
                              <div className="w-full bg-white/10 rounded-full h-2">
                                <div
                                  className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                                  style={{ width: `${Math.round(repairProgress * 100)}%` }}
                                ></div>
                              </div>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  }
                  onDismiss={() => setIntegrityResult(null)}
                />
              ) : (
                <ActionResult
                  type={integrityResult.type}
                  title="Integrity Check Failed"
                  message={integrityResult.message}
                  onDismiss={() => setIntegrityResult(null)}
                />
              )}
            </div>
          )}

          {/* Repair Result Display */}
          {repairResult && (
            <div className="mt-4">
              {repairResult.type === 'success' ? (
                <ActionResult
                  type="success"
                  title="Repair Completed"
                  details={
                    <div className="text-sm text-green-300 space-y-2">
                      <div className="flex gap-4">
                        <span>Issues repaired: {repairResult.data.issues_repaired}</span>
                        <span>Issues failed: {repairResult.data.issues_failed}</span>
                        <span>Duration: {formatDuration(repairResult.data.duration_ms)}</span>
                      </div>
                      {Object.keys(repairResult.data.repairs_by_type).length > 0 && (
                        <div className="mt-2">
                          <p className="font-medium mb-1">Repairs by type:</p>
                          <ul className="list-disc list-inside space-y-0.5">
                            {Object.entries(repairResult.data.repairs_by_type).map(([type, count]) => (
                              <li key={type}>{type}: {count as number}</li>
                            ))}
                          </ul>
                        </div>
                      )}
                      {repairResult.data.errors.length > 0 && (
                        <div className="mt-2">
                          <p className="font-medium text-red-300 mb-1">Errors:</p>
                          <ul className="list-disc list-inside space-y-0.5 text-red-300 text-xs">
                            {repairResult.data.errors.map((error: string, idx: number) => (
                              <li key={idx}>{error}</li>
                            ))}
                          </ul>
                        </div>
                      )}
                    </div>
                  }
                  onDismiss={() => setRepairResult(null)}
                />
              ) : (
                <ActionResult
                  type={repairResult.type}
                  title="Repair Failed"
                  message={repairResult.message}
                  onDismiss={() => setRepairResult(null)}
                />
              )}
            </div>
          )}
        </GlassCard>
      </div>

      {/* Index Management Section */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4 flex items-center gap-2">
          <Database className="w-5 h-5 text-primary-400" />
          Index Management
        </h2>
        <GlassCard>
          <h3 className="text-lg font-semibold text-white mb-2">Verify Indexes</h3>
          <p className="text-zinc-400 text-sm mb-4">
            Check index consistency and find any issues with property, reference, or child order indexes
          </p>

          <div className="flex gap-2">
            <ActionButton
              onClick={handleVerifyIndexes}
              loading={verifyLoading}
              icon={FileText}
              variant="secondary"
              disabled={!selectedWorkspace}
            >
              Verify Indexes
            </ActionButton>

            {verifyLoading && verifyJobId && (
              <button
                onClick={handleCancelVerify}
                className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
              >
                <XCircle className="w-4 h-4" />
                Cancel
              </button>
            )}
          </div>

          {verifyLoading && verifyProgress > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between text-sm mb-2">
                <div className="flex items-center gap-2 text-zinc-300">
                  <RefreshCw className="w-4 h-4 animate-spin" />
                  <span>Verifying indexes...</span>
                </div>
                <span className="text-white font-medium">{Math.round(verifyProgress * 100)}%</span>
              </div>
              <div className="w-full bg-white/10 rounded-full h-2">
                <div
                  className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${Math.round(verifyProgress * 100)}%` }}
                ></div>
              </div>
            </div>
          )}

          {verifyResult && (
            <div className="mt-4">
              {verifyResult.type === 'success' ? (
                <ActionResult
                  type={verifyResult.data.length > 0 ? 'warning' : 'success'}
                  title={`Index Verification Complete - ${verifyResult.data.length} issue(s) found`}
                  details={
                    verifyResult.data.length > 0 && (
                      <div className="mt-2 max-h-40 overflow-y-auto">
                        <ul className="list-disc list-inside text-zinc-300 space-y-1 text-sm">
                          {verifyResult.data.map((issue: any, idx: number) => (
                            <li key={idx}>
                              {issue.index_type}: {issue.description} (Node: {issue.node_id})
                            </li>
                          ))}
                        </ul>
                      </div>
                    )
                  }
                  onDismiss={() => setVerifyResult(null)}
                />
              ) : (
                <ActionResult
                  type={verifyResult.type}
                  title="Index Verification Failed"
                  message={verifyResult.message}
                  onDismiss={() => setVerifyResult(null)}
                />
              )}
            </div>
          )}

          <div className="mt-6 pt-6 border-t border-white/10">
            <h3 className="text-lg font-semibold text-white mb-2">Rebuild Indexes</h3>
            <p className="text-zinc-400 text-sm mb-4">
              Rebuild corrupted or inconsistent indexes. This will regenerate the selected indexes from scratch.
            </p>

            <div className="mb-4">
              <label className="block text-sm font-medium text-zinc-300 mb-2">Index Type</label>
              <select
                value={indexType}
                onChange={(e) => setIndexType(e.target.value)}
                className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
              >
                <option value="all">All Indexes</option>
                <option value="property">Property Index</option>
                <option value="reference">Reference Index</option>
                <option value="child_order">Child Order Index</option>
              </select>
            </div>

            <div className="flex gap-2">
              <ActionButton
                onClick={handleRebuildIndexes}
                loading={rebuildLoading}
                icon={Database}
                variant="secondary"
                disabled={!selectedWorkspace}
              >
                Rebuild Indexes
              </ActionButton>

              {rebuildLoading && rebuildJobIds.has(indexType) && (
                <button
                  onClick={() => handleCancelRebuild(indexType)}
                  className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
                >
                  <XCircle className="w-4 h-4" />
                  Cancel
                </button>
              )}
            </div>

            {rebuildLoading && rebuildProgress.has(indexType) && rebuildProgress.get(indexType)! > 0 && (
              <div className="mt-4">
                <div className="flex items-center justify-between text-sm mb-2">
                  <div className="flex items-center gap-2 text-zinc-300">
                    <RefreshCw className="w-4 h-4 animate-spin" />
                    <span>Rebuilding {indexType} index...</span>
                  </div>
                  <span className="text-white font-medium">{Math.round(rebuildProgress.get(indexType)! * 100)}%</span>
                </div>
                <div className="w-full bg-white/10 rounded-full h-2">
                  <div
                    className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                    style={{ width: `${Math.round(rebuildProgress.get(indexType)! * 100)}%` }}
                  ></div>
                </div>
              </div>
            )}

            {rebuildResult && (
              <div className="mt-4">
                {rebuildResult.type === 'success' ? (
                  <ActionResult
                    type={rebuildResult.data.success ? 'success' : 'error'}
                    title={rebuildResult.data.success ? 'Indexes Rebuilt Successfully' : 'Index Rebuild Failed'}
                    details={
                      <div className="text-sm text-zinc-300 space-y-1">
                        <p>Index type: {rebuildResult.data.index_type}</p>
                        <p>Items processed: {rebuildResult.data.items_processed}</p>
                        <p>Errors: {rebuildResult.data.errors}</p>
                        <p>Duration: {formatDuration(rebuildResult.data.duration_ms)}</p>
                      </div>
                    }
                    onDismiss={() => setRebuildResult(null)}
                  />
                ) : (
                  <ActionResult
                    type={rebuildResult.type}
                    title="Index Rebuild Failed"
                    message={rebuildResult.message}
                    onDismiss={() => setRebuildResult(null)}
                  />
                )}
              </div>
            )}
          </div>
        </GlassCard>
      </div>

      {/* Relation Index Integrity Section */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4 flex items-center gap-2">
          <Link2 className="w-5 h-5 text-primary-400" />
          Relation Index Integrity
        </h2>
        <GlassCard>
          <h3 className="text-lg font-semibold text-white mb-2">Verify Relation Index</h3>
          <p className="text-zinc-400 text-sm mb-4">
            Scan the global relation index for orphaned relations (relations pointing to deleted or tombstoned nodes).
            This helps diagnose "Node not found" errors in GRAPH_TABLE queries.
          </p>

          <div className="flex gap-2">
            <ActionButton
              onClick={handleVerifyRelations}
              loading={relationVerifyLoading}
              icon={FileText}
              variant="secondary"
              disabled={!selectedWorkspace}
            >
              Verify Relations
            </ActionButton>

            {relationVerifyLoading && relationVerifyJobId && (
              <button
                onClick={handleCancelRelationVerify}
                className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
              >
                <XCircle className="w-4 h-4" />
                Cancel
              </button>
            )}
          </div>

          {relationVerifyLoading && relationVerifyProgress > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between text-sm mb-2">
                <div className="flex items-center gap-2 text-zinc-300">
                  <RefreshCw className="w-4 h-4 animate-spin" />
                  <span>Verifying relation index...</span>
                </div>
                <span className="text-white font-medium">{Math.round(relationVerifyProgress * 100)}%</span>
              </div>
              <div className="w-full bg-white/10 rounded-full h-2">
                <div
                  className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${Math.round(relationVerifyProgress * 100)}%` }}
                ></div>
              </div>
            </div>
          )}

          {relationVerifyResult && (
            <div className="mt-4">
              {relationVerifyResult.type === 'success' ? (
                <ActionResult
                  type="success"
                  title="Relation Verification Complete"
                  details={
                    relationVerifyResult.data ? (
                      <div className="text-sm text-zinc-300 space-y-1">
                        <p>Relations scanned: {relationVerifyResult.data.relations_scanned || 0}</p>
                        <p>Orphaned (source missing): {relationVerifyResult.data.orphaned_source || 0}</p>
                        <p>Orphaned (target missing): {relationVerifyResult.data.orphaned_target || 0}</p>
                        <p>Errors: {relationVerifyResult.data.errors || 0}</p>
                      </div>
                    ) : (
                      <p className="text-sm text-zinc-300">{relationVerifyResult.message}</p>
                    )
                  }
                  onDismiss={() => setRelationVerifyResult(null)}
                />
              ) : (
                <ActionResult
                  type={relationVerifyResult.type}
                  title="Relation Verification Failed"
                  message={relationVerifyResult.message}
                  onDismiss={() => setRelationVerifyResult(null)}
                />
              )}
            </div>
          )}

          <div className="mt-6 pt-6 border-t border-white/10">
            <h3 className="text-lg font-semibold text-white mb-2">Repair Orphaned Relations</h3>
            <p className="text-zinc-400 text-sm mb-4">
              Write tombstones for orphaned relations in the global index. This fixes "Node not found" errors
              in GRAPH_TABLE queries by marking stale relations as deleted.
            </p>

            <div className="flex gap-2">
              <ActionButton
                onClick={handleRepairRelations}
                loading={relationRepairLoading}
                icon={Wrench}
                variant="danger"
                disabled={!selectedWorkspace}
              >
                Repair Relations
              </ActionButton>

              {relationRepairLoading && relationRepairJobId && (
                <button
                  onClick={handleCancelRelationRepair}
                  className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
                >
                  <XCircle className="w-4 h-4" />
                  Cancel
                </button>
              )}
            </div>

            {relationRepairLoading && relationRepairProgress > 0 && (
              <div className="mt-4">
                <div className="flex items-center justify-between text-sm mb-2">
                  <div className="flex items-center gap-2 text-zinc-300">
                    <RefreshCw className="w-4 h-4 animate-spin" />
                    <span>Repairing orphaned relations...</span>
                  </div>
                  <span className="text-white font-medium">{Math.round(relationRepairProgress * 100)}%</span>
                </div>
                <div className="w-full bg-white/10 rounded-full h-2">
                  <div
                    className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                    style={{ width: `${Math.round(relationRepairProgress * 100)}%` }}
                  ></div>
                </div>
              </div>
            )}

            {relationRepairResult && (
              <div className="mt-4">
                {relationRepairResult.type === 'success' ? (
                  <ActionResult
                    type="success"
                    title="Relation Repair Complete"
                    details={
                      relationRepairResult.data ? (
                        <div className="text-sm text-zinc-300 space-y-1">
                          <p>Relations scanned: {relationRepairResult.data.relations_scanned || 0}</p>
                          <p>Tombstones written: {relationRepairResult.data.tombstones_written || 0}</p>
                          <p>Errors: {relationRepairResult.data.errors || 0}</p>
                        </div>
                      ) : (
                        <p className="text-sm text-zinc-300">{relationRepairResult.message}</p>
                      )
                    }
                    onDismiss={() => setRelationRepairResult(null)}
                  />
                ) : (
                  <ActionResult
                    type={relationRepairResult.type}
                    title="Relation Repair Failed"
                    message={relationRepairResult.message}
                    onDismiss={() => setRelationRepairResult(null)}
                  />
                )}
              </div>
            )}
          </div>
        </GlassCard>
      </div>

      {/* Cleanup Section */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4 flex items-center gap-2">
          <Trash2 className="w-5 h-5 text-primary-400" />
          Data Cleanup
        </h2>
        <GlassCard>
          <h3 className="text-lg font-semibold text-white mb-2">Cleanup Orphaned Nodes</h3>
          <p className="text-zinc-400 text-sm mb-4">
            Remove nodes that are no longer reachable from any workspace root. This is a permanent operation.
          </p>
          <div className="flex gap-2">
            <ActionButton
              onClick={handleCleanupOrphans}
              loading={cleanupLoading}
              icon={Trash2}
              variant="danger"
              disabled={!selectedWorkspace}
            >
              Cleanup Orphans
            </ActionButton>

            {cleanupLoading && cleanupJobId && (
              <button
                onClick={handleCancelCleanup}
                className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
              >
                <XCircle className="w-4 h-4" />
                Cancel
              </button>
            )}
          </div>

          {cleanupLoading && cleanupProgress > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between text-sm mb-2">
                <div className="flex items-center gap-2 text-zinc-300">
                  <RefreshCw className="w-4 h-4 animate-spin" />
                  <span>Cleaning up orphaned nodes...</span>
                </div>
                <span className="text-white font-medium">{Math.round(cleanupProgress * 100)}%</span>
              </div>
              <div className="w-full bg-white/10 rounded-full h-2">
                <div
                  className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${Math.round(cleanupProgress * 100)}%` }}
                ></div>
              </div>
            </div>
          )}

          {cleanupResult && (
            <div className="mt-4">
              {cleanupResult.type === 'success' ? (
                <ActionResult
                  type="success"
                  title="Cleanup Complete"
                  message={`Removed ${cleanupResult.data} orphaned node(s)`}
                  onDismiss={() => setCleanupResult(null)}
                />
              ) : (
                <ActionResult
                  type={cleanupResult.type}
                  title="Cleanup Failed"
                  message={cleanupResult.message}
                  onDismiss={() => setCleanupResult(null)}
                />
              )}
            </div>
          )}
        </GlassCard>

        {/* Property Index Cleanup Card */}
        <GlassCard className="mt-4">
          <h3 className="text-lg font-semibold text-white mb-2">Cleanup Orphaned Property Indexes</h3>
          <p className="text-zinc-400 text-sm mb-4">
            Remove property index entries that point to non-existent nodes. This fixes issues where
            SQL queries with LIMIT return fewer rows than expected due to orphaned index entries.
          </p>
          <div className="flex gap-2">
            <ActionButton
              onClick={handlePropertyIndexCleanup}
              loading={propertyIndexCleanupLoading}
              icon={Database}
              variant="secondary"
              disabled={!selectedWorkspace}
            >
              Cleanup Property Indexes
            </ActionButton>
          </div>

          {propertyIndexCleanupResult && (
            <div className="mt-4">
              {propertyIndexCleanupResult.type === 'success' ? (
                <ActionResult
                  type="success"
                  title="Property Index Cleanup Complete"
                  message={propertyIndexCleanupResult.message}
                  details={propertyIndexCleanupResult.data ? (
                    <div className="mt-2 text-sm text-zinc-400">
                      <div className="grid grid-cols-2 gap-2">
                        <div>Entries Scanned:</div>
                        <div className="text-white">{propertyIndexCleanupResult.data.entries_scanned?.toLocaleString()}</div>
                        <div>Orphaned Found:</div>
                        <div className="text-yellow-400">{propertyIndexCleanupResult.data.orphaned_found?.toLocaleString()}</div>
                        <div>Orphaned Deleted:</div>
                        <div className="text-green-400">{propertyIndexCleanupResult.data.orphaned_deleted?.toLocaleString()}</div>
                        <div>Duration:</div>
                        <div className="text-white">{propertyIndexCleanupResult.data.duration_ms}ms</div>
                        <div>Workspaces Processed:</div>
                        <div className="text-white">{propertyIndexCleanupResult.data.workspaces_processed}</div>
                      </div>
                    </div>
                  ) : undefined}
                  onDismiss={() => setPropertyIndexCleanupResult(null)}
                />
              ) : (
                <ActionResult
                  type={propertyIndexCleanupResult.type}
                  title="Property Index Cleanup Failed"
                  message={propertyIndexCleanupResult.message}
                  onDismiss={() => setPropertyIndexCleanupResult(null)}
                />
              )}
            </div>
          )}
        </GlassCard>
      </div>

      {/* Backup Section */}
      <div className="mb-8">
        <h2 className="text-xl font-semibold text-white mb-4 flex items-center gap-2">
          <Package className="w-5 h-5 text-primary-400" />
          Backup
        </h2>
        <GlassCard>
          <h3 className="text-lg font-semibold text-white mb-2">Create Backup</h3>
          <p className="text-zinc-400 text-sm mb-4">
            Create a complete backup of all workspaces and data. The backup will be saved to the specified directory.
          </p>

          <div className="mb-4">
            <label className="block text-sm font-medium text-zinc-300 mb-2">Backup Path</label>
            <input
              type="text"
              value={backupPath}
              onChange={(e) => setBackupPath(e.target.value)}
              placeholder="./backup"
              className="w-full px-4 py-2 bg-white/10 border border-white/20 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-primary-500"
            />
            <p className="text-xs text-zinc-500 mt-1">
              The backup directory where all data will be exported
            </p>
          </div>

          <div className="flex gap-2">
            <ActionButton
              onClick={handleBackupAll}
              loading={backupLoading}
              icon={Package}
              variant="secondary"
            >
              Backup All Data
            </ActionButton>

            {backupLoading && backupJobId && (
              <button
                onClick={handleCancelBackup}
                className="px-4 py-2 bg-red-500/20 hover:bg-red-500/30 border border-red-500/30 rounded-lg text-red-300 flex items-center gap-2 transition-colors"
              >
                <XCircle className="w-4 h-4" />
                Cancel
              </button>
            )}
          </div>

          {backupLoading && backupProgress > 0 && (
            <div className="mt-4">
              <div className="flex items-center justify-between text-sm mb-2">
                <div className="flex items-center gap-2 text-zinc-300">
                  <RefreshCw className="w-4 h-4 animate-spin" />
                  <span>Creating backup...</span>
                </div>
                <span className="text-white font-medium">{Math.round(backupProgress * 100)}%</span>
              </div>
              <div className="w-full bg-white/10 rounded-full h-2">
                <div
                  className="bg-gradient-to-r from-primary-500 to-accent-500 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${Math.round(backupProgress * 100)}%` }}
                ></div>
              </div>
            </div>
          )}

          {backupResult && (
            <div className="mt-4">
              {backupResult.type === 'success' && Array.isArray(backupResult.data) ? (
                <ActionResult
                  type="success"
                  title={`Backup Created - ${backupResult.data.length} workspace(s)`}
                  details={
                    <div className="mt-2 space-y-2">
                      {backupResult.data.map((backup: any, idx: number) => (
                        <div key={idx} className="text-sm text-zinc-300 bg-white/5 p-2 rounded">
                          <p className="font-medium">{backup.tenant}</p>
                          <p className="text-xs">Size: {formatBytes(backup.size_bytes)}</p>
                          <p className="text-xs">Nodes: {backup.node_count}</p>
                          <p className="text-xs">Duration: {formatDuration(backup.duration_ms)}</p>
                        </div>
                      ))}
                    </div>
                  }
                  onDismiss={() => setBackupResult(null)}
                />
              ) : (
                <ActionResult
                  type={backupResult.type}
                  title="Backup Failed"
                  message={backupResult.message}
                  onDismiss={() => setBackupResult(null)}
                />
              )}
            </div>
          )}
        </GlassCard>
      </div>
      <ConfirmDialog
        open={repairAllConfirm !== null}
        title="Confirm Auto-Repair"
        message={repairAllConfirm?.message || ''}
        variant="danger"
        confirmText="Auto-Repair All"
        onConfirm={() => {
          repairAllConfirm?.onConfirm()
          setRepairAllConfirm(null)
        }}
        onCancel={() => setRepairAllConfirm(null)}
      />
      <ConfirmDialog
        open={relationRepairConfirm !== null}
        title="Confirm Relation Repair"
        message={relationRepairConfirm?.message || ''}
        variant="danger"
        confirmText="Repair Relations"
        onConfirm={() => {
          relationRepairConfirm?.onConfirm()
          setRelationRepairConfirm(null)
        }}
        onCancel={() => setRelationRepairConfirm(null)}
      />
    </div>
  )
}
