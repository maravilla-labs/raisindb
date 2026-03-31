import { api, getAuthHeaders } from './client'

// Types

export interface FunctionSummary {
  path: string
  name: string
  title: string
  description?: string
  language: string
  enabled: boolean
  execution_mode: string
  has_http_trigger: boolean
  has_event_triggers: boolean
  has_schedule_triggers: boolean
}

export interface FunctionDetails {
  path: string
  name: string
  title: string
  description?: string
  language: string
  enabled: boolean
  execution_mode: string
  /** Entry file in format 'filename:function' (e.g., 'index.js:handler') */
  entry_file: string
  /** @deprecated Use entry_file instead */
  entrypoint?: string
  resource_limits?: ResourceLimits
  network_policy?: NetworkPolicy
  triggers?: TriggerCondition[]
  input_schema?: Record<string, unknown>
  output_schema?: Record<string, unknown>
  code?: string
  created_at: string
  updated_at: string
}

/** File within a function */
export interface FunctionFile {
  path: string
  name: string
  node_type: string
  size?: number
  mime_type?: string
}

export interface ResourceLimits {
  max_execution_time_ms: number
  max_memory_bytes: number
  max_instructions?: number
}

export interface NetworkPolicy {
  allowed_hosts: string[]
  max_requests_per_execution: number
}

export interface TriggerCondition {
  name: string
  enabled: boolean
  trigger_type: TriggerType
  filters?: TriggerFilters
  priority?: number
}

export type TriggerType =
  | { type: 'node_event'; event_kinds: string[] }
  | { type: 'schedule'; cron: string; timezone?: string }
  | { type: 'http'; methods?: string[]; path_prefix?: string }

export interface TriggerFilters {
  node_types?: string[]
  paths?: string[]
  workspaces?: string[]
  properties?: Record<string, unknown>
}

export interface ExecutionRequest {
  input?: Record<string, unknown>
  sync?: boolean
  timeout_ms?: number
}

export interface ExecutionResponse {
  execution_id: string
  sync: boolean
  result?: unknown
  error?: string
  job_id?: string
  duration_ms?: number
  logs?: string[]
}

export interface ExecutionRecord {
  execution_id: string
  function_path: string
  trigger_name?: string
  status: 'scheduled' | 'running' | 'completed' | 'failed' | 'cancelled'
  started_at: string
  completed_at?: string
  duration_ms?: number
  result?: unknown
  error?: string
}

export interface ListFunctionsParams {
  language?: string
  enabled?: boolean
  include_disabled?: boolean
  limit?: number
  offset?: number
}

export interface ListExecutionsParams {
  status?: string
  trigger_name?: string
  limit?: number
  offset?: number
}

// ============================================================================
// Direct File Execution Types (SSE streaming)
// ============================================================================

/** Request to run a JavaScript file directly */
export interface RunFileRequest {
  /** Node ID of the raisin:Asset containing JS code (optional if code is provided) */
  node_id?: string
  /** Inline code to execute (used when file is unsaved in editor) */
  code?: string
  /** File name for inline code (e.g., "index.js") - used for validation */
  file_name?: string
  /** Path to the parent raisin:Function node (for network_policy lookup with unsaved code) */
  function_path?: string
  /** Name of the exported function to call (e.g., "handler") */
  handler: string
  /** JSON input data (mutually exclusive with input_node_id) */
  input?: Record<string, unknown>
  /** Node ID to use as input (loads node and passes as JSON) */
  input_node_id?: string
  /** Workspace to look up input_node_id from (defaults to "content") */
  input_workspace?: string
  /** Optional timeout override in milliseconds */
  timeout_ms?: number
}

/** Log entry from execution */
export interface LogEntry {
  level: string
  message: string
  timestamp: string
}

/** SSE event: execution started */
export interface RunFileStartedEvent {
  type: 'started'
  execution_id: string
  file_name: string
  handler: string
}

/** SSE event: log output */
export interface RunFileLogEvent {
  type: 'log'
  level: string
  message: string
  timestamp: string
}

/** SSE event: execution result */
export interface RunFileResultEvent {
  type: 'result'
  execution_id: string
  success: boolean
  result?: unknown
  error?: string
  duration_ms: number
}

/** SSE event: stream complete */
export interface RunFileDoneEvent {
  type: 'done'
}

/** All possible SSE events from file execution */
export type RunFileEvent =
  | RunFileStartedEvent
  | RunFileLogEvent
  | RunFileResultEvent
  | RunFileDoneEvent

/** Callbacks for handling SSE events from file execution */
export interface RunFileCallbacks {
  onStarted?: (event: RunFileStartedEvent) => void
  onLog?: (event: RunFileLogEvent) => void
  onResult?: (event: RunFileResultEvent) => void
  onDone?: () => void
  onError?: (error: Error) => void
}

// API Functions

