import { api, getAuthHeaders } from './client'
import { fetchEventSource } from '@microsoft/fetch-event-source'

// Types for Management API responses
export interface HealthStatus {
  status: 'Healthy' | 'Degraded' | 'Critical'
  tenant: string | null
  checks: HealthCheck[]
  needs_healing: boolean
  last_check: string
}

export interface HealthCheck {
  name: string
  status: 'Healthy' | 'Degraded' | 'Critical'
  message: string | null
}

export interface IntegrityReport {
  tenant: string
  scan_time: string
  nodes_checked: number
  issues_found: Issue[]
  health_score: number
  duration_ms: number
}

export type Issue =
  | { type: 'OrphanedNode'; id: string; parent_id: string | null }
  | { type: 'MissingIndex'; node_id: string; index_type: IndexType }
  | { type: 'InconsistentIndex'; node_id: string; expected: string; actual: string }
  | { type: 'CorruptedData'; node_id: string; error: string }
  | { type: 'BrokenReference'; from_id: string; to_id: string; ref_type: string }
  | { type: 'DuplicateChild'; parent_id: string; child_id: string }
  | { type: 'MissingWorkspace'; node_id: string; workspace_id: string }

export interface IndexIssue {
  index_type: IndexType
  node_id: string
  description: string
}

export type IndexType = 'Property' | 'Reference' | 'ChildOrder' | 'All'

export interface RebuildStats {
  index_type: IndexType
  items_processed: number
  errors: number
  duration_ms: number
  success: boolean
}

export interface Metrics {
  tenant: string | null
  operations_per_sec: number
  error_rate: number
  disk_usage_bytes: number
  index_sizes: Record<string, number>
  node_count: number
  active_connections: number
  cache_hit_rate: number
  last_compaction: string | null
}

export interface CompactionStats {
  tenant: string | null
  bytes_before: number
  bytes_after: number
  duration_ms: number
  files_compacted: number
}

export interface BackupInfo {
  tenant: string
  path: string
  size_bytes: number
  created_at: string
  duration_ms: number
  node_count: number
  version: string
}

export interface RepairResult {
  tenant: string
  issues_repaired: number
  issues_failed: number
  repairs_by_type: Record<string, number>
  duration_ms: number
  errors: string[]
}

export interface JobInfo {
  id: string
  job_type: JobType
  status: JobStatus
  tenant: string | null
  started_at: string
  completed_at: string | null
  progress: number | null
  error: string | null
  result: any | null  // Job result data (e.g., integrity report for IntegrityScan jobs)
  retry_count: number  // Current retry attempt (0-based, 0 = first attempt)
  max_retries: number  // Maximum number of retry attempts (default 3)
  last_heartbeat: string | null  // Last heartbeat timestamp (for timeout detection)
  timeout_seconds: number  // Timeout in seconds (default 300 = 5 minutes)
  next_retry_at: string | null  // When the job should be retried (null = process immediately)
}

export type JobType =
  | 'IntegrityScan'
  | 'IndexRebuild'
  | 'IndexVerify'
  | 'Compaction'
  | 'Backup'
  | 'Restore'
  | 'OrphanCleanup'
  | 'Repair'
  | { Custom: string }

export type JobStatus =
  | 'Scheduled'
  | 'Running'
  | 'Executing'
  | 'Completed'
  | 'Cancelled'
  | { Failed: string }

/** Log entry from function execution */
export interface SseLogEntry {
  level: 'debug' | 'info' | 'warn' | 'error'
  message: string
  timestamp: string
}

/** Function execution result from backend */
export interface FunctionExecutionResult {
  execution_id: string
  success: boolean
  result?: unknown
  error?: string
  duration_ms: number
  logs: string[]
}

export interface JobEvent {
  job_id: string
  job_type: string
  status: string
  old_status: string | null
  tenant: string | null
  progress: number | null
  error: string | null
  timestamp: string
  retry_count?: number
  max_retries?: number
  last_heartbeat?: string | null
  timeout_seconds?: number
  next_retry_at?: string | null
  /** Logs from function execution (only for FunctionExecution jobs) */
  logs?: SseLogEntry[]
  /** Full function execution result (only for FunctionExecution jobs) */
  function_result?: FunctionExecutionResult
  /** Function path (for FunctionExecution jobs) */
  function_path?: string
  /** Trigger path (for FlowExecution and FunctionExecution jobs) */
  trigger_path?: string
  /** Workspace ID */
  workspace?: string
}

/** Real-time job log event from SSE */
export interface JobLogEvent {
  job_id: string
  level: string
  message: string
  timestamp: string
}

