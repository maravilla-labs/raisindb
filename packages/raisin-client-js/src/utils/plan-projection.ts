/**
 * Plan projection utility.
 *
 * Builds deterministic plan/task state from persisted chat messages.
 */

import type { ChatMessage } from '../types/chat';

export interface PlanProjectionTask {
  taskId?: string;
  title: string;
  status: string;
  description?: string;
  priority?: string;
}

export interface PlanProjection {
  key: string;
  planPath?: string;
  planId?: string;
  title: string;
  status: string;
  requiresApproval: boolean;
  tasks: PlanProjectionTask[];
  sourceMessagePath?: string;
  updatedAt?: string;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  return value as Record<string, unknown>;
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined;
}

export function parsePlanTasks(rawTasks: unknown): PlanProjectionTask[] {
  if (!Array.isArray(rawTasks)) return [];
  return rawTasks
    .map((rawTask, index) => {
      if (typeof rawTask === 'string') {
        return {
          title: rawTask,
          status: 'pending',
        } satisfies PlanProjectionTask;
      }
      const task = asRecord(rawTask);
      if (!task) return null;
      return {
        taskId: asString(task.task_id) ?? asString(task.id),
        title: asString(task.title) ?? `Task ${index + 1}`,
        status: asString(task.status) ?? 'pending',
        description: asString(task.description),
        priority: asString(task.priority),
      } satisfies PlanProjectionTask;
    })
    .filter((task): task is PlanProjectionTask => task !== null);
}

const TERMINAL_TASK_STATUSES = new Set(['completed', 'cancelled']);
const TERMINAL_PLAN_STATUSES = new Set(['completed', 'failed', 'cancelled']);

/**
 * Same race as upsertTask: an ai_task_update message carrying a stale
 * plan_status (e.g. "in_progress", computed before the final task's
 * completion count caught up) can be persisted and thus replayed here after
 * the message that actually reflects the terminal plan status. Guard it the
 * same way - never regress out of a terminal plan status.
 */
function setPlanStatus(plan: PlanProjection, nextStatus: string | undefined): void {
  if (!nextStatus) return;
  if (TERMINAL_PLAN_STATUSES.has(plan.status) && !TERMINAL_PLAN_STATUSES.has(nextStatus)) {
    return;
  }
  plan.status = nextStatus;
}

export function upsertTask(plan: PlanProjection, nextTask: PlanProjectionTask): void {
  const existingIndex = plan.tasks.findIndex((task) =>
    (nextTask.taskId && task.taskId === nextTask.taskId) ||
    (!nextTask.taskId && task.title === nextTask.title)
  );
  if (existingIndex === -1) {
    plan.tasks.push(nextTask);
    return;
  }
  const existing = plan.tasks[existingIndex];
  // Mirrors the server-side monotonic guard in update-task/index.js: tool
  // calls from one model turn can execute in parallel, so an ai_task_update
  // message for an earlier "in_progress" transition can be persisted (and
  // thus replayed here) after the "completed" message for the same task.
  // Never let that regress an already-terminal status.
  if (TERMINAL_TASK_STATUSES.has(existing.status) && !TERMINAL_TASK_STATUSES.has(nextTask.status)) {
    return;
  }
  plan.tasks[existingIndex] = {
    ...existing,
    ...nextTask,
  };
}

export function ensurePlan(
  plans: Map<string, PlanProjection>,
  key: string,
  fallbackTitle: string,
  timestamp: string | undefined,
): PlanProjection {
  const existing = plans.get(key);
  if (existing) {
    if (timestamp) existing.updatedAt = timestamp;
    return existing;
  }
  const created: PlanProjection = {
    key,
    title: fallbackTitle,
    status: 'pending_approval',
    requiresApproval: true,
    tasks: [],
    updatedAt: timestamp,
  };
  plans.set(key, created);
  return created;
}

/**
 * Build deterministic plan/task state from persisted chat messages.
 * Uses ai_plan as canonical plan source and ai_task_update for incremental task status changes.
 */
