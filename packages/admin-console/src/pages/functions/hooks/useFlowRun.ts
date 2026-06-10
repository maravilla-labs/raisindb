/**
 * useFlowRun
 *
 * Encapsulates the run + SSE wiring for executing a flow from the flow editor.
 * Extracted from RaisinFlowNodeTypeEditor's inline Run dialog logic.
 *
 * Responsibilities:
 * - Start a real run (`flowsApi.runFlow`) or a test run (`flowsApi.testFlow`)
 * - Subscribe to flow execution events (SSE) and map them onto the designer's
 *   ExecutionState for canvas highlighting
 * - Subscribe to job events as a completion backup
 * - Collect raw events and per-step outputs for inspection
 * - When the flow waits on a human task, look up the pending inbox task for
 *   the instance so it can be completed inline (the SSE stream stays open
 *   while waiting; only terminal events close it)
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type { ExecutionState } from '@raisindb/flow-designer'
import {
  flowsApi,
  subscribeToFlowEvents,
  type FlowEvent,
  type FlowFunctionMock,
} from '../../../api/flows'
import { jobsApi } from '../../../api/jobs'
import {
  listInboxTasks,
  completeInboxTask,
  type InboxTask,
} from '../../../api/inbox'
import type { LogEntry } from '../types'

/** Per-step output collected from step_started / step_completed / step_failed events */
export interface StepOutputRecord {
  status: 'running' | 'completed' | 'failed'
  output?: unknown
  error?: string
  durationMs?: number
  /** Step name from the step_started event (if provided) */
  stepName?: string
  /** Timestamp of the most recent event for this step */
  timestamp?: string
}

export interface StartFlowRunOptions {
  /** Run via the test endpoint with mock support */
  test?: boolean
  /** Parsed input payload for the flow */
  input: unknown
  /** Function mock configuration keyed by function path (test runs only) */
  mockConfig?: Record<string, FlowFunctionMock>
  /** Agent mock configuration keyed by agent path (test runs only) */
  agentMockConfig?: Record<string, FlowFunctionMock>
}

export interface UseFlowRunArgs {
  repo: string
  /** Path to the raisin:Flow node */
  flowPath: string
  /** Display name used in log messages */
  flowName: string
  addLog: (log: LogEntry) => void
  clearLogs: () => void
}

export interface UseFlowRunResult {
  /** Start a run (real or test). Errors starting the run are logged, not thrown. */
  start: (options: StartFlowRunOptions) => Promise<void>
  /** Close all subscriptions and mark the run as no longer executing (does not cancel the server-side instance) */
  stop: () => void
  /** Designer execution state for canvas highlighting */
  executionState: ExecutionState
  /** All flow execution events received for the current run */
  events: FlowEvent[]
  /** Per-step outputs keyed by node ID */
  stepOutputs: Record<string, StepOutputRecord>
  /** Flow instance ID of the current/last run */
  instanceId: string | null
  /** Whether a run is currently executing */
  running: boolean
  /** Whether the current/last run was started in test mode */
  isTestRun: boolean
  /** Pending inbox task when the flow is waiting on a human task */
  waitingTask: InboxTask | null
  /** Complete the waiting inbox task; clears waitingTask on success. Throws on API error. */
  completeWaitingTask: (response: Record<string, unknown>) => Promise<void>
}

function createIdleExecutionState(isExecuting = false): ExecutionState {
  return {
    completedNodeIds: new Set<string>(),
    failedNodeIds: new Set<string>(),
    isExecuting,
  }
}

/** How many times to retry the inbox lookup after a flow_waiting event */
const WAITING_TASK_RETRIES = 5
/** Delay between inbox lookup retries (indexing is near-instant, but be tolerant) */
const WAITING_TASK_RETRY_DELAY_MS = 500

