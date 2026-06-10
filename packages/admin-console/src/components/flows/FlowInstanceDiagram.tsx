/**
 * FlowInstanceDiagram Component
 *
 * Read-only canvas view of a flow instance using @raisindb/flow-designer.
 * Shows the flow definition snapshot with the current execution position
 * highlighted, plus who/what started the instance.
 *
 * For running/waiting instances it subscribes to the per-instance SSE event
 * stream and updates the highlighted position live.
 */

import { useEffect, useMemo, useState } from 'react'
import { Workflow, Zap, Clock3, User } from 'lucide-react'
import { FlowDesigner, type ExecutionState, type FlowDefinition } from '@raisindb/flow-designer'
import { subscribeToFlowEvents } from '../../api/flows'
import { runtimeToDesignerFlow, collectStepIdsBefore, collectAllStepIds } from '../../utils/flowSnapshot'

/** Minimal flow-instance shape required by the diagram */
export interface DiagramFlowInstance {
  id: string
  flow_ref: string
  status: 'pending' | 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled' | 'rolled_back'
  current_node_id?: string
  started_at?: string
  flow_definition_snapshot?: unknown
  variables?: Record<string, unknown>
}

interface FlowInstanceDiagramProps {
  repo: string
  instance: DiagramFlowInstance
}

/** Trigger info stored under variables.__trigger_info by the flow runtime */
interface TriggerInfo {
  event_type?: string
  node_path?: string
  node_type?: string
  actor?: string
  workspace?: string
}

const STATUS_BADGE: Record<DiagramFlowInstance['status'], { label: string; className: string }> = {
  pending: { label: 'Pending', className: 'bg-zinc-500/10 text-zinc-400' },
  running: { label: 'Running', className: 'bg-blue-500/10 text-blue-400' },
  waiting: { label: 'Waiting', className: 'bg-yellow-500/10 text-yellow-400' },
  completed: { label: 'Completed', className: 'bg-green-500/10 text-green-400' },
  failed: { label: 'Failed', className: 'bg-red-500/10 text-red-400' },
  cancelled: { label: 'Cancelled', className: 'bg-orange-500/10 text-orange-400' },
  rolled_back: { label: 'Rolled Back', className: 'bg-purple-500/10 text-purple-400' },
}

/** Extract __trigger_info from instance variables (defensive) */
function getTriggerInfo(instance: DiagramFlowInstance): TriggerInfo | null {
  const raw = instance.variables?.__trigger_info
  if (typeof raw !== 'object' || raw === null) return null
  return raw as TriggerInfo
}

/** Human-readable started-by label */
function describeTrigger(info: TriggerInfo | null): string {
  if (!info || !info.event_type) return 'Manual run'
  const actorSuffix = info.actor ? ` by ${info.actor}` : ''
  switch (info.event_type) {
    case 'manual':
      return `Manual run${actorSuffix}`
    case 'scheduled':
      return `Scheduled run${actorSuffix}`
    case 'created':
    case 'updated':
    case 'deleted': {
      const event = info.event_type.charAt(0).toUpperCase() + info.event_type.slice(1)
      return info.node_path
        ? `Triggered by ${event} on ${info.node_path}`
        : `Triggered by ${event}${actorSuffix}`
    }
    default:
      return info.node_path
        ? `Triggered by ${info.event_type} on ${info.node_path}`
        : `Triggered by ${info.event_type}${actorSuffix}`
  }
}

/** IDs of completed steps from variables.step_outputs (preferred source) */
function getStepOutputIds(instance: DiagramFlowInstance): string[] {
  const outputs = instance.variables?.step_outputs
  if (typeof outputs !== 'object' || outputs === null || Array.isArray(outputs)) return []
  return Object.keys(outputs)
}

/** Derive the initial ExecutionState from a flow instance */
function deriveExecutionState(
  instance: DiagramFlowInstance,
  flow: FlowDefinition | null
): ExecutionState {
  const completed = new Set<string>(getStepOutputIds(instance))
  const failed = new Set<string>()

  // Fallback: walk the linear chain up to the current node
  if (completed.size === 0 && flow && instance.current_node_id) {
    for (const id of collectStepIdsBefore(flow, instance.current_node_id)) {
      completed.add(id)
    }
  }

  let currentNodeId: string | undefined
  let waitingNodeId: string | undefined

  switch (instance.status) {
    case 'running':
      currentNodeId = instance.current_node_id
      break
    case 'waiting':
      waitingNodeId = instance.current_node_id
      break
    case 'failed':
      if (instance.current_node_id) {
        failed.add(instance.current_node_id)
        completed.delete(instance.current_node_id)
      }
      break
    case 'completed':
      // Show every step as completed for finished flows
      if (flow) {
        for (const id of collectAllStepIds(flow)) completed.add(id)
      }
      break
    default:
      break
  }

  if (currentNodeId) completed.delete(currentNodeId)
  if (waitingNodeId) completed.delete(waitingNodeId)

  // isExecuting gates status rendering inside FlowDesigner, so keep it on
  // whenever there is any execution position/progress to display.
  const isExecuting =
    instance.status === 'running' ||
    instance.status === 'waiting' ||
    instance.status === 'failed' ||
    instance.status === 'completed' ||
    completed.size > 0

  return {
    currentNodeId,
    waitingNodeId,
    completedNodeIds: completed,
    failedNodeIds: failed,
    isExecuting,
  }
}