export function projectPlansFromMessages(messages: ChatMessage[]): PlanProjection[] {
  const ordered = [...messages].sort((a, b) => {
    const aTime = Date.parse(a.timestamp || '');
    const bTime = Date.parse(b.timestamp || '');
    return (Number.isNaN(aTime) ? 0 : aTime) - (Number.isNaN(bTime) ? 0 : bTime);
  });

  const plans = new Map<string, PlanProjection>();

  for (const message of ordered) {
    const timestamp = message.timestamp;
    const data = asRecord(message.data);

    if (message.messageType === 'ai_plan' && data) {
      const planPath = asString(data.plan_path);
      const planId = asString(data.plan_id);
      const key = planPath || planId || message.path || message.id || `plan:${timestamp}`;
      const plan = ensurePlan(
        plans,
        key,
        asString(data.title) || message.content || 'Plan',
        timestamp
      );

      plan.planPath = planPath ?? plan.planPath;
      plan.planId = planId ?? plan.planId;
      plan.title = asString(data.title) || plan.title;
      setPlanStatus(plan, asString(data.status));
      plan.requiresApproval = Boolean(data.requires_approval) || plan.status === 'pending_approval';
      plan.sourceMessagePath = message.path ?? plan.sourceMessagePath;

      const parsedTasks = parsePlanTasks(data.tasks);
      if (parsedTasks.length > 0) {
        plan.tasks = parsedTasks;
      }
    }

    if (message.messageType === 'ai_task_update' && data) {
      const planPath = asString(data.plan_path);
      const planId = asString(data.plan_id);
      const key = planPath || planId;
      if (!key) continue;

      const plan = ensurePlan(plans, key, 'Plan', timestamp);
      plan.planPath = planPath ?? plan.planPath;
      plan.planId = planId ?? plan.planId;
      setPlanStatus(plan, asString(data.plan_status));
      plan.requiresApproval = plan.status === 'pending_approval';

      const taskUpdate: PlanProjectionTask = {
        taskId: asString(data.task_id),
        title: asString(data.task_title) || asString(data.title) || 'Task',
        status: asString(data.status) || 'pending',
      };
      upsertTask(plan, taskUpdate);
    }

    if (Array.isArray(message.children)) {
      for (const child of message.children) {
        if (child.type !== 'plan') continue;
        const key = child.path || message.path || message.id || `plan-child:${child.id}:${timestamp}`;
        const plan = ensurePlan(plans, key, child.planTitle || message.content || 'Plan', timestamp);
        plan.title = child.planTitle || plan.title;
        plan.status = child.status || plan.status;
        plan.requiresApproval = plan.status === 'pending_approval';
        if (Array.isArray(child.tasks) && child.tasks.length > 0) {
          plan.tasks = child.tasks.map((task) => ({
            taskId: task.id,
            title: task.title,
            status: task.status,
          }));
        }
      }
    }
  }

  // Belt-and-suspenders: if every task landed in a terminal state but the
  // plan's own status field is stuck non-terminal (the same message-race
  // that setPlanStatus guards against can also leave it simply never
  // updated), derive completion from the tasks - mirrors the server's
  // auto-complete check in update-task/index.js's updatePlanProgress.
  for (const plan of plans.values()) {
    if (
      !TERMINAL_PLAN_STATUSES.has(plan.status) &&
      plan.tasks.length > 0 &&
      plan.tasks.every((task) => TERMINAL_TASK_STATUSES.has(task.status))
    ) {
      plan.status = 'completed';
      plan.requiresApproval = false;
    }
  }

  return [...plans.values()].sort((a, b) => {
    const aTime = Date.parse(a.updatedAt || '');
    const bTime = Date.parse(b.updatedAt || '');
    return (Number.isNaN(bTime) ? 0 : bTime) - (Number.isNaN(aTime) ? 0 : aTime);
  });
}