// API Response wrapper
interface ApiResponse<T> {
  success: boolean
  data?: T
  error?: string
}

// Job queue stats types
export interface JobQueueStats {
  queue: QueueDepthStats
  workers: WorkerStats
  persisted: PersistedStats
  categories?: CategoryQueueDepthStats[]
}

export interface QueueDepthStats {
  high_queue_len: number
  high_queue_capacity: number
  normal_queue_len: number
  normal_queue_capacity: number
  low_queue_len: number
  low_queue_capacity: number
  total_high_dispatched: number
  total_normal_dispatched: number
  total_low_dispatched: number
}

export interface CategoryQueueDepthStats {
  category: string
  high_queue_len: number
  normal_queue_len: number
  low_queue_len: number
  total_dispatched: number
}

export interface WorkerStats {
  pool_size: number
}

export interface PersistedStats {
  total_entries: number
  orphaned_entries: number
}

// Management API client
export const managementApi = {
  // Health endpoints
  getHealth: () =>
    api.get<ApiResponse<HealthStatus>>('/management/health'),

  getTenantHealth: (tenant: string) =>
    api.get<ApiResponse<HealthStatus>>(`/management/health/${tenant}`),

  // Integrity endpoints
  checkIntegrity: (tenant: string) =>
    api.get<ApiResponse<IntegrityReport>>(`/management/integrity/${tenant}`),

  startIntegrityCheck: (tenant: string) =>
    api.post<ApiResponse<string>>(`/management/integrity/${tenant}/start`),

  getLastIntegrityReport: (tenant: string) =>
    api.get<ApiResponse<IntegrityReport>>(`/management/integrity/${tenant}/last`),

  repairIssues: (tenant: string, issues: Issue[]) =>
    api.post<ApiResponse<RepairResult>>(
      `/management/integrity/${tenant}/repair`,
      { issues }
    ),

  startRepair: (tenant: string, issues: Issue[]) =>
    api.post<ApiResponse<string>>(
      `/management/integrity/${tenant}/repair/start`,
      { issues }
    ),

  verifyIndexes: (tenant: string) =>
    api.get<ApiResponse<IndexIssue[]>>(`/management/integrity/${tenant}/verify`),

  startVerifyIndexes: (tenant: string) =>
    api.post<ApiResponse<string>>(`/management/integrity/${tenant}/verify/start`),

  rebuildIndexes: (tenant: string, indexType: string) =>
    api.post<ApiResponse<RebuildStats>>(
      `/management/integrity/${tenant}/rebuild`,
      { index_type: indexType }
    ),

  startRebuildIndexes: (tenant: string, indexType: string) =>
    api.post<ApiResponse<string>>(
      `/management/integrity/${tenant}/rebuild/start`,
      { index_type: indexType }
    ),

  cleanupOrphans: (tenant: string) =>
    api.post<ApiResponse<number>>(`/management/integrity/${tenant}/cleanup`),

  startCleanupOrphans: (tenant: string) =>
    api.post<ApiResponse<string>>(`/management/integrity/${tenant}/cleanup/start`),

  // Property Index Orphan Cleanup - removes index entries pointing to non-existent nodes
  // This fixes issues where LIMIT queries return 0 rows due to orphaned index entries
  cleanupPropertyIndexOrphans: (tenant: string) =>
    api.post<ApiResponse<{
      entries_scanned: number
      orphaned_found: number
      orphaned_deleted: number
      errors: number
      duration_ms: number
      workspaces_processed: number
    }>>(`/management/integrity/${tenant}/cleanup-property-indexes`),

  // Metrics endpoints
  getMetrics: () =>
    api.get<ApiResponse<Metrics>>('/management/metrics'),

  getTenantMetrics: (tenant: string) =>
    api.get<ApiResponse<Metrics>>(`/management/metrics/${tenant}`),

  // Maintenance endpoints
  triggerCompaction: () =>
    api.post<ApiResponse<CompactionStats>>('/management/compact'),

  startCompaction: () =>
    api.post<ApiResponse<string>>('/management/compact/start'),

  triggerTenantCompaction: (tenant: string) =>
    api.post<ApiResponse<CompactionStats>>(`/management/compact/${tenant}`),

  // Backup endpoints
  backupTenant: (tenant: string, path: string) =>
    api.post<ApiResponse<BackupInfo>>(
      `/management/backup/${tenant}`,
      { path }
    ),

  backupAll: (path: string) =>
    api.post<ApiResponse<BackupInfo[]>>(
      '/management/backup/all',
      { path }
    ),

  startBackup: (path: string) =>
    api.post<ApiResponse<string>>(
      '/management/backup/all/start',
      { path }
    ),

  // Reindex endpoint
  startReindex: (
    tenant: string,
    repo: string,
    workspace: string,
    indexTypes: string[],
    branch?: string
  ) =>
    api.post<ApiResponse<{ job_id: string; message: string }>>(
      `/api/admin/management/database/${tenant}/${repo}/reindex/start${branch ? `?branch=${branch}` : ''}`,
      {
        workspace,
        index_types: indexTypes,
      }
    ),

  // Job management endpoints
  listJobs: () =>
    api.get<ApiResponse<JobInfo[]>>('/management/jobs'),

  getJobStatus: (id: string) =>
    api.get<ApiResponse<JobStatus>>(`/management/jobs/${id}`),

  getJobInfo: (id: string) =>
    api.get<ApiResponse<JobInfo>>(`/management/jobs/${id}/info`),

  deleteJob: (id: string) =>
    api.delete<ApiResponse<void>>(`/management/jobs/${id}`),

  cancelJob: (id: string) =>
    api.post<ApiResponse<void>>(`/management/jobs/${id}/cancel`),

  batchDeleteJobs: (jobIds: string[]) =>
    api.post<ApiResponse<{ deleted: number; skipped: number }>>(
      '/management/jobs/batch-delete',
      { job_ids: jobIds }
    ),

  scheduleIntegrityScan: (tenant: string, intervalMinutes: number) =>
    api.post<ApiResponse<string>>(
      '/management/jobs/schedule/integrity',
      { tenant, interval_minutes: intervalMinutes }
    ),

  // Job queue management endpoints
  getJobQueueStats: () =>
    api.get<ApiResponse<JobQueueStats>>('/management/jobs/stats'),

  purgeAllJobs: () =>
    api.post<ApiResponse<{ purged: number }>>('/management/jobs/purge-all'),

  purgeOrphanedJobs: () =>
    api.post<ApiResponse<{ purged: number }>>('/management/jobs/purge-orphaned'),

  forceFailStuckJobs: (stuckMinutes = 10) =>
    api.post<ApiResponse<{ failed_count: number; job_ids: string[] }>>(
      '/management/jobs/force-fail-stuck',
      { stuck_minutes: stuckMinutes }
    ),
}

