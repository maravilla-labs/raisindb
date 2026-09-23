/**
 * Every agent conversation runs as an AgentRun.
 *
 * A user message either STARTS a run (one live run per conversation — core
 * admits one per subject) or STEERS the live one: the message is already in
 * the transcript, and the steer tells the runtime to feed it to the loop at
 * the next safe boundary. The message records which, and the UI is told —
 * queued now, consumed when a model turn reads it.
 *
 * What the run is given at create: the agent's tools as a compact list (the
 * reducer offers them; core executes them as the run's principal), the
 * execution-mode config, the conversation context tools expect, and the open
 * plan of an earlier run so a plan survives across user turns.
 */

import { log } from './logger.js';
import { emitConversationEvent } from './streaming.js';
import { resolveRunTools } from './run-tools.js';
import { latestOpenPlan } from './run-plan-card.js';
import { DEFAULT_RUN_REDUCER, DEFAULT_MODEL_TURN_FUNCTION } from './run-names.js';
import { getEffectiveExecutionMode, requiresPlanApproval } from './utils.js';
import { runIdOfChat, getRun, isTerminal, controlRun, commandFor } from './run-control.js';

const nowIso = () => new Date().toISOString();

/** Default budgets: generous, and a pause (not a failure) when exceeded. */
export const DEFAULT_RUN_BUDGETS = Object.freeze({
  max_model_calls: 60,
  max_operations: 300,
  max_consecutive_op_failures: 8,
  max_wall_ms: 60 * 60 * 1000,
  on_exceeded: 'pause',
});

function messageText(message) {
  const p = (message && message.properties) || {};
  if (typeof p.content === 'string' && p.content.trim()) return p.content.trim();
  if (typeof p.body === 'string') return p.body.trim();
  if (p.body && typeof p.body === 'object') return String(p.body.content || p.body.message_text || '').trim();
  return '';
}

/**
 * The IDENTITY a sender id names. Messages carry the sender's user NODE (its id
 * or home path); a run's principal acts for an identity (the JWT `sub`), which
 * is what core resolves permissions for and what the person's own API calls
 * authenticate as — so a person can read and control the run made for them.
 */
export async function identityOf(senderId) {
  const id = typeof senderId === 'string' ? senderId.trim() : '';
  if (!id) return null;
  try {
    const byPath = id.startsWith('/');
    const res = await raisin.sql.query(
      `SELECT properties->>'user_id' AS uid FROM 'raisin:access_control' WHERE node_type = 'raisin:User' AND ${byPath ? 'path' : 'id'} = $1`,
      [id],
    );
    const rows = Array.isArray(res) ? res : (res && res.rows) || [];
    const uid = rows[0] && rows[0].uid;
    return typeof uid === 'string' && uid ? uid : id;
  } catch (_) {
    return id;
  }
}

/**
 * THE RUN'S GRANT: the agent node's `node_dev.roots` — `{workspace, path, ops}`
 * working roots. Core narrows every node-development call the run's tools
 * make by `executor_config.node_dev.roots` on the run RECORD (never by tool
 * arguments), and the reducer sees the same roots in `input.config.node_dev`
 * so it can refuse a plan that reaches outside them before anything runs.
 * `null` when the agent declares none.
 */
export function nodeDevGrant(agentProps) {
  const roots = agentProps && agentProps.node_dev && agentProps.node_dev.roots;
  if (!Array.isArray(roots)) return null;
  const clean = roots
    .filter((r) => r && typeof r === 'object' && typeof r.workspace === 'string' && r.workspace.trim())
    .map((r) => ({
      workspace: r.workspace.trim(),
      path: typeof r.path === 'string' && r.path.trim() ? r.path.trim() : '/',
      ...(Array.isArray(r.ops) ? { ops: r.ops.filter((o) => typeof o === 'string') } : {}),
    }));
  return clean.length ? { roots: clean } : null;
}

export function buildCreateRequest({ workspace, chatPath, chat, message, agentProps, agentRef, tools, plan, senderId, actingUser }) {
  const mode = getEffectiveExecutionMode(agentProps.execution_mode);
  const grant = nodeDevGrant(agentProps);
  const agentName = chatPath.split('/')[2] || null;
  return {
    subject: { workspace, path: chatPath, ...(chat && chat.id ? { node_id: chat.id } : {}) },
    input: {
      text: messageText(message),
      message_path: message.path,
      tools,
      plan: plan || null,
      config: {
        execution_mode: mode,
        requires_approval: requiresPlanApproval(mode) && tools.some((t) => t.domain_op === 'plan.create'),
        ...(agentProps.run_config && typeof agentProps.run_config === 'object' ? agentProps.run_config : {}),
        ...(grant ? { node_dev: grant } : {}),
      },
      context: { workspace, chat_path: chatPath, agent_name: agentName, sender_id: senderId || null },
    },
    reducer: { function_path: agentProps.run_reducer || DEFAULT_RUN_REDUCER },
    agent_ref: agentRef,
    as_agent: agentRef,
    ...((actingUser || senderId) ? { on_behalf_of: actingUser || senderId } : {}),
    create_key: `msg:${message.id || message.path}`,
    budgets: { ...DEFAULT_RUN_BUDGETS, ...(agentProps.run_budgets && typeof agentProps.run_budgets === 'object' ? agentProps.run_budgets : {}) },
    executor_config: {
      ...(grant ? { node_dev: grant } : {}),
      model_turn_function: agentProps.model_turn_function || DEFAULT_MODEL_TURN_FUNCTION,
      workspace,
      chat_path: chatPath,
    },
  };
}

/**
 * Stamp the live run on BOTH sides of the conversation. The person's copy is
 * what their UI reads — a plain read of their own node, no lookup through the
 * agent's side (which row-level security rightly hides from them).
 */