export function useFlowRun({
  repo,
  flowPath,
  flowName,
  addLog,
  clearLogs,
}: UseFlowRunArgs): UseFlowRunResult {
  const [executionState, setExecutionState] = useState<ExecutionState>(() =>
    createIdleExecutionState()
  )
  const [events, setEvents] = useState<FlowEvent[]>([])
  const [stepOutputs, setStepOutputs] = useState<Record<string, StepOutputRecord>>({})
  const [instanceId, setInstanceId] = useState<string | null>(null)
  const [running, setRunning] = useState(false)
  const [isTestRun, setIsTestRun] = useState(false)
  const [waitingTask, setWaitingTask] = useState<InboxTask | null>(null)

  // Active subscriptions for cleanup on stop/unmount/restart
  const unsubscribersRef = useRef<Array<() => void>>([])
  // Instance the hook currently tracks; used to drop stale async results
  const activeInstanceRef = useRef<string | null>(null)

  const cleanupSubscriptions = useCallback(() => {
    for (const unsubscribe of unsubscribersRef.current) {
      try {
        unsubscribe()
      } catch {
        // Ignore cleanup errors
      }
    }
    unsubscribersRef.current = []
  }, [])

  // Close subscriptions when the editor unmounts
  useEffect(() => {
    return () => {
      activeInstanceRef.current = null
      cleanupSubscriptions()
    }
  }, [cleanupSubscriptions])

  const stop = useCallback(() => {
    activeInstanceRef.current = null
    cleanupSubscriptions()
    setRunning(false)
    setWaitingTask(null)
    setExecutionState((prev) => ({
      ...prev,
      currentNodeId: undefined,
      waitingNodeId: undefined,
      isExecuting: false,
    }))
  }, [cleanupSubscriptions])

  /**
   * Look up the pending inbox task for a waiting flow instance.
   * Retries a few times since task indexing may lag the flow_waiting event.
   */
  const fetchWaitingTask = useCallback(
    async (forInstanceId: string) => {
      for (let attempt = 0; attempt < WAITING_TASK_RETRIES; attempt++) {
        if (activeInstanceRef.current !== forInstanceId) return
        try {
          const result = await listInboxTasks(repo)
          const task = result.tasks.find(
            (t) => t.flow_instance_id === forInstanceId && t.status === 'pending'
          )
          if (task) {
            if (activeInstanceRef.current === forInstanceId) {
              setWaitingTask(task)
            }
            return
          }
        } catch {
          // Tolerate transient errors; retry below
        }
        await new Promise((resolve) => setTimeout(resolve, WAITING_TASK_RETRY_DELAY_MS))
      }
      addLog({
        level: 'warn',
        message:
          'Flow is waiting on a human task, but no pending inbox task was found for this instance. Check the Inbox page.',
        timestamp: new Date().toISOString(),
      })
    },
    [repo, addLog]
  )

  const start = useCallback(
    async ({ test, input, mockConfig, agentMockConfig }: StartFlowRunOptions) => {
      // Reset any previous run
      activeInstanceRef.current = null
      cleanupSubscriptions()
      setRunning(true)
      setIsTestRun(Boolean(test))
      setEvents([])
      setStepOutputs({})
      setWaitingTask(null)
      setInstanceId(null)
      clearLogs()
      setExecutionState(createIdleExecutionState(true))
      addLog({
        level: 'info',
        message: `Starting flow: ${flowName}${test ? ' (test run)' : ''}`,
        timestamp: new Date().toISOString(),
      })

      try {
        const result = test
          ? await flowsApi.testFlow(repo, {
              flow_path: flowPath,
              input,
              test_config: {
                is_test_run: true,
                mock_functions: mockConfig ?? {},
                mock_agents: agentMockConfig ?? {},
              },
            })
          : await flowsApi.runFlow(repo, {
              flow_path: flowPath,
              input,
            })

        activeInstanceRef.current = result.instance_id
        setInstanceId(result.instance_id)
        addLog({
          level: 'info',
          message: `Flow queued: ${result.instance_id}`,
          timestamp: new Date().toISOString(),
        })

        // Subscribe to flow step events for canvas highlighting.
        // NOTE: the stream intentionally stays open on flow_waiting; only
        // terminal events (flow_completed / flow_failed / job terminal) close it.
        const unsubscribeFlow = subscribeToFlowEvents(repo, result.instance_id, {
          onEvent: (event) => {
            setEvents((prev) => [...prev, event])
          },
          onStepStarted: (event) => {
            addLog({ level: 'debug', message: `Step started: ${event.node_id}`, timestamp: event.timestamp })
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: event.node_id,
              waitingNodeId: undefined,
            }))
            setStepOutputs((prev) => ({
              ...prev,
              [event.node_id]: {
                status: 'running',
                stepName: event.step_name,
                timestamp: event.timestamp,
              },
            }))
          },
          onStepCompleted: (event) => {
            addLog({ level: 'debug', message: `Step completed: ${event.node_id} (${event.duration_ms}ms)`, timestamp: event.timestamp })
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: undefined,
              completedNodeIds: new Set([...prev.completedNodeIds, event.node_id]),
            }))
            setStepOutputs((prev) => ({
              ...prev,
              [event.node_id]: {
                ...prev[event.node_id],
                status: 'completed',
                output: event.output,
                durationMs: event.duration_ms,
                timestamp: event.timestamp,
              },
            }))
          },
          onStepFailed: (event) => {
            addLog({ level: 'error', message: `Step failed: ${event.node_id} - ${event.error}`, timestamp: event.timestamp })
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: undefined,
              failedNodeIds: new Set([...prev.failedNodeIds, event.node_id]),
            }))
            setStepOutputs((prev) => ({
              ...prev,
              [event.node_id]: {
                ...prev[event.node_id],
                status: 'failed',
                error: event.error,
                durationMs: event.duration_ms,
                timestamp: event.timestamp,
              },
            }))
          },
          onFlowWaiting: (event) => {
            addLog({ level: 'info', message: `Flow waiting: ${event.reason}`, timestamp: event.timestamp })
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: undefined,
              waitingNodeId: event.node_id,
            }))
            // Human task: look up the pending inbox task for inline completion
            if (event.reason === 'human_task' || event.wait_type === 'human_task') {
              void fetchWaitingTask(result.instance_id)
            }
          },
          onFlowResumed: (event) => {
            addLog({ level: 'info', message: `Flow resumed from: ${event.node_id}`, timestamp: event.timestamp })
            setWaitingTask(null)
            setExecutionState((prev) => ({
              ...prev,
              waitingNodeId: undefined,
            }))
          },
          onFlowCompleted: (event) => {
            addLog({ level: 'info', message: `Flow completed (${event.total_duration_ms}ms)`, timestamp: event.timestamp })
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: undefined,
              waitingNodeId: undefined,
              isExecuting: false,
            }))
            setRunning(false)
            setWaitingTask(null)
            unsubscribeFlow()
          },
          onFlowFailed: (event) => {
            addLog({ level: 'error', message: `Flow failed: ${event.error}`, timestamp: event.timestamp })
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: undefined,
              waitingNodeId: undefined,
              isExecuting: false,
            }))
            setRunning(false)
            setWaitingTask(null)
            unsubscribeFlow()
          },
          onLog: (event) => {
            addLog({
              level: event.level as 'debug' | 'info' | 'warn' | 'error',
              message: event.message,
              timestamp: event.timestamp,
            })
          },
        })
        unsubscribersRef.current.push(unsubscribeFlow)

        // Subscribe to job events for this specific job (for overall status)
        const unsubscribeJob = jobsApi.subscribeToJobEvents((event) => {
          if (event.job_id !== result.job_id) return

          // Add any logs from the event
          if (event.logs) {
            for (const log of event.logs) {
              addLog({
                level: log.level as 'debug' | 'info' | 'warn' | 'error',
                message: log.message,
                timestamp: log.timestamp,
              })
            }
          }

          // Check for completion (backup in case flow events don't arrive)
          if (event.status === 'Completed') {
            if (event.function_result) {
              // Check for flow_status and flow_error in the result
              const flowResult = event.function_result as { flow_status?: string; flow_error?: string; instance_id?: string; current_node_id?: string }
              if (flowResult.flow_status === 'failed' && flowResult.flow_error) {
                addLog({ level: 'error', message: `Flow failed: ${flowResult.flow_error}`, timestamp: new Date().toISOString() })
                // Highlight the failed step if we know which one
                const failedNode = flowResult.current_node_id
                setExecutionState((prev) => ({
                  ...prev,
                  isExecuting: false,
                  failedNodeIds: failedNode ? new Set([...prev.failedNodeIds, failedNode]) : prev.failedNodeIds,
                }))
              } else if (flowResult.flow_status === 'waiting') {
                addLog({ level: 'info', message: `Flow waiting (instance: ${flowResult.instance_id})`, timestamp: new Date().toISOString() })
              } else if (flowResult.flow_status === 'completed') {
                addLog({ level: 'info', message: `Flow completed (instance: ${flowResult.instance_id})`, timestamp: new Date().toISOString() })
              } else {
                addLog({ level: 'info', message: `Result: ${JSON.stringify(event.function_result)}`, timestamp: new Date().toISOString() })
              }
            }
            // Clean up both subscriptions
            setRunning(false)
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: undefined,
              waitingNodeId: undefined,
              isExecuting: false,
            }))
            unsubscribeJob()
            unsubscribeFlow()
          } else if (event.status.startsWith('Failed')) {
            addLog({ level: 'error', message: `Job failed: ${event.error || 'Unknown error'}`, timestamp: new Date().toISOString() })
            setRunning(false)
            setExecutionState((prev) => ({
              ...prev,
              currentNodeId: undefined,
              waitingNodeId: undefined,
              isExecuting: false,
            }))
            unsubscribeJob()
            unsubscribeFlow()
          }
        })
        unsubscribersRef.current.push(unsubscribeJob)
      } catch (error) {
        addLog({ level: 'error', message: `Failed to start flow: ${error}`, timestamp: new Date().toISOString() })
        setRunning(false)
        setExecutionState(createIdleExecutionState())
      }
    },
    [repo, flowPath, flowName, addLog, clearLogs, cleanupSubscriptions, fetchWaitingTask]
  )

  const completeWaitingTask = useCallback(
    async (response: Record<string, unknown>) => {
      if (!waitingTask) return
      await completeInboxTask(repo, waitingTask.id, response)
      addLog({
        level: 'info',
        message: `Task completed: ${waitingTask.title}`,
        timestamp: new Date().toISOString(),
      })
      // Clear the task; the SSE stream stays open and the flow_resumed /
      // subsequent step events keep flowing on the same subscription.
      setWaitingTask(null)
    },
    [repo, waitingTask, addLog]
  )

  return {
    start,
    stop,
    executionState,
    events,
    stepOutputs,
    instanceId,
    running,
    isTestRun,
    waitingTask,
    completeWaitingTask,
  }
}

export default useFlowRun
