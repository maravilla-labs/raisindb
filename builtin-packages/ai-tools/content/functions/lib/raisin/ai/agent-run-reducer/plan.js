/**
 * The plan as RUN STATE, and the plan card as its projection.
 *
 * The planning tools (`create-plan`, `add-task`, `update-task`,
 * `get-plan-status`) are DOMAIN tools under a run: core never executes them,
 * their calls come back to this reducer, which answers them itself.
 *
 * The model REQUESTS completion; it never writes it. `update-task` with
 * `status: completed` is accepted only when the task holds evidence — a
 * successful tool operation made while the task was in progress, and no
 * failure after it. Otherwise the request is recorded, the task stays open,
 * and the model is told what is missing.
 *
 * PURE (deterministic execution policy).
 */

import { DIGEST_ALG, digestOf, planOpOf } from '../agent-shared/run-names.js';

const MAX_TASKS = 50;
const MAX_EVIDENCE = 10;
const OPEN = new Set(['pending', 'in_progress']);

export { planOpOf };

/** Argument schemas the reducer offers for its domain plan tools. */
export const PLAN_SCHEMAS = Object.freeze({
  'plan.create': {
    type: 'object',
    required: ['title', 'tasks'],
    properties: {
      title: { type: 'string', description: 'Plan objective' },
      description: { type: 'string' },
      tasks: {
        type: 'array',
        items: {
          type: 'object',
          required: ['title'],
          properties: {
            title: { type: 'string' },
            description: { type: 'string' },
            priority: { type: 'string', enum: ['low', 'normal', 'high', 'urgent'] },
          },
        },
      },
    },
  },
  'plan.add_task': {
    type: 'object',
    required: ['title'],
    properties: { title: { type: 'string' }, description: { type: 'string' }, priority: { type: 'string' } },
  },
  'plan.update_task': {
    type: 'object',
    required: ['status'],
    properties: {
      task_id: { type: 'string', description: 'The task key returned by create_plan, e.g. "t2"' },
      task_number: { type: 'integer' },
      status: { type: 'string', enum: ['pending', 'in_progress', 'completed', 'failed', 'cancelled'] },
      notes: { type: 'string' },
    },
  },
  'plan.status': { type: 'object', properties: {} },
});

function task(i, t) {
  return {
    key: `t${i}`,
    title: String(t.title || `Task ${i}`).slice(0, 300),
    description: typeof t.description === 'string' ? t.description.slice(0, 1000) : '',
    priority: t.priority || 'normal',
    status: 'pending',
    completion_requested: false,
    evidence: [],
    notes: null,
  };
}

/** A plan carried over from an earlier run of the same conversation. */
export function adoptPlan(input) {
  if (!input || !Array.isArray(input.tasks) || input.tasks.length === 0) return null;
  // An approval belongs to the run that asked for it; it is never carried over.
  if (input.status === 'pending_approval') return null;
  const tasks = input.tasks.slice(0, MAX_TASKS).map((t, i) => {
    const out = task(i + 1, t);
    if (typeof t.key === 'string' && t.key) out.key = t.key;
    if (['pending', 'in_progress', 'completed', 'failed', 'cancelled'].includes(t.status)) out.status = t.status;
    if (out.status === 'completed') out.evidence = [{ op: 'earlier_run', ok: true, writes: 0 }];
    return out;
  });
  return {
    no: Number(input.no) || 1,
    title: String(input.title || 'Plan').slice(0, 300),
    description: String(input.description || ''),
    status: 'active',
    anchor_op: input.anchor_op || null,
    card_path: input.card_path || null,
    approval: null,
    tasks,
  };
}

function view(plan) {
  return {
    plan_id: `plan-${plan.no}`,
    title: plan.title,
    status: plan.status,
    tasks: plan.tasks.map((t, i) => ({
      task_id: t.key, task_number: i + 1, title: t.title, status: t.status,
      ...(t.completion_requested && t.status !== 'completed' ? { completion_requested: true } : {}),
    })),
  };
}

