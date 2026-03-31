import { sqlApi } from './sql'
import { nodesApi, type Node } from './nodes'

// ===== TYPES =====

export interface AgentConversation {
  path: string
  name: string
  id: string
  subject?: string
  agentRef?: string
  conversationType?: string
  status?: string
  createdAt?: string
  updatedAt?: string
  messageCount: number
  hasErrors: boolean
}

export interface ToolCallTrace {
  path: string
  name: string
  toolCallId: string
  functionName: string
  arguments?: unknown
  status: string
  result?: unknown
  error?: string
  durationMs?: number
}

export interface ThoughtTrace {
  path: string
  content: string
  thoughtType?: string
}

export interface CostRecord {
  path: string
  model?: string
  provider?: string
  inputTokens?: number
  outputTokens?: number
}

export interface PlanTask {
  path: string
  name: string
  title: string
  status: string
  description?: string
}

export interface PlanTrace {
  path: string
  name: string
  title?: string
  status?: string
  tasks: PlanTask[]
}

export interface ExecutionDiagnostics {
  history_length?: number
  tools_available?: number
  planning_enabled?: boolean
  handler?: string
  continuation_depth?: number
  timestamp?: string
}

export interface ErrorDetails {
  type?: string
  message?: string
  finish_reason?: string
  model?: string
  was_retry?: boolean
  plan_action_id?: string
  looped_tools?: string
  depth?: number
}

export interface ConversationMessage {
  path: string
  name: string
  id: string
  role: string
  content?: string
  messageType?: string
  status?: string
  senderDisplayName?: string
  finishReason?: string
  model?: string
  tokens?: { input?: number; output?: number }
  createdAt?: string
  executionDiagnostics?: ExecutionDiagnostics
  errorDetails?: ErrorDetails
  planActionId?: string
  continuationDepth?: number
  toolCalls: ToolCallTrace[]
  thoughts: ThoughtTrace[]
  plans: PlanTrace[]
  costRecords: CostRecord[]
}

export interface ConversationTree {
  conversation: AgentConversation
  messages: ConversationMessage[]
}

// ===== HELPERS =====

const WORKSPACE = 'ai'
const BRANCH = 'main'

function extractAgentRef(props: Record<string, unknown> | undefined): string | undefined {
  if (!props?.agent_ref) return undefined
  const ref = props.agent_ref
  if (typeof ref === 'string') return ref
  if (ref && typeof ref === 'object' && 'raisin:path' in ref) {
    return (ref as Record<string, string>)['raisin:path']
  }
  return undefined
}

function nodeToConversation(node: Node, messageCount = 0, hasErrors = false): AgentConversation {
  return {
    path: node.path,
    name: node.name,
    id: node.id,
    subject: node.properties?.subject as string | undefined,
    agentRef: extractAgentRef(node.properties),
    conversationType: node.properties?.conversation_type as string | undefined,
    status: node.properties?.status as string | undefined,
    createdAt: node.created_at,
    updatedAt: node.properties?.updated_at as string | undefined,
    messageCount,
    hasErrors,
  }
}

function nodeToMessage(node: Node): ConversationMessage {
  const props = node.properties ?? {}
  const body = props.body as Record<string, unknown> | undefined
  const tokens = props.tokens as { input?: number; output?: number } | undefined

  return {
    path: node.path,
    name: node.name,
    id: node.id,
    role: (props.role as string) ?? 'unknown',
    content: (body?.content as string) ?? (body?.message_text as string) ?? undefined,
    messageType: props.message_type as string | undefined,
    status: props.status as string | undefined,
    senderDisplayName: props.sender_display_name as string | undefined,
    finishReason: props.finish_reason as string | undefined,
    model: props.model as string | undefined,
    tokens,
    createdAt: props.created_at as string | undefined,
    executionDiagnostics: props.execution_diagnostics as ExecutionDiagnostics | undefined,
    errorDetails: props.error_details as ErrorDetails | undefined,
    planActionId: props.plan_action_id as string | undefined,
    continuationDepth: props.continuation_depth as number | undefined,
    toolCalls: [],
    thoughts: [],
    plans: [],
    costRecords: [],
  }
}

function nodeToToolCall(node: Node): ToolCallTrace {
  const props = node.properties ?? {}
  return {
    path: node.path,
    name: node.name,
    toolCallId: (props.tool_call_id as string) ?? node.name,
    functionName: (props.function_name as string) ?? 'unknown',
    arguments: props.arguments as unknown,
    status: (props.status as string) ?? 'unknown',
  }
}

function nodeToThought(node: Node): ThoughtTrace {
  const props = node.properties ?? {}
  return {
    path: node.path,
    content: (props.content as string) ?? '',
    thoughtType: props.thought_type as string | undefined,
  }
}

function nodeToCostRecord(node: Node): CostRecord {
  const props = node.properties ?? {}
  return {
    path: node.path,
    model: props.model as string | undefined,
    provider: props.provider as string | undefined,
    inputTokens: props.input_tokens as number | undefined,
    outputTokens: props.output_tokens as number | undefined,
  }
}

function nodeToPlanTask(node: Node): PlanTask {
  const props = node.properties ?? {}
  return {
    path: node.path,
    name: node.name,
    title: (props.title as string) ?? node.name,
    status: (props.status as string) ?? 'pending',
    description: props.description as string | undefined,
  }
}

// ===== API =====

