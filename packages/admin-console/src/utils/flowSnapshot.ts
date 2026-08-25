/**
 * Flow snapshot conversion utilities
 *
 * A flow instance stores an immutable `flow_definition_snapshot` that can be in
 * one of two formats:
 *
 * 1. DESIGNER format (`{ nodes: [{ node_type: 'raisin:FlowStep', ... }] }`)
 *    which the `@raisindb/flow-designer` FlowDesigner renders directly.
 * 2. RUNTIME format (`{ nodes: [{ id, step_type, next_node, ... }] }`)
 *    which the designer cannot render.
 *
 * `runtimeToDesignerFlow()` detects the format and, for runtime snapshots,
 * produces a best-effort read-only designer representation by walking the
 * `next_node` chain from the `start` node. Returns `null` when the snapshot
 * cannot be converted (callers should fall back to a JSON view).
 */

import { isValidTaskTypeSlug } from '@raisindb/flow-designer'
import type {
  FlowDefinition,
  FlowNode as DesignerFlowNode,
  FlowStep,
  FlowStepProperties,
  RaisinReference,
} from '@raisindb/flow-designer'

/** Runtime-format flow node (mirrors raisin-flow-runtime FlowNode) */
interface RuntimeFlowNode {
  id: string
  step_type: string
  properties?: Record<string, unknown>
  children?: RuntimeFlowNode[]
  next_node?: string | null
}

/** Safety cap when walking next_node chains (guards against cycles) */
const MAX_WALK = 500

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

/** Build a RaisinReference from a runtime string path */
function toReference(path: string, workspace?: string): RaisinReference {
  return {
    'raisin:ref': path,
    'raisin:workspace': workspace || 'default',
    'raisin:path': path,
  }
}

/** Last path segment, for display labels */
function lastSegment(path: string): string {
  const segments = path.split('/').filter(Boolean)
  return segments[segments.length - 1] || path
}

/**
 * Normalize a designer-format node tree: legacy snapshots may carry
 * reference fields (`function_ref`, `agent_ref`, `chat_config.agent_ref`) as a
 * plain string instead of a RaisinReference object. Convert each so the
 * renderer (which calls getRefDisplayName/getRefPath) always sees an object.
 */
function normalizeDesignerNodes(nodes: DesignerFlowNode[]): DesignerFlowNode[] {
  return nodes.map((node) => {
    if (node.node_type === 'raisin:FlowContainer') {
      return { ...node, children: normalizeDesignerNodes(node.children || []) }
    }
    const props = (node.properties || {}) as FlowStepProperties
    const nextProps: FlowStepProperties = { ...props }
    let changed = false

    if (typeof (props.function_ref as unknown) === 'string') {
      nextProps.function_ref = toReference(props.function_ref as unknown as string)
      changed = true
    }
    if (typeof (props.agent_ref as unknown) === 'string') {
      nextProps.agent_ref = toReference(props.agent_ref as unknown as string)
      changed = true
    }
    const chatConfig = props.chat_config as { agent_ref?: unknown } | undefined
    if (chatConfig && typeof chatConfig.agent_ref === 'string') {
      nextProps.chat_config = {
        ...chatConfig,
        agent_ref: toReference(chatConfig.agent_ref),
      } as FlowStepProperties['chat_config']
      changed = true
    }

    return changed ? { ...node, properties: nextProps } : node
  })
}

/** Convert a single runtime node into a read-only designer step */
function runtimeNodeToStep(node: RuntimeFlowNode): FlowStep {
  const props = node.properties || {}
  const action = asString(props.action)
  const functionRef = asString(props.function_ref)
  const functionWorkspace = asString(props.function_workspace)
  const agentRef = asString(props.agent_ref)
  const agentWorkspace = asString(props.agent_workspace)

  const stepProps: FlowStepProperties = {}

  switch (node.step_type) {
    case 'function_step':
      stepProps.action = action || (functionRef ? lastSegment(functionRef) : node.id)
      if (functionRef) {
        stepProps.function_ref = toReference(functionRef, functionWorkspace)
      }
      break

    case 'agent_step':
    case 'ai_agent':
      stepProps.step_type = 'ai_agent'
      stepProps.action = action || (agentRef ? lastSegment(agentRef) : node.id)
      if (agentRef) {
        stepProps.agent_ref = toReference(agentRef, agentWorkspace)
      }
      break

    case 'human_task': {
      stepProps.step_type = 'human_task'
      stepProps.action = action || asString(props.title) || node.id
      // Task types are an OPEN set: preserve any well-formed slug, not just
      // the canonical four, or a custom type would be silently dropped on
      // every snapshot round-trip.
      const taskType = asString(props.task_type)
      if (taskType && isValidTaskTypeSlug(taskType)) {
        stepProps.task_type = taskType
      }
      const assignee = asString(props.assignee)
      if (assignee) stepProps.assignee = assignee
      const description = asString(props.description)
      if (description) stepProps.task_description = description
      break
    }

    case 'chat':
    case 'chat_step':
    case 'chat_session':
      stepProps.step_type = 'chat'
      stepProps.action = action || node.id
      break

    case 'decision': {
      const condition = asString(props.condition)
      stepProps.action = action || (condition ? `decision: ${condition}` : `decision: ${node.id}`)
      break
    }

    case 'wait':
      stepProps.action = action || `wait: ${asString(props.wait_type) || node.id}`
      break

    case 'loop':
      stepProps.action = action || `loop: ${node.id}`
      break

    case 'parallel':
      stepProps.action = action || `parallel: ${node.id}`
      break

    case 'sub_flow':
      stepProps.action = action || `sub-flow: ${asString(props.flow_ref) || node.id}`
      break

    case 'ai_container':
    case 'ai_sequence':
      stepProps.step_type = 'ai_agent'
      stepProps.action = action || (agentRef ? lastSegment(agentRef) : `ai: ${node.id}`)
      if (agentRef) stepProps.agent_ref = toReference(agentRef, agentWorkspace)
      break

    // An agent picking the branch, and competing agents judged by a referee.
    // These fell through to the generic label below, which is a shame precisely
    // because they are the interesting nodes in an agent graph — a run showed
    // "agent_decision: route" instead of naming the agent doing the routing.
    case 'agent_decision':
    case 'ai_decision':
    case 'agent_router':
      stepProps.step_type = 'ai_agent'
      stepProps.action = action || (agentRef ? `route via ${lastSegment(agentRef)}` : `route: ${node.id}`)
      if (agentRef) stepProps.agent_ref = toReference(agentRef, agentWorkspace)
      break

    case 'competition':
    case 'ai_competition':
      stepProps.step_type = 'ai_agent'
      stepProps.action = action || `best of: ${node.id}`
      break

    case 'join':
      stepProps.action = action || `join: ${node.id}`
      break

    default:
      stepProps.action = action || `${node.step_type}: ${node.id}`
      break
  }

  return {
    id: node.id,
    node_type: 'raisin:FlowStep',
    properties: stepProps,
  }
}