function findTask(plan, args) {
  if (!plan) return null;
  const id = args && (args.task_id ?? args.task_key ?? args.id);
  if (typeof id === 'string' && id) {
    const byKey = plan.tasks.find((t) => t.key === id);
    if (byKey) return byKey;
  }
  const n = Number(args && (args.task_number ?? (typeof id === 'number' ? id : NaN)));
  if (Number.isInteger(n) && n >= 1 && n <= plan.tasks.length) return plan.tasks[n - 1];
  const title = args && typeof args.title === 'string' ? args.title.trim().toLowerCase() : '';
  if (title) return plan.tasks.find((t) => t.title.toLowerCase() === title) || null;
  return null;
}

/** Evidence a completion request needs: a success, and no failure after it. */
export function hasEvidence(t, functionToolCount) {
  if (functionToolCount === 0) return true;
  if (!t.evidence.length) return false;
  return t.evidence[t.evidence.length - 1].ok === true;
}

/** Attribute a tool outcome to every task in progress. */
export function attributeEvidence(state, opId, ok, writes) {
  if (!state.plan) return;
  for (const t of state.plan.tasks) {
    if (t.status !== 'in_progress') continue;
    t.evidence.push({ op: opId, ok, writes });
    if (t.evidence.length > MAX_EVIDENCE) t.evidence.shift();
  }
  // Evidence alone does not re-project the card: its statuses did not move.
}

/**
 * Apply one domain plan call. Returns `{ content, approval? }` — the synthetic
 * answer the model reads, and whether the plan now awaits approval.
 */
export function applyPlanCall(state, op, args, callId, opId) {
  const fnTools = state.tools.filter((t) => t.kind === 'function').length;
  const a = args && typeof args === 'object' ? args : {};
  if (op === 'plan.create') {
    if (!a.title || !Array.isArray(a.tasks) || a.tasks.length === 0) {
      return { content: { success: false, error: 'create_plan needs a title and at least one task' } };
    }
    state.plan_seq += 1;
    state.plan = {
      no: state.plan_seq,
      title: String(a.title).slice(0, 300),
      description: typeof a.description === 'string' ? a.description.slice(0, 2000) : '',
      status: state.cfg.requires_approval ? 'pending_approval' : 'active',
      anchor_op: opId,
      card_path: null,
      approval: null,
      tasks: a.tasks.slice(0, MAX_TASKS).map((t, i) => task(i + 1, t || {})),
    };
    state.plan_dirty = true;
    if (state.plan.status === 'pending_approval') {
      state.plan.approval = { call_id: callId, digest: planDigest(state.plan) };
      return { content: null, approval: true };
    }
    return {
      content: {
        success: true, ...view(state.plan),
        message: `Created plan "${state.plan.title}". Start with ${state.plan.tasks[0].key}: mark it in_progress, do the work with your tools, then request completion.`,
      },
    };
  }
  if (!state.plan) return { content: { success: false, error: 'No plan exists. Use create_plan first.' } };
  if (op === 'plan.status') return { content: { success: true, ...view(state.plan) } };
  if (op === 'plan.add_task') {
    if (!a.title) return { content: { success: false, error: 'add_task needs a title' } };
    if (state.plan.tasks.length >= MAX_TASKS) return { content: { success: false, error: `a plan holds at most ${MAX_TASKS} tasks` } };
    const t = task(state.plan.tasks.length + 1, a);
    state.plan.tasks.push(t);
    state.plan_dirty = true;
    return { content: { success: true, task_id: t.key, task_number: state.plan.tasks.length, title: t.title, status: t.status } };
  }
  if (op === 'plan.update_task') {
    if (state.plan.status === 'pending_approval') {
      return { content: { success: false, error: 'The plan is waiting for approval; no task can change yet.' } };
    }
    const t = findTask(state.plan, a);
    if (!t) return { content: { success: false, error: 'No such task. Use the task_id from create_plan (e.g. "t1").', tasks: view(state.plan).tasks } };
    const status = String(a.status || '');
    if (typeof a.notes === 'string') t.notes = a.notes.slice(0, 1000);
    if (status === 'completed') {
      t.completion_requested = true;
      state.plan_dirty = true;
      if (t.status === 'completed') return { content: { success: true, task_id: t.key, status: 'completed', message: 'Already completed.' } };
      if (!hasEvidence(t, fnTools)) {
        return {
          content: {
            success: false,
            task_id: t.key,
            status: t.status,
            refused: 'no_evidence',
            message: t.evidence.length
              ? 'Not completed: the last tool operation for this task failed. Fix it and try again before requesting completion.'
              : 'Not completed: no tool operation has succeeded while this task was in progress. Mark it in_progress, do the work with your tools, then request completion.',
          },
        };
      }
      t.status = 'completed';
      return { content: { success: true, task_id: t.key, status: 'completed', ...nextHint(state.plan) }, completed: true };
    }
    if (!['pending', 'in_progress', 'failed', 'cancelled'].includes(status)) {
      return { content: { success: false, error: `Unknown status "${status}"` } };
    }
    t.status = status;
    state.plan_dirty = true;
    return { content: { success: true, task_id: t.key, status } };
  }
  return { content: { success: false, error: `Unknown plan operation ${op}` } };
}