/**
 * List all functions in a repository
 */
export async function listFunctions(
  repo: string,
  params?: ListFunctionsParams
): Promise<FunctionSummary[]> {
  const searchParams = new URLSearchParams()
  if (params?.language) searchParams.set('language', params.language)
  if (params?.enabled !== undefined) searchParams.set('enabled', String(params.enabled))
  if (params?.include_disabled) searchParams.set('include_disabled', 'true')
  if (params?.limit) searchParams.set('limit', String(params.limit))
  if (params?.offset) searchParams.set('offset', String(params.offset))

  const query = searchParams.toString()
  const path = `/api/functions/${repo}${query ? `?${query}` : ''}`
  return api.get<FunctionSummary[]>(path)
}

/**
 * Get function details
 */
export async function getFunction(
  repo: string,
  name: string,
  includeCode = false
): Promise<FunctionDetails> {
  const query = includeCode ? '?include_code=true' : ''
  return api.get<FunctionDetails>(`/api/functions/${repo}/${name}${query}`)
}

/**
 * Invoke a function
 */
export async function invokeFunction(
  repo: string,
  name: string,
  request: ExecutionRequest = {}
): Promise<ExecutionResponse> {
  return api.post<ExecutionResponse>(`/api/functions/${repo}/${name}/invoke`, request)
}

/**
 * List function executions
 */
export async function listExecutions(
  repo: string,
  name: string,
  params?: ListExecutionsParams
): Promise<ExecutionRecord[]> {
  const searchParams = new URLSearchParams()
  if (params?.status) searchParams.set('status', params.status)
  if (params?.trigger_name) searchParams.set('trigger_name', params.trigger_name)
  if (params?.limit) searchParams.set('limit', String(params.limit))
  if (params?.offset) searchParams.set('offset', String(params.offset))

  const query = searchParams.toString()
  const path = `/api/functions/${repo}/${name}/executions${query ? `?${query}` : ''}`
  return api.get<ExecutionRecord[]>(path)
}

/**
 * Get execution details
 */
export async function getExecution(
  repo: string,
  name: string,
  executionId: string
): Promise<ExecutionRecord> {
  return api.get<ExecutionRecord>(`/api/functions/${repo}/${name}/executions/${executionId}`)
}

/**
 * Poll for execution completion
 * Returns the final execution record when complete or times out
 */
export async function pollExecution(
  repo: string,
  name: string,
  executionId: string,
  options: { timeoutMs?: number; intervalMs?: number } = {}
): Promise<ExecutionRecord> {
  const { timeoutMs = 30000, intervalMs = 500 } = options
  const startTime = Date.now()

  while (Date.now() - startTime < timeoutMs) {
    const execution = await getExecution(repo, name, executionId)

    if (execution.status === 'completed' || execution.status === 'failed' || execution.status === 'cancelled') {
      return execution
    }

    await new Promise(resolve => setTimeout(resolve, intervalMs))
  }

  throw new Error(`Execution ${executionId} timed out after ${timeoutMs}ms`)
}

/**
 * Run a JavaScript file directly with SSE streaming
 *
 * Returns an AbortController that can be used to cancel the request
 *
 * @example
 * ```ts
 * const abort = runFileStream(repo, request, {
 *   onLog: (event) => console.log(event.message),
 *   onResult: (event) => console.log('Result:', event.result),
 *   onError: (error) => console.error('Error:', error),
 * })
 *
 * // To cancel:
 * abort.abort()
 * ```
 */
export function runFileStream(
  repo: string,
  request: RunFileRequest,
  callbacks: RunFileCallbacks
): AbortController {
  const abortController = new AbortController()

  // Start the fetch in the background
  ;(async () => {
    try {
      const response = await fetch(`/api/files/${repo}/run`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Accept': 'text/event-stream',
          ...getAuthHeaders(),
        },
        body: JSON.stringify(request),
        signal: abortController.signal,
      })

      if (!response.ok) {
        const errorText = await response.text()
        throw new Error(`HTTP ${response.status}: ${errorText}`)
      }

      if (!response.body) {
        throw new Error('No response body')
      }

      const reader = response.body.getReader()
      const decoder = new TextDecoder()
      let buffer = ''

      while (true) {
        const { done, value } = await reader.read()

        if (done) {
          break
        }

        buffer += decoder.decode(value, { stream: true })

        // Parse SSE events from buffer
        const lines = buffer.split('\n')
        buffer = lines.pop() || '' // Keep incomplete line in buffer

        let currentEventType = ''
        let currentData = ''

        for (const line of lines) {
          if (line.startsWith('event:')) {
            currentEventType = line.slice(6).trim()
          } else if (line.startsWith('data:')) {
            currentData = line.slice(5).trim()
          } else if (line === '' && currentData) {
            // End of event, process it
            try {
              const event = JSON.parse(currentData) as RunFileEvent

              switch (currentEventType || event.type) {
                case 'started':
                  callbacks.onStarted?.(event as RunFileStartedEvent)
                  break
                case 'log':
                  callbacks.onLog?.(event as RunFileLogEvent)
                  break
                case 'result':
                  callbacks.onResult?.(event as RunFileResultEvent)
                  break
                case 'done':
                  callbacks.onDone?.()
                  break
              }
            } catch {
              // Ignore parse errors for malformed events
            }

            currentEventType = ''
            currentData = ''
          }
        }
      }
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        // Request was cancelled, ignore
        return
      }
      callbacks.onError?.(error instanceof Error ? error : new Error(String(error)))
    }
  })()

  return abortController
}