export default function FlowInstanceDiagram({ repo, instance }: FlowInstanceDiagramProps) {
  // Convert the snapshot once per instance
  const flow = useMemo(
    () => runtimeToDesignerFlow(instance.flow_definition_snapshot),
    [instance.flow_definition_snapshot]
  )

  const [executionState, setExecutionState] = useState<ExecutionState>(() =>
    deriveExecutionState(instance, flow)
  )

  // Re-derive when the instance status/position changes (e.g. list refresh)
  useEffect(() => {
    setExecutionState(deriveExecutionState(instance, flow))
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instance.status, instance.current_node_id, flow])

  // Live updates for in-flight instances
  const isLive = instance.status === 'running' || instance.status === 'waiting'
  useEffect(() => {
    if (!isLive || !repo) return

    const unsubscribe = subscribeToFlowEvents(repo, instance.id, {
      onStepStarted: (event) => {
        setExecutionState((prev) => ({
          ...prev,
          currentNodeId: event.node_id,
          waitingNodeId: undefined,
          isExecuting: true,
        }))
      },
      onStepCompleted: (event) => {
        setExecutionState((prev) => ({
          ...prev,
          currentNodeId: prev.currentNodeId === event.node_id ? undefined : prev.currentNodeId,
          completedNodeIds: new Set([...prev.completedNodeIds, event.node_id]),
        }))
      },
      onStepFailed: (event) => {
        setExecutionState((prev) => ({
          ...prev,
          currentNodeId: prev.currentNodeId === event.node_id ? undefined : prev.currentNodeId,
          failedNodeIds: new Set([...prev.failedNodeIds, event.node_id]),
        }))
      },
      onFlowWaiting: (event) => {
        setExecutionState((prev) => ({
          ...prev,
          currentNodeId: undefined,
          waitingNodeId: event.node_id,
        }))
      },
      onFlowResumed: (event) => {
        setExecutionState((prev) => ({
          ...prev,
          waitingNodeId: undefined,
          currentNodeId: event.node_id,
        }))
      },
      onFlowCompleted: () => {
        setExecutionState((prev) => ({
          ...prev,
          currentNodeId: undefined,
          waitingNodeId: undefined,
        }))
      },
      onFlowFailed: (event) => {
        setExecutionState((prev) => {
          const failed = new Set(prev.failedNodeIds)
          if (event.failed_at_node) failed.add(event.failed_at_node)
          return {
            ...prev,
            currentNodeId: undefined,
            waitingNodeId: undefined,
            failedNodeIds: failed,
          }
        })
      },
    })

    return unsubscribe
  }, [isLive, repo, instance.id])

  const triggerInfo = getTriggerInfo(instance)
  const badge = STATUS_BADGE[instance.status] || STATUS_BADGE.pending

  // Header bar shared by both render paths
  const header = (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 bg-white/5 border-b border-white/10 rounded-t-lg">
      <div className="flex items-center gap-2 min-w-0">
        <Workflow className="w-4 h-4 text-purple-400 flex-shrink-0" />
        <span className="text-sm text-white font-medium truncate" title={instance.flow_ref}>
          {instance.flow_ref}
        </span>
      </div>
      <span className={`px-2 py-0.5 rounded-full text-xs font-medium ${badge.className}`}>
        {badge.label}
      </span>
      <div className="flex items-center gap-1.5 text-xs text-zinc-400" title="Started by">
        {triggerInfo?.actor ? (
          <User className="w-3.5 h-3.5 text-zinc-500" />
        ) : (
          <Zap className="w-3.5 h-3.5 text-zinc-500" />
        )}
        <span className="truncate max-w-[360px]">{describeTrigger(triggerInfo)}</span>
      </div>
      {instance.started_at && (
        <div className="flex items-center gap-1.5 text-xs text-zinc-500 ml-auto">
          <Clock3 className="w-3.5 h-3.5" />
          {new Date(instance.started_at).toLocaleString()}
        </div>
      )}
    </div>
  )

  // Fallback: snapshot missing or unconvertible — show JSON with a notice
  if (!flow) {
    return (
      <div className="border border-white/10 rounded-lg overflow-hidden">
        {header}
        <div className="p-4 space-y-3">
          <div className="text-xs text-yellow-300/80 bg-yellow-500/10 border border-yellow-500/20 rounded-lg px-3 py-2">
            This flow snapshot could not be rendered as a diagram. Showing the raw definition instead.
          </div>
          {instance.flow_definition_snapshot != null ? (
            <pre className="text-xs text-zinc-400 bg-black/30 p-3 rounded-lg overflow-auto max-h-72">
              {JSON.stringify(instance.flow_definition_snapshot, null, 2)}
            </pre>
          ) : (
            <p className="text-sm text-zinc-500">No flow definition snapshot stored on this instance.</p>
          )}
        </div>
      </div>
    )
  }

  return (
    <div className="border border-white/10 rounded-lg overflow-hidden">
      {header}
      <div className="h-[480px] bg-black/20">
        <FlowDesigner
          key={instance.id}
          flow={flow}
          disabled
          showPalette={false}
          showToolbar={false}
          theme="dark"
          executionState={executionState}
          className="h-full"
        />
      </div>
    </div>
  )
}