function nextHint(plan) {
  const next = plan.tasks.find((t) => OPEN.has(t.status));
  return next
    ? { next_task: next.key, message: `Next: ${next.key} "${next.title}".` }
    : { message: 'Every task is closed. Give the user a final summary.' };
}

/** Digest the approval is bound to: the plan the user saw. */
export function planDigest(plan) {
  return digestOf({ title: plan.title, tasks: plan.tasks.map((t) => ({ key: t.key, title: t.title })) });
}

/** The `request_approval` subject for a pending plan. */
export function approvalSubject(plan) {
  return {
    kind: 'plan',
    digest: plan.approval.digest,
    digest_alg: DIGEST_ALG,
    summary: `Plan "${plan.title}" with ${plan.tasks.length} task(s)`,
    changes: plan.tasks.map((t) => ({ key: t.key, title: t.title })),
  };
}

/** Plan statistics for summaries. */
export function planStats(plan) {
  if (!plan) return null;
  const done = plan.tasks.filter((t) => t.status === 'completed').length;
  const open = plan.tasks.filter((t) => OPEN.has(t.status));
  return { total: plan.tasks.length, done, open: open.length, requested_without_evidence: open.filter((t) => t.completion_requested).map((t) => t.key) };
}

function itemStatus(plan, t) {
  if (plan.status === 'pending_approval') return 'waiting';
  if (plan.status === 'rejected') return 'blocked';
  return { pending: 'pending', in_progress: 'in_progress', completed: 'completed', failed: 'failed', cancelled: 'blocked' }[t.status] || 'pending';
}

/** The generic projection (contract §C.5) of the plan. */
export function projection(state) {
  const plan = state.plan;
  if (!plan) return null;
  const s = planStats(plan);
  return {
    items: plan.tasks.map((t) => ({
      key: t.key,
      title: t.title,
      status: itemStatus(plan, t),
      detail: {
        evidence_ops: t.evidence.filter((e) => e.ok).map((e) => e.op).slice(-5),
        completion_requested: t.completion_requested,
        proof: t.status === 'completed' ? 'tool_evidence' : (t.completion_requested ? 'missing' : 'none'),
        ...(t.status === 'cancelled' ? { cancelled: true } : {}),
      },
    })),
    summary: `${plan.title}: ${s.done}/${s.total} completed`,
  };
}

/** The card the projection operation writes. */
export function planCardView(state) {
  const plan = state.plan;
  if (!plan) return null;
  return {
    no: plan.no,
    title: plan.title,
    description: plan.description,
    status: plan.status,
    anchor_op: plan.anchor_op,
    card_path: plan.card_path || null,
    approval_digest: plan.approval ? plan.approval.digest : null,
    tasks: plan.tasks.map((t) => ({
      key: t.key, title: t.title, description: t.description, priority: t.priority, status: t.status,
      completion_requested: t.completion_requested,
      evidence_ops: t.evidence.filter((e) => e.ok).map((e) => e.op).slice(-5),
    })),
  };
}
