/**
 * The plan CARD of a run: a projection of the plan held in run state.
 *
 * The card is written only by the run's projection operation. The model never
 * writes it — under a run the planning tools are answered by the reducer, and
 * a task reads `completed` here only when the runtime granted it.
 *
 * Shapes are the ones the chat UI already renders: a `raisin:AIPlan` with
 * `raisin:AITask` children under the assistant message that proposed it, and
 * an `ai_plan` outbox card mirrored into the user's conversation.
 */

import { log } from './logger.js';
import { createOrGet } from './run-transcript.js';
import { sendAgentOutboxMessage } from './outbox.js';
import { shortRun, turnMessagePath } from './run-names.js';

const CARD_STATUS = { pending_approval: 'pending_approval', active: 'in_progress', rejected: 'cancelled' };

/** The card's status for the plan's current state. */
export function cardStatus(plan) {
  if (plan.status === 'active' && plan.tasks.every((t) => !['pending', 'in_progress'].includes(t.status))) {
    return 'completed';
  }
  return CARD_STATUS[plan.status] || 'in_progress';
}

/** Where the card lives: under the message whose turn created the plan. */
export function planCardPath(chatPath, runId, plan, fallbackMsgPath) {
  const anchor = plan.anchor_op ? turnMessagePath(chatPath, runId, plan.anchor_op) : fallbackMsgPath;
  return anchor ? `${anchor}/plan-run-${shortRun(runId)}-${plan.no}` : null;
}

function taskProps(t) {
  return {
    title: t.title,
    description: t.description || '',
    status: t.status,
    priority: t.priority || 'normal',
    run_task_key: t.key,
    completion_requested: t.completion_requested === true,
    evidence_ops: t.evidence_ops || [],
  };
}

async function upsert(workspace, parentPath, name, nodeType, properties) {
  const path = `${parentPath}/${name}`;
  const existing = await raisin.nodes.get(workspace, path);
  if (!existing) return createOrGet(workspace, parentPath, { name, node_type: nodeType, properties });
  await raisin.nodes.update(workspace, path, { properties: { ...(existing.properties || {}), ...properties } });
  return { ...existing, path };
}

/** Write the card and its tasks; returns the card path, or null. */
export async function upsertPlanCard(ctx, plan, fallbackMsgPath) {
  const path = plan.card_path || planCardPath(ctx.chatPath, ctx.runId, plan, fallbackMsgPath);
  if (!path) return null;
  const parent = path.split('/').slice(0, -1).join('/');
  const anchor = await raisin.nodes.get(ctx.workspace, parent);
  if (!anchor) {
    log.warn('run-plan-card', 'Plan anchor message missing', { parent });
    return null;
  }
  const status = cardStatus(plan);
  const done = plan.tasks.filter((t) => t.status === 'completed').length;
  const card = await upsert(ctx.workspace, parent, path.split('/').pop(), 'raisin:AIPlan', {
    title: plan.title,
    description: plan.description || '',
    status,
    estimated_steps: plan.tasks.length,
    completed_steps: done,
    run_id: ctx.runId,
    run_plan_no: plan.no,
    projection_of_run: true,
    // What an approval of this card is bound to (the runtime checks it).
    approval_digest: plan.approval_digest || null,
  });
  for (let i = 0; i < plan.tasks.length; i++) {
    await upsert(ctx.workspace, path, `task-${i + 1}`, 'raisin:AITask', taskProps(plan.tasks[i]));
  }
  return card ? path : null;
}

/** The `ai_plan` card data the chat UI renders. */
export function planCardData(plan, planPath, runId) {
  const status = cardStatus(plan);
  return {
    plan_id: `plan-${plan.no}`,
    plan_path: planPath,
    run_id: runId,
    title: plan.title,
    description: plan.description || '',
    tasks: plan.tasks.map((t) => ({ task_id: t.key, title: t.title, status: t.status, completion_requested: t.completion_requested === true })),
    status,
    requires_approval: status === 'pending_approval',
    approval_digest: plan.approval_digest || null,
  };
}

async function updateCards(workspace, chatPath, planPath, data) {
  const rows = await raisin.sql.query(
    `SELECT path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:Message' AND properties->>'message_type' = 'ai_plan'`,
    [chatPath],
  );
  let n = 0;
  for (const row of Array.isArray(rows) ? rows : []) {
    const props = row.properties || {};
    if (!props.data || props.data.plan_path !== planPath) continue;
    await raisin.nodes.update(workspace, row.path, { properties: { ...props, data } });
    n += 1;
  }
  return n;
}

/**
 * Deliver the card to the user once per plan, and keep every copy (agent
 * side and the user's mirrored conversation) in step with the projection.
 */
export async function syncPlanCards(ctx, outboxCtx, plan, planPath) {
  const data = planCardData(plan, planPath, ctx.runId);
  let delivered = 0;
  try {
    delivered += await updateCards(ctx.workspace, ctx.chatPath, planPath, data);
    const humanPath = ctx.chat && ctx.chat.properties && ctx.chat.properties.human_sender_path;
    if (ctx.workspace === 'ai' && humanPath) {
      const mirror = `${humanPath}/inbox/chats/${ctx.chatPath.split('/').pop()}`;
      delivered += await updateCards('raisin:access_control', mirror, planPath, data);
    }
  } catch (err) {
    log.warn('run-plan-card', 'Plan card sync failed', { error: String(err && err.message) });
  }
  // First delivery only: a card the user already has is updated in place.
  if (outboxCtx && delivered === 0) {
    await sendAgentOutboxMessage(ctx.workspace, outboxCtx, plan.title || 'Plan', 'ai_plan', data, {
      dedupe_key: `ai_plan:run:${planPath}`,
    });
  }
  return data;
}

/**
 * The newest projected plan of a conversation that still has open tasks —
 * carried into the next run so a plan survives across user turns.
 */
export async function latestOpenPlan(workspace, chatPath) {
  const rows = await raisin.sql.query(
    `SELECT path, properties FROM "${workspace}"
     WHERE DESCENDANT_OF($1) AND node_type = 'raisin:AIPlan'
     ORDER BY created_at DESC LIMIT 1`,
    [chatPath],
  );
  const card = Array.isArray(rows) ? rows[0] : null;
  if (!card || !card.properties || card.properties.projection_of_run !== true) return null;
  if (['completed', 'cancelled'].includes(card.properties.status)) return null;
  const tasks = await raisin.sql.query(
    `SELECT name, properties FROM "${workspace}"
     WHERE CHILD_OF($1) AND node_type = 'raisin:AITask'`,
    [card.path],
  );
  const ordered = (Array.isArray(tasks) ? tasks : [])
    .map((t) => ({ n: Number(String(t.name || '').replace('task-', '')) || 0, p: t.properties || {} }))
    .sort((a, b) => a.n - b.n)
    .map(({ p }) => ({ key: p.run_task_key, title: p.title, description: p.description, priority: p.priority, status: p.status }));
  if (!ordered.some((t) => t.status === 'pending' || t.status === 'in_progress')) return null;
  return {
    no: Number(card.properties.run_plan_no) || 1,
    title: card.properties.title,
    description: card.properties.description,
    status: card.properties.status === 'pending_approval' ? 'pending_approval' : 'active',
    card_path: card.path,
    tasks: ordered,
  };
}