// SSE Event Source Manager using fetchEventSource for auth support
export class EventSourceManager {
  private controllers: Map<string, AbortController> = new Map()
  private reconnectTimeouts: Map<string, ReturnType<typeof setTimeout>> = new Map()

  connect(
    endpoint: string,
    handlers: {
      onMessage?: (event: { data: string }) => void
      onJobUpdate?: (event: JobEvent) => void
      onJobLog?: (event: JobLogEvent) => void
      onHealthUpdate?: (health: HealthStatus) => void
      onMetricsUpdate?: (metrics: Metrics) => void
      onError?: (error: Error) => void
      onOpen?: () => void
    }
  ): () => void {
    // Close existing connection if any
    this.disconnect(endpoint)

    const url = `/management/events/${endpoint}`
    const controller = new AbortController()
    this.controllers.set(endpoint, controller)

    // Get auth headers (without impersonation for SSE)
    const authHeaders = getAuthHeaders()
    // Remove impersonation header for SSE - not needed
    delete authHeaders['X-Raisin-Impersonate']

    fetchEventSource(url, {
      headers: authHeaders,
      signal: controller.signal,
      openWhenHidden: true,

      onopen: async () => {
        console.log(`SSE connected to ${endpoint}`)
        handlers.onOpen?.()

        // Clear any reconnection timeout
        const timeout = this.reconnectTimeouts.get(endpoint)
        if (timeout) {
          clearTimeout(timeout)
          this.reconnectTimeouts.delete(endpoint)
        }
      },

      onmessage: (event) => {
        // Handle specific event types
        if (event.event === 'job-update' && handlers.onJobUpdate) {
          try {
            const data = JSON.parse(event.data) as JobEvent
            handlers.onJobUpdate(data)
          } catch (e) {
            console.error('Failed to parse job event:', e)
          }
        } else if (event.event === 'job-log' && handlers.onJobLog) {
          try {
            const data = JSON.parse(event.data) as JobLogEvent
            handlers.onJobLog(data)
          } catch (e) {
            console.error('Failed to parse job log event:', e)
          }
        } else if (event.event === 'health-update' && handlers.onHealthUpdate) {
          try {
            const data = JSON.parse(event.data) as HealthStatus
            handlers.onHealthUpdate(data)
          } catch (e) {
            console.error('Failed to parse health event:', e)
          }
        } else if (event.event === 'metrics-update' && handlers.onMetricsUpdate) {
          try {
            const data = JSON.parse(event.data) as Metrics
            handlers.onMetricsUpdate(data)
          } catch (e) {
            console.error('Failed to parse metrics event:', e)
          }
        } else if (event.event === 'keep-alive') {
          // Ignore keep-alive messages
        } else if (handlers.onMessage && !event.event) {
          // Generic message handler for messages without event type
          handlers.onMessage({ data: event.data })
        }
      },

      onerror: (error) => {
        console.error(`SSE error on ${endpoint}:`, error)
        handlers.onError?.(error)

        // Auto-reconnect after 5 seconds if not aborted
        if (!controller.signal.aborted) {
          console.log(`Reconnecting to ${endpoint} in 5 seconds...`)
          const timeout = setTimeout(() => {
            this.connect(endpoint, handlers)
          }, 5000)
          this.reconnectTimeouts.set(endpoint, timeout)
        }
      },
    })

    // Return cleanup function
    return () => this.disconnect(endpoint)
  }

