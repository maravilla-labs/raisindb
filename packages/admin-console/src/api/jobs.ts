import { api, getAuthHeaders } from './client'
import { fetchEventSource } from '@microsoft/fetch-event-source'

// Types

export type JobStatus =
  | 'Scheduled'
  | 'Running'
  | 'Completed'
  | 'Cancelled'
  | string // For "Failed: <message>"

export interface JobInfo {
  id: string
  job_type: string
  status: JobStatus
  progress?: number
  error?: string
  result?: unknown
  created_at: string
  updated_at?: string
  completed_at?: string
}

export interface JobEventData {
  job_id: string
  job_type: string
  status: string
  old_status?: string
  tenant?: string
  progress?: number
  error?: string
  timestamp: string
  retry_count: number
  max_retries: number
  last_heartbeat?: string
  timeout_seconds: number
  next_retry_at?: string
  logs?: Array<{
    level: string
    message: string
    timestamp: string
  }>
  function_result?: unknown
  /** Flow instance ID (for FlowInstanceExecution jobs) */
  flow_instance_id?: string
}

// API Functions

/**
 * Get job status by ID
 */
export async function getJobStatus(jobId: string): Promise<JobStatus> {
  const response = await api.get<{ data: JobStatus }>(`/management/jobs/${jobId}`)
  return response.data
}

/**
 * Get job info by ID
 */
export async function getJobInfo(jobId: string): Promise<JobInfo> {
  const response = await api.get<{ data: JobInfo }>(`/management/jobs/${jobId}/info`)
  return response.data
}

/**
 * Subscribe to job events via SSE
 * Returns a cleanup function to close the connection
 */
export function subscribeToJobEvents(
  onEvent: (event: JobEventData) => void,
  onError?: (error: Error) => void
): () => void {
  const controller = new AbortController()

  fetchEventSource('/management/events/jobs', {
    headers: getAuthHeaders(),
    signal: controller.signal,
    onmessage(event) {
      if (event.event === 'job-update') {
        try {
          const data = JSON.parse(event.data) as JobEventData
          onEvent(data)
        } catch (e) {
          console.error('Failed to parse job event:', e)
        }
      }
    },
    onerror(error) {
      console.error('SSE connection error:', error)
      onError?.(error)
    },
    openWhenHidden: true, // Keep connection alive when tab is hidden
  })

  // Return cleanup function
  return () => {
    controller.abort()
  }
}

/**
 * Poll for job completion
 * @param jobId - The job ID to poll
 * @param onProgress - Callback for progress updates
 * @param intervalMs - Poll interval in milliseconds (default 1000)
 * @param maxAttempts - Maximum poll attempts (default 300 = 5 minutes at 1s intervals)
 */
export async function pollJobUntilComplete(
  jobId: string,
  onProgress?: (status: JobStatus, progress?: number) => void,
  intervalMs = 1000,
  maxAttempts = 300
): Promise<JobInfo> {
  let attempts = 0

  while (attempts < maxAttempts) {
    const info = await getJobInfo(jobId)

    if (onProgress) {
      onProgress(info.status, info.progress)
    }

    // Check if job is complete (success or failure)
    if (info.status === 'Completed' || info.status === 'Cancelled' || info.status.startsWith('Failed')) {
      return info
    }

    // Wait before next poll
    await new Promise(resolve => setTimeout(resolve, intervalMs))
    attempts++
  }

  throw new Error(`Job ${jobId} did not complete within ${maxAttempts * intervalMs / 1000} seconds`)
}

// Convenience exports
export const jobsApi = {
  getJobStatus,
  getJobInfo,
  subscribeToJobEvents,
  pollJobUntilComplete,
}