export const agentConversationsApi = {
  /**
   * List conversations for a specific agent.
   * Queries the `ai` workspace for raisin:Conversation nodes whose
   * agent_ref contains the agent path.
   */
  listConversations: async (repo: string, agentName: string): Promise<AgentConversation[]> => {
    // Agent conversations are stored under /agents/{name}/inbox/chats/ in the ai workspace
    const sql = `
      SELECT path, id, name, properties, created_at, updated_at
      FROM "ai"
      WHERE node_type = 'raisin:Conversation'
        AND DESCENDANT_OF($1)
      ORDER BY created_at DESC
    `
    const agentBasePath = `/agents/${agentName}`
    console.debug('[agent-conversations] listConversations', { repo, agentName, agentBasePath, sql: sql.trim() })
    const result = await sqlApi.executeQuery(repo, sql, [agentBasePath])
    console.debug('[agent-conversations] listConversations result:', { rowCount: result.rows.length, rows: result.rows })

    return result.rows.map(row => {
      const props = typeof row.properties === 'string'
        ? JSON.parse(row.properties)
        : row.properties
      return nodeToConversation({
        id: row.id,
        name: row.name,
        path: row.path,
        node_type: 'raisin:Conversation',
        properties: props,
        created_at: row.created_at,
        updated_at: row.updated_at,
      })
    })
  },

  /**
   * List all agent conversations (across all agents).
   */
  listAllConversations: async (repo: string): Promise<AgentConversation[]> => {
    const sql = `
      SELECT path, id, name, properties, created_at, updated_at
      FROM "ai"
      WHERE node_type = 'raisin:Conversation'
        AND properties->>'conversation_type'::STRING = 'ai_chat'
      ORDER BY created_at DESC
    `
    console.debug('[agent-conversations] listAllConversations', { repo, sql: sql.trim() })
    const result = await sqlApi.executeQuery(repo, sql)
    console.debug('[agent-conversations] listAllConversations result:', { rowCount: result.rows.length, rows: result.rows })

    return result.rows.map(row => {
      const props = typeof row.properties === 'string'
        ? JSON.parse(row.properties)
        : row.properties
      return nodeToConversation({
        id: row.id,
        name: row.name,
        path: row.path,
        node_type: 'raisin:Conversation',
        properties: props,
        created_at: row.created_at,
        updated_at: row.updated_at,
      })
    })
  },

  /**
   * Fetch all messages of a conversation.
   * Returns raisin:Message children sorted by creation order.
   */
  getConversationMessages: async (repo: string, chatPath: string): Promise<Node[]> => {
    const children = await nodesApi.listChildrenAtHead(repo, BRANCH, WORKSPACE, chatPath)
    return children.filter(n => n.node_type === 'raisin:Message')
  },

  /**
   * Fetch all child nodes of a message (tool calls, thoughts, plans, cost records).
   */
  getMessageChildren: async (repo: string, messagePath: string): Promise<Node[]> => {
    return nodesApi.listChildrenAtHead(repo, BRANCH, WORKSPACE, messagePath)
  },

  /**
   * Build the full conversation tree: messages with nested tool calls,
   * thoughts, plans (with tasks), and cost records.
   */
  getConversationTree: async (repo: string, chatPath: string): Promise<ConversationTree> => {
    // Get the conversation node itself
    const convNode = await nodesApi.getAtHead(repo, BRANCH, WORKSPACE, chatPath)
    const messageNodes = await agentConversationsApi.getConversationMessages(repo, chatPath)

    let hasErrors = false
    const messages: ConversationMessage[] = []

    for (const msgNode of messageNodes) {
      const message = nodeToMessage(msgNode)

      // Fetch children of this message
      let msgChildren: Node[] = []
      try {
        msgChildren = await nodesApi.listChildrenAtHead(repo, BRANCH, WORKSPACE, msgNode.path)
      } catch {
        // No children or path not found
      }

      for (const child of msgChildren) {
        switch (child.node_type) {
          case 'raisin:AIToolCall': {
            const tc = nodeToToolCall(child)
            // Fetch tool call results (children of the tool call)
            try {
              const tcChildren = await nodesApi.listChildrenAtHead(repo, BRANCH, WORKSPACE, child.path)
              for (const resultNode of tcChildren) {
                if (resultNode.node_type === 'raisin:AIToolResult' || resultNode.node_type === 'raisin:AIToolSingleCallResult') {
                  tc.result = resultNode.properties?.result
                  tc.error = resultNode.properties?.error as string | undefined
                  tc.durationMs = resultNode.properties?.duration_ms as number | undefined
                  if (tc.error) hasErrors = true
                }
              }
            } catch {
              // No children
            }
            message.toolCalls.push(tc)
            break
          }
          case 'raisin:AIThought':
            message.thoughts.push(nodeToThought(child))
            break
          case 'raisin:AIPlan': {
            const plan: PlanTrace = {
              path: child.path,
              name: child.name,
              title: child.properties?.title as string | undefined,
              status: child.properties?.status as string | undefined,
              tasks: [],
            }
            // Fetch tasks (children of the plan)
            try {
              const planChildren = await nodesApi.listChildrenAtHead(repo, BRANCH, WORKSPACE, child.path)
              plan.tasks = planChildren
                .filter(t => t.node_type === 'raisin:AITask')
                .map(nodeToPlanTask)
            } catch {
              // No children
            }
            message.plans.push(plan)
            break
          }
          case 'raisin:AICostRecord':
            message.costRecords.push(nodeToCostRecord(child))
            break
        }
      }

      if (message.finishReason === 'error') hasErrors = true
      messages.push(message)
    }

    const conversation = nodeToConversation(convNode, messages.length, hasErrors)
    return { conversation, messages }
  },
}