  disconnect(endpoint: string) {
    const controller = this.controllers.get(endpoint)
    if (controller) {
      controller.abort()
      this.controllers.delete(endpoint)
      console.log(`SSE disconnected from ${endpoint}`)
    }

    const timeout = this.reconnectTimeouts.get(endpoint)
    if (timeout) {
      clearTimeout(timeout)
      this.reconnectTimeouts.delete(endpoint)
    }
  }

  disconnectAll() {
    for (const endpoint of this.controllers.keys()) {
      this.disconnect(endpoint)
    }
  }
}

// Singleton instance
export const sseManager = new EventSourceManager()

// Utility functions
export function formatJobStatus(status: JobStatus): string {
  if (typeof status === 'string') {
    return status
  }
  return 'Failed'
}

export function formatJobType(type: JobType): string {
  if (typeof type === 'string') {
    return type
  }
  return type.Custom
}

export function getJobStatusColor(status: JobStatus): string {
  if (typeof status === 'string') {
    switch (status) {
      case 'Scheduled': return 'blue'
      case 'Running': return 'yellow'
      case 'Executing': return 'yellow'
      case 'Completed': return 'green'
      case 'Cancelled': return 'gray'
      default: return 'gray'
    }
  }
  return 'red' // Failed
}

export function getHealthStatusColor(status: HealthStatus['status']): string {
  switch (status) {
    case 'Healthy': return 'green'
    case 'Degraded': return 'yellow'
    case 'Critical': return 'red'
    default: return 'gray'
  }
}

export function formatBytes(bytes: number): string {
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  if (bytes === 0) return '0 B'
  const i = Math.floor(Math.log(bytes) / Math.log(1024))
  return `${(bytes / Math.pow(1024, i)).toFixed(2)} ${sizes[i]}`
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}m`
}

// Database-level Index Management Types
export interface FulltextHealth {
  memory_usage_bytes: number
  disk_usage_bytes: number
  entry_count: number
  cache_hit_rate: number
  last_optimized: string | null
}

export interface VectorHealth {
  memory_usage_bytes: number
  disk_usage_bytes: number
  entry_count: number
  dimensions: number
  index_type: string
  last_optimized: string | null
}

export interface JobResponse {
  job_id: string
  message: string
}

// Database-level Index Management API
export const databaseManagementApi = {
  // Fulltext Index Operations
  fulltextVerify: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/fulltext/verify`),

  fulltextRebuild: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/fulltext/rebuild`),

  fulltextOptimize: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/fulltext/optimize`),

  fulltextPurge: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/fulltext/purge`),

  fulltextHealth: (tenant: string, repo: string) =>
    api.get<FulltextHealth>(`/api/admin/management/database/${tenant}/${repo}/fulltext/health`),

  // Vector Index Operations
  vectorVerify: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/vector/verify`),

  vectorRebuild: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/vector/rebuild`),

  vectorRegenerate: (tenant: string, repo: string, force: boolean = false) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/vector/regenerate?force=${force}`),

  vectorOptimize: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/vector/optimize`),

  vectorRestore: (tenant: string, repo: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/vector/restore`),

  vectorHealth: (tenant: string, repo: string) =>
    api.get<VectorHealth>(`/api/admin/management/database/${tenant}/${repo}/vector/health`),

  // Relation Index Integrity Operations
  relationsVerify: (tenant: string, repo: string, branch?: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/relations/verify${branch ? `?branch=${branch}` : ''}`),

  relationsRepair: (tenant: string, repo: string, branch?: string) =>
    api.post<JobResponse>(`/api/admin/management/database/${tenant}/${repo}/relations/repair${branch ? `?branch=${branch}` : ''}`),
}