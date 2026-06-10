// SPDX-License-Identifier: BSL-1.1

/**
 * Plan support for the agent Test Chat.
 *
 * Mirrors the JS SDK's plan handling (`@raisindb/client`):
 * - `loadPlans` projects plan/task state from the persisted `ai_plan` /
 *   `ai_task_update` message cards in the conversation (same algorithm as
 *   `packages/raisin-client-js/src/utils/plan-projection.ts`).
 * - `approvePlan` / `rejectPlan` invoke `/lib/raisin/ai/plan-approval-handler`
 *   via SQL INVOKE, exactly like `ConversationStore.approvePlan/rejectPlan`.
 */

import { sqlApi } from './sql'

const CHAT_WORKSPACE = 'raisin:access_control'

export interface PlanTask {
  taskId?: string
  title: string
  status: string
  description?: string
  priority?: string
}

export interface PlanProjection {
  key: string
  planPath?: string
  planId?: string
  title: string
  status: string
  requiresApproval: boolean
  tasks: PlanTask[]
  updatedAt?: string
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  return value as Record<string, unknown>
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

function parsePlanTasks(rawTasks: unknown): PlanTask[] {
  if (!Array.isArray(rawTasks)) return []
  return rawTasks
    .map((rawTask, index): PlanTask | null => {
      if (typeof rawTask === 'string') return { title: rawTask, status: 'pending' }
      const task = asRecord(rawTask)
      if (!task) return null
      return {
        taskId: asString(task.task_id) ?? asString(task.id),
        title: asString(task.title) ?? `Task ${index + 1}`,
        status: asString(task.status) ?? 'pending',
        description: asString(task.description),
        priority: asString(task.priority),
      }
    })
    .filter((task): task is PlanTask => task !== null)
}

function upsertTask(plan: PlanProjection, nextTask: PlanTask): void {
  const existingIndex = plan.tasks.findIndex(task =>
    (nextTask.taskId && task.taskId === nextTask.taskId) ||
    (!nextTask.taskId && task.title === nextTask.title),
  )
  if (existingIndex === -1) {
    plan.tasks.push(nextTask)
    return
  }
  plan.tasks[existingIndex] = { ...plan.tasks[existingIndex], ...nextTask }
}

function ensurePlan(
  plans: Map<string, PlanProjection>,
  key: string,
  fallbackTitle: string,
  timestamp: string | undefined,
): PlanProjection {
  const existing = plans.get(key)
  if (existing) {
    if (timestamp) existing.updatedAt = timestamp
    return existing
  }
  const created: PlanProjection = {
    key,
    title: fallbackTitle,
    status: 'pending_approval',
    requiresApproval: true,
    tasks: [],
    updatedAt: timestamp,
  }
  plans.set(key, created)
  return created
}

interface PlanCardRow {
  path: string
  created_at?: string
  properties?: {
    message_type?: string
    created_at?: string
    data?: Record<string, unknown>
    content?: string
  }
}

function projectPlans(rows: PlanCardRow[]): PlanProjection[] {
  const plans = new Map<string, PlanProjection>()

  for (const row of rows) {
    const props = row.properties ?? {}
    const data = asRecord(props.data)
    if (!data) continue
    const timestamp = asString(props.created_at) ?? asString(row.created_at)

    if (props.message_type === 'ai_plan') {
      const planPath = asString(data.plan_path)
      const planId = asString(data.plan_id)
      const key = planPath || planId || row.path
      const plan = ensurePlan(plans, key, asString(data.title) || props.content || 'Plan', timestamp)

      plan.planPath = planPath ?? plan.planPath
      plan.planId = planId ?? plan.planId
      plan.title = asString(data.title) || plan.title
      plan.status = asString(data.status) || plan.status
      plan.requiresApproval = Boolean(data.requires_approval) || plan.status === 'pending_approval'

      const parsedTasks = parsePlanTasks(data.tasks)
      if (parsedTasks.length > 0) plan.tasks = parsedTasks
    }

    if (props.message_type === 'ai_task_update') {
      const planPath = asString(data.plan_path)
      const planId = asString(data.plan_id)
      const key = planPath || planId
      if (!key) continue

      const plan = ensurePlan(plans, key, 'Plan', timestamp)
      plan.planPath = planPath ?? plan.planPath
      plan.planId = planId ?? plan.planId
      plan.status = asString(data.plan_status) || plan.status
      plan.requiresApproval = plan.status === 'pending_approval'

      upsertTask(plan, {
        taskId: asString(data.task_id),
        title: asString(data.task_title) || asString(data.title) || 'Task',
        status: asString(data.status) || 'pending',
      })
    }
  }

  // Oldest first so cards keep a stable order in the chat.
  return [...plans.values()].sort((a, b) => {
    const aTime = Date.parse(a.updatedAt || '')
    const bTime = Date.parse(b.updatedAt || '')
    return (Number.isNaN(aTime) ? 0 : aTime) - (Number.isNaN(bTime) ? 0 : bTime)
  })
}

async function invokePlanAction(
  repo: string,
  action: 'approve' | 'reject',
  planPath: string,
  feedback?: string,
): Promise<void> {
  const payload: Record<string, unknown> = {
    action,
    plan_path: planPath,
    plan_action_id: crypto.randomUUID(),
  }
  if (action === 'reject' && feedback?.trim()) payload.feedback = feedback.trim()

  const result = await sqlApi.executeQuery(
    repo,
    `SELECT INVOKE($1, $2::jsonb, $3) AS invoke`,
    ['/lib/raisin/ai/plan-approval-handler', JSON.stringify(payload), 'functions'],
  )

  const row = result?.rows?.[0] as Record<string, unknown> | undefined
  let receipt = (row?.invoke ?? row?.INVOKE ?? null) as Record<string, unknown> | string | null
  if (typeof receipt === 'string') {
    try {
      receipt = JSON.parse(receipt) as Record<string, unknown>
    } catch {
      receipt = null
    }
  }
  if (!receipt || typeof receipt !== 'object' || !receipt.job_id) {
    throw new Error(`Plan ${action} was not accepted by the backend queue`)
  }
}

export const agentChatPlansApi = {
  /**
   * Load + project the plan state for a conversation from its persisted
   * `ai_plan` / `ai_task_update` message cards.
   */
  async loadPlans(repo: string, conversationPath: string): Promise<PlanProjection[]> {
    const result = await sqlApi.executeQuery(
      repo,
      `SELECT path, properties, created_at FROM '${CHAT_WORKSPACE}'
       WHERE CHILD_OF($1) AND node_type = 'raisin:Message'
         AND properties->>'message_type'::String IN ('ai_plan', 'ai_task_update')
       ORDER BY created_at ASC`,
      [conversationPath],
    )
    return projectPlans((result?.rows ?? []) as PlanCardRow[])
  },

  /** Approve a pending plan (async; status flips via the job queue). */
  approvePlan(repo: string, planPath: string): Promise<void> {
    return invokePlanAction(repo, 'approve', planPath)
  },

  /** Reject a pending plan with optional feedback (agent proposes a revision). */
  rejectPlan(repo: string, planPath: string, feedback?: string): Promise<void> {
    return invokePlanAction(repo, 'reject', planPath, feedback)
  },
}