// ============================================================================
// Webhook/Trigger Invocation Types
// ============================================================================

/** Request to invoke a webhook or trigger via HTTP */
export interface WebhookInvokeRequest {
  /** HTTP method to use */
  method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
  /** Query parameters */
  query_params?: Record<string, string>
  /** Request body (for POST/PUT/PATCH) */
  body?: Record<string, unknown>
  /** Custom headers */
  headers?: Record<string, string>
  /** Whether to wait for result (sync) or return immediately (async) */
  sync?: boolean
  /** Timeout in milliseconds */
  timeout_ms?: number
}

/** Response from webhook/trigger invocation */
export interface WebhookInvokeResponse {
  /** Unique execution ID */
  execution_id: string
  /** Execution status */
  status: 'queued' | 'completed' | 'failed'
  /** Result from function (if sync and successful) */
  result?: unknown
  /** Error message (if failed) */
  error?: string
  /** Execution duration (if sync) */
  duration_ms?: number
  /** Logs from execution (if sync) */
  logs?: string[]
  /** Job ID (if async) */
  job_id?: string
}

/**
 * Invoke a trigger by its unique name
 * Uses: POST /api/triggers/{repo}/{triggerName}
 */
export async function invokeTrigger(
  repo: string,
  triggerName: string,
  request: WebhookInvokeRequest
): Promise<WebhookInvokeResponse> {
  const queryParams = new URLSearchParams()
  if (request.sync) queryParams.set('sync', 'true')
  if (request.timeout_ms) queryParams.set('timeout_ms', String(request.timeout_ms))
  if (request.query_params) {
    Object.entries(request.query_params).forEach(([key, value]) => {
      queryParams.set(key, value)
    })
  }

  const query = queryParams.toString()
  const path = `/api/triggers/${repo}/${triggerName}${query ? `?${query}` : ''}`

  // Build headers - include auth headers and request-specific headers
  const headers: Record<string, string> = { ...getAuthHeaders(), ...request.headers }
  const hasBody = ['POST', 'PUT', 'PATCH'].includes(request.method) && request.body

  if (hasBody) {
    headers['Content-Type'] = 'application/json'
  }

  // Use the appropriate HTTP method
  const fetchOptions: RequestInit = {
    method: request.method,
    headers,
  }

  // Add body for methods that support it
  if (hasBody) {
    fetchOptions.body = JSON.stringify(request.body)
  }

  const response = await fetch(path, fetchOptions)

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }

  return response.json()
}

/**
 * Invoke a webhook by its nanoid webhook_id
 * Uses: POST /api/webhooks/{repo}/{webhookId}
 */
export async function invokeWebhook(
  repo: string,
  webhookId: string,
  request: WebhookInvokeRequest
): Promise<WebhookInvokeResponse> {
  const queryParams = new URLSearchParams()
  if (request.sync) queryParams.set('sync', 'true')
  if (request.timeout_ms) queryParams.set('timeout_ms', String(request.timeout_ms))
  if (request.query_params) {
    Object.entries(request.query_params).forEach(([key, value]) => {
      queryParams.set(key, value)
    })
  }

  const query = queryParams.toString()
  const path = `/api/webhooks/${repo}/${webhookId}${query ? `?${query}` : ''}`

  // Build headers - include auth headers and request-specific headers
  const headers: Record<string, string> = { ...getAuthHeaders(), ...request.headers }
  const hasBody = ['POST', 'PUT', 'PATCH'].includes(request.method) && request.body

  if (hasBody) {
    headers['Content-Type'] = 'application/json'
  }

  // Use the appropriate HTTP method
  const fetchOptions: RequestInit = {
    method: request.method,
    headers,
  }

  // Add body for methods that support it
  if (hasBody) {
    fetchOptions.body = JSON.stringify(request.body)
  }

  const response = await fetch(path, fetchOptions)

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }

  return response.json()
}

// Convenience exports
export const functionsApi = {
  listFunctions,
  getFunction,
  invokeFunction,
  listExecutions,
  getExecution,
  pollExecution,
  runFileStream,
  invokeTrigger,
  invokeWebhook,
}