/**
 * Convert a flow instance `flow_definition_snapshot` into a designer-format
 * FlowDefinition that FlowDesigner can render (read-only).
 *
 * - Designer-format snapshots are normalized and returned as-is.
 * - Runtime-format snapshots are mapped linearly by following `next_node`
 *   from the `start` node; non-linear constructs (decision branches, parallel
 *   branches, ...) are rendered as plain labeled steps (best effort).
 *
 * Returns `null` when the snapshot cannot be converted.
 */
export function runtimeToDesignerFlow(snapshot: unknown): FlowDefinition | null {
  if (!isRecord(snapshot)) return null
  const rawNodes = snapshot.nodes
  if (!Array.isArray(rawNodes)) return null

  const version = typeof snapshot.version === 'number' ? snapshot.version : 1
  const errorStrategy = snapshot.error_strategy === 'continue' ? 'continue' : 'fail_fast'

  // Empty flows are valid designer flows (no steps)
  if (rawNodes.length === 0) {
    return { version, error_strategy: errorStrategy, nodes: [] }
  }

  const first = rawNodes[0]
  if (!isRecord(first)) return null

  // Designer format: nodes carry node_type
  if (typeof first.node_type === 'string') {
    return {
      version,
      error_strategy: errorStrategy,
      timeout_ms: typeof snapshot.timeout_ms === 'number' ? snapshot.timeout_ms : undefined,
      nodes: normalizeDesignerNodes(rawNodes as DesignerFlowNode[]),
    }
  }

  // Runtime format: nodes carry step_type
  if (typeof first.step_type !== 'string') return null

  const runtimeNodes = rawNodes.filter(
    (n): n is RuntimeFlowNode => isRecord(n) && typeof n.id === 'string' && typeof n.step_type === 'string'
  ) as RuntimeFlowNode[]
  if (runtimeNodes.length === 0) return null

  const byId = new Map<string, RuntimeFlowNode>()
  for (const node of runtimeNodes) {
    byId.set(node.id, node)
  }

  const startNode =
    runtimeNodes.find((n) => n.step_type === 'start') || byId.get('start') || runtimeNodes[0]

  const ordered: RuntimeFlowNode[] = []
  const visited = new Set<string>()

  // Walk the next_node chain from start
  let cursor: RuntimeFlowNode | undefined = startNode
  let steps = 0
  while (cursor && steps < MAX_WALK) {
    steps++
    if (visited.has(cursor.id)) break
    visited.add(cursor.id)
    if (cursor.step_type !== 'start' && cursor.step_type !== 'end') {
      ordered.push(cursor)
    }
    cursor = cursor.next_node ? byId.get(cursor.next_node) : undefined
  }

  // Append unreachable nodes (e.g. decision branch targets) so they still
  // appear on the canvas — display only, order is best effort.
  for (const node of runtimeNodes) {
    if (visited.has(node.id)) continue
    if (node.step_type === 'start' || node.step_type === 'end') continue
    ordered.push(node)
  }

  if (ordered.length === 0) {
    // Only start/end nodes: render as an empty flow
    return { version, error_strategy: errorStrategy, nodes: [] }
  }

  return {
    version,
    error_strategy: errorStrategy,
    nodes: ordered.map(runtimeNodeToStep),
  }
}

/**
 * Collect step IDs that appear before `currentNodeId` in the converted
 * designer flow (linear walk including container children). Used as a
 * fallback for completed-step derivation when `variables.step_outputs`
 * is not available.
 */
export function collectStepIdsBefore(flow: FlowDefinition, currentNodeId: string): string[] {
  const result: string[] = []
  let found = false

  const walk = (nodes: DesignerFlowNode[]) => {
    for (const node of nodes) {
      if (found) return
      if (node.id === currentNodeId) {
        found = true
        return
      }
      if (node.node_type === 'raisin:FlowContainer') {
        walk(node.children || [])
        if (found) return
        continue
      }
      result.push(node.id)
    }
  }

  walk(flow.nodes)
  return found ? result : []
}

/** Collect every step ID in a designer flow (including container children) */
export function collectAllStepIds(flow: FlowDefinition): string[] {
  const result: string[] = []
  const walk = (nodes: DesignerFlowNode[]) => {
    for (const node of nodes) {
      if (node.node_type === 'raisin:FlowContainer') {
        result.push(node.id)
        walk(node.children || [])
      } else {
        result.push(node.id)
      }
    }
  }
  walk(flow.nodes)
  return result
}