async function markChatRun(workspace, chatPath, chat, runId) {
  await markMessage(workspace, chatPath, { active_agent_run_id: runId });
  const humanPath = chat && chat.properties && chat.properties.human_sender_path;
  if (workspace === 'ai' && humanPath) {
    const mirror = `${humanPath}/inbox/chats/${chatPath.split('/').pop()}`;
    await markMessage('raisin:access_control', mirror, { active_agent_run_id: runId });
  }
}

async function markMessage(workspace, path, props) {
  for (const [k, v] of Object.entries(props)) {
    try {
      await raisin.nodes.updateProperty(workspace, path, k, v);
    } catch (err) {
      log.warn('run-entry', 'Could not mark message', { path, key: k, error: String(err && err.message) });
    }
  }
}

/** Budgets raised past what a paused run has used, for a user's "continue". */
function raisedBudgets(view) {
  const b = (view && view.run && view.run.budgets) || {};
  const u = (view && view.run && view.run.usage) || {};
  const plus = (limit, used, step) => (limit ? Math.max(limit, (used || 0) + step) : limit);
  return {
    ...b,
    max_model_calls: plus(b.max_model_calls, u.model_calls, DEFAULT_RUN_BUDGETS.max_model_calls),
    max_operations: plus(b.max_operations, u.operations, DEFAULT_RUN_BUDGETS.max_operations),
    max_total_tokens: b.max_total_tokens ? b.max_total_tokens * 2 : b.max_total_tokens,
    max_wall_ms: b.max_wall_ms ? b.max_wall_ms + DEFAULT_RUN_BUDGETS.max_wall_ms : b.max_wall_ms,
  };
}

/** Steer `message` into the live run; null when the run could not take it. */
async function steer(ctx, runId, view, message) {
  const key = message.id || message.path;
  const ack = await controlRun(runId, commandFor('steer', {
    input: { text: messageText(message), message_path: message.path },
  }), `steer:${key}`);
  if (!ack || ack.ack === 'rejected') return null;
  // A paused run takes the user's message as "continue": resume it.
  if (view.status === 'paused') {
    await controlRun(runId, { command: 'resume', budget_increase: raisedBudgets(view) }, `resume:${key}`).catch(() => null);
  }
  await markMessage(ctx.workspace, message.path, { agent_run_id: runId, run_steer_state: 'queued' });
  await emitConversationEvent('conversation:steer_queued', {
    type: 'steer_queued', messagePath: message.path, runId, timestamp: nowIso(),
  }, ctx.chatPath, ctx.streamChannel);
  log.info('run-entry', 'Steered into the live run', { run: runId, message: message.path });
  return { mode: 'steered', run_id: runId, ack };
}

/**
 * Route one inbound user message to the conversation's run.
 * Returns `{ mode: 'created' | 'steered' | 'duplicate', run_id }`, or null when
 * the server cannot run it (no runtime, or the reducer is not installed); the
 * caller then tells the user so.
 */
export async function routeToAgentRun({
  workspace, chatPath, chat, message, agentProps, agentPath, agentWorkspace, outboxCtx, streamChannel, hasSkills,
}) {
  const ctx = { workspace, chatPath, streamChannel };
  const createKey = `msg:${message.id || message.path}`;
  const liveId = runIdOfChat(chat);
  if (liveId) {
    const view = await getRun(liveId);
    if (view && !isTerminal(view)) {
      if (view.run && view.run.create_key === createKey) return { mode: 'duplicate', run_id: liveId };
      const steered = await steer(ctx, liveId, view, message);
      if (steered) return steered;
    }
  }

  const { tools } = await resolveRunTools(agentProps, { hasSkills });
  const plan = await latestOpenPlan(workspace, chatPath).catch(() => null);
  const senderId = (outboxCtx && outboxCtx.senderId)
    || (chat && chat.properties && chat.properties.human_sender_id)
    || (message.properties && message.properties.sender_id) || null;
  const agentRef = `${agentWorkspace}:${agentPath}`;
  const actingUser = await identityOf(senderId);
  const req = buildCreateRequest({ workspace, chatPath, chat, message, agentProps, agentRef, tools, plan, senderId, actingUser });
  let created;
  try {
    created = await raisin.agentRuns.create(req);
  } catch (err) {
    const text = String(err && err.message);
    if (/not running on this server/i.test(text)) return null;
    // The reducer is not installed: this conversation cannot run.
    if (/reducer_unavailable/i.test(text)) {
      log.error('run-entry', 'Run reducer unavailable', { reducer: req.reducer.function_path, error: text });
      return null;
    }
    throw err;
  }
  // No run came back: this server does not serve runs.
  if (!created || typeof created.run_id !== 'string') return null;
  const runId = created.run_id;

  if (!created.created) {
    // Either a redelivered trigger (this message's own run) or a live run the
    // chat node did not know about yet: steer into the latter.
    const view = await getRun(runId);
    if (view && view.run && view.run.create_key === createKey) return { mode: 'duplicate', run_id: runId };
    if (view && !isTerminal(view)) {
      const steered = await steer(ctx, runId, view, message);
      if (steered) {
        await markChatRun(workspace, chatPath, chat, runId);
        return steered;
      }
    }
  }

  await markChatRun(workspace, chatPath, chat, runId);
  await markMessage(workspace, message.path, { agent_run_id: runId });
  await emitConversationEvent('conversation:run_started', {
    type: 'run_started', runId, messagePath: message.path, status: created.status, timestamp: nowIso(),
  }, chatPath, streamChannel);
  log.info('run-entry', 'Started agent run', { run: runId, reducer: req.reducer.function_path, tools: tools.length });
  return { mode: 'created', run_id: runId };
}
