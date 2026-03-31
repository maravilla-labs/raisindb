/**
 * Flow execution API
 *
 * API functions for executing flows via the raisin-flow-runtime.
 */

import { getAuthHeaders } from './client'
import { fetchEventSource } from '@microsoft/fetch-event-source'

// Types

/** Flow execution event types for real-time step tracking */
export type FlowEventType =
  | 'step_started'
  | 'step_completed'
  | 'step_failed'
  | 'flow_waiting'
  | 'flow_resumed'
  | 'flow_completed'
  | 'flow_failed'
  | 'log'

/** Base flow event structure */
export interface FlowEventBase {
  type: FlowEventType
  timestamp: string
}

/** Step started event */
export interface StepStartedEvent extends FlowEventBase {
  type: 'step_started'
  node_id: string
  step_name?: string
  step_type: string
}

/** Step completed event */
export interface StepCompletedEvent extends FlowEventBase {
  type: 'step_completed'
  node_id: string
  output: unknown
  duration_ms: number
}

/** Step failed event */
export interface StepFailedEvent extends FlowEventBase {
  type: 'step_failed'
  node_id: string
  error: string
  duration_ms: number
}

/** Flow waiting event */
export interface FlowWaitingEvent extends FlowEventBase {
  type: 'flow_waiting'
  node_id: string
  wait_type: string
  reason: string
}

/** Flow resumed event */
export interface FlowResumedEvent extends FlowEventBase {
  type: 'flow_resumed'
  node_id: string
  wait_duration_ms: number
}

/** Flow completed event */
export interface FlowCompletedEvent extends FlowEventBase {
  type: 'flow_completed'
  output: unknown
  total_duration_ms: number
}

/** Flow failed event */
export interface FlowFailedEvent extends FlowEventBase {
  type: 'flow_failed'
  error: string
  failed_at_node?: string
  total_duration_ms: number
}

/** Log event */
export interface LogEvent extends FlowEventBase {
  type: 'log'
  level: string
  message: string
  node_id?: string
}

/** Union of all flow event types */
export type FlowEvent =
  | StepStartedEvent
  | StepCompletedEvent
  | StepFailedEvent
  | FlowWaitingEvent
  | FlowResumedEvent
  | FlowCompletedEvent
  | FlowFailedEvent
  | LogEvent

export interface RunFlowRequest {
  /** Path to the raisin:Flow node */
  flow_path: string
  /** Input data passed to the flow */
  input?: unknown
}

export interface RunFlowResponse {
  /** The created flow instance ID */
  instance_id: string
  /** Job ID for tracking execution */
  job_id: string
  /** Status (always "queued" for async execution) */
  status: string
}

// API Functions

/**
 * Run a flow by path
 *
 * Queues a FlowInstanceExecution job and returns the instance ID and job ID
 * for tracking. Subscribe to /management/events/jobs SSE for real-time updates.
 */
export async function runFlow(
  repo: string,
  request: RunFlowRequest
): Promise<RunFlowResponse> {
  const response = await fetch(`/api/flows/${repo}/run`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...getAuthHeaders() },
    body: JSON.stringify(request),
  })

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }

  return response.json()
}

/**
 * Callbacks for flow event subscription
 */
export interface FlowEventCallbacks {
  onStepStarted?: (event: StepStartedEvent) => void
  onStepCompleted?: (event: StepCompletedEvent) => void
  onStepFailed?: (event: StepFailedEvent) => void
  onFlowWaiting?: (event: FlowWaitingEvent) => void
  onFlowResumed?: (event: FlowResumedEvent) => void
  onFlowCompleted?: (event: FlowCompletedEvent) => void
  onFlowFailed?: (event: FlowFailedEvent) => void
  onLog?: (event: LogEvent) => void
  /** Called for any event */
  onEvent?: (event: FlowEvent) => void
  /** Called on connection error */
  onError?: (error: Event) => void
}

/**
 * Subscribe to flow execution events via SSE
 *
 * Connects to the flow events SSE endpoint and dispatches events to callbacks.
 * Returns a cleanup function to close the connection.
 *
 * @param repo - Repository ID
 * @param instanceId - Flow instance ID
 * @param callbacks - Event callbacks
 * @returns Cleanup function to close the SSE connection
 */
export function subscribeToFlowEvents(
  repo: string,
  instanceId: string,
  callbacks: FlowEventCallbacks
): () => void {
  const controller = new AbortController()
  const url = `/api/flows/${repo}/instances/${instanceId}/events`

  // Get auth headers (without impersonation for SSE)
  const authHeaders = getAuthHeaders()
  delete authHeaders['X-Raisin-Impersonate']

  fetchEventSource(url, {
    headers: authHeaders,
    signal: controller.signal,
    openWhenHidden: true,

    onmessage: (event) => {
      if (event.event === 'flow-event') {
        try {
          const data = JSON.parse(event.data) as FlowEvent

          // Call the generic onEvent callback
          callbacks.onEvent?.(data)

          // Call specific callbacks based on event type
          switch (data.type) {
            case 'step_started':
              callbacks.onStepStarted?.(data)
              break
            case 'step_completed':
              callbacks.onStepCompleted?.(data)
              break
            case 'step_failed':
              callbacks.onStepFailed?.(data)
              break
            case 'flow_waiting':
              callbacks.onFlowWaiting?.(data)
              break
            case 'flow_resumed':
              callbacks.onFlowResumed?.(data)
              break
            case 'flow_completed':
              callbacks.onFlowCompleted?.(data)
              break
            case 'flow_failed':
              callbacks.onFlowFailed?.(data)
              break
            case 'log':
              callbacks.onLog?.(data)
              break
          }
        } catch (e) {
          console.error('Failed to parse flow event:', e)
        }
      }
    },

    onerror: (error) => {
      console.error('Flow events SSE connection error:', error)
      callbacks.onError?.(error as unknown as Event)
    },
  })

  // Return cleanup function
  return () => {
    controller.abort()
  }
}

/**
 * Cancel a running or waiting flow instance
 */
export async function cancelFlowInstance(
  repo: string,
  instanceId: string
): Promise<{ id: string; status: string }> {
  const response = await fetch(`/api/flows/${repo}/instances/${instanceId}/cancel`, {
    method: 'POST',
    headers: { ...getAuthHeaders() },
  })

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }

  return response.json()
}

/**
 * Delete a flow instance (must be in terminal state)
 */
export async function deleteFlowInstance(
  repo: string,
  instanceId: string
): Promise<void> {
  const response = await fetch(`/api/flows/${repo}/instances/${instanceId}`, {
    method: 'DELETE',
    headers: { ...getAuthHeaders() },
  })

  if (!response.ok) {
    const errorText = await response.text()
    throw new Error(`HTTP ${response.status}: ${errorText}`)
  }
}

// Convenience exports
export const flowsApi = {
  runFlow,
  subscribeToFlowEvents,
  cancelFlowInstance,
  deleteFlowInstance,
}
