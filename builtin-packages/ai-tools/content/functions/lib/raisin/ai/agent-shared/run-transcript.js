/**
 * The conversation TRANSCRIPT of an AgentRun.
 *
 * Messages are the user-visible record and audit trail of a run — a
 * projection written by the run's own operations, never the run state. Every
 * write here is idempotent (deterministic names, create-or-get), because a
 * replay-safe operation may be re-dispatched after a worker dies.
 *
 * Tool calls are written with a FINAL status and their result in one go, so
 * core's `raisin:AIToolCall` executor (which only runs `pending` calls) never
 * executes a run's call a second time.
 *
 * A model's arguments must never stop the transcript: the server validates
 * every reference envelope in a node it stores, so a call naming a node that
 * does not exist (measured: `{raisin:ref: /agents/x, raisin:workspace:
 * agents}`, the wrong workspace) was refused. Such a write is retried once
 * with its envelopes as plain path strings (`createOrGet`).
 */

import { log } from './logger.js';
import { resolveStreamChannel, emitConversationEvent } from './streaming.js';
import { safeName, turnMessageName, shortRun } from './run-names.js';
import { plainReferences } from './utils.js';

const nowIso = () => new Date().toISOString();

function exists(err) {
  return String((err && err.message) || err || '').includes('already exists');
}

const BAD_REFERENCE = /Referenced node not found|reference/i;

/**
 * Create a node, or return the one a previous attempt created. A write the
 * server refuses for a reference envelope is stored once more with plain
 * paths (and `references_rewritten` saying why); any other failure throws.
 */
export async function createOrGet(workspace, parentPath, spec) {
  try {
    return await raisin.nodes.create(workspace, parentPath, spec);
  } catch (err) {
    if (exists(err)) return raisin.nodes.get(workspace, `${parentPath}/${spec.name}`);
    const why = String((err && err.message) || err || '');
    if (!BAD_REFERENCE.test(why)) throw err;
    log.warn('run-transcript', 'A reference in the stored value was refused; storing plain paths', { name: spec.name, error: why });
    const plain = { ...spec, properties: { ...plainReferences(spec.properties || {}), references_rewritten: why } };
    try {
      return await raisin.nodes.create(workspace, parentPath, plain);
    } catch (err2) {
      if (exists(err2)) return raisin.nodes.get(workspace, `${parentPath}/${spec.name}`);
      throw err2;
    }
  }
}

/** The conversation a run is about, from its subject. */
export async function loadRunChat(subject) {
  const workspace = (subject && subject.workspace) || 'ai';
  const chatPath = subject && subject.path;
  if (!chatPath) throw new Error('run subject has no conversation path');
  const chat = await raisin.nodes.get(workspace, chatPath);
  if (!chat) throw new Error(`conversation not found: ${workspace}:${chatPath}`);
  return { workspace, chatPath, chat, streamChannel: resolveStreamChannel(chatPath, chat) };
}

/** The assistant message of the model turn `lastTurnOp`, else the run's latest. */
export async function findPreviousTurn(workspace, chatPath, runId, lastTurnOp) {
  if (lastTurnOp) {
    const byName = await raisin.nodes.get(workspace, `${chatPath}/${turnMessageName(runId, lastTurnOp)}`);
    if (byName) return byName;
  }
  const rows = await raisin.sql.query(`
    SELECT path, name, properties, created_at FROM '${workspace}'
    WHERE CHILD_OF($1) AND node_type = 'raisin:Message' AND properties->>'run_id' = $2
    ORDER BY created_at DESC LIMIT 1
  `, [chatPath, runId]);
  return Array.isArray(rows) && rows[0] ? rows[0] : null;
}

function resultProps(content) {
  const failed = content && typeof content === 'object'
    && (content.status === 'failed' || content.status === 'refused' || content.status === 'blocked' || content.success === false);
  const error = failed ? String(content.error || content.message || content.status) : undefined;
  return { failed, error };
}

/**
 * Write the answers to the previous turn's tool calls under its message:
 * one `raisin:AIToolCall` (final status) + one `raisin:AIToolResult` each.
 */
export async function writeToolResults(ctx, prevMsg, toolResults) {
  if (!prevMsg || !Array.isArray(toolResults) || toolResults.length === 0) return 0;
  const calls = Array.isArray(prevMsg.properties && prevMsg.properties.run_tool_calls)
    ? prevMsg.properties.run_tool_calls : [];
  let written = 0;
  for (const tr of toolResults) {
    if (!tr || !tr.call_id) continue;
    const call = calls.find((c) => c.call_id === tr.call_id) || { call_id: tr.call_id, name: 'tool', args: {} };
    const { failed, error } = resultProps(tr.content);
    const name = `tool-call-${safeName(tr.call_id)}`;
    const existing = await raisin.nodes.get(ctx.workspace, `${prevMsg.path}/${name}`);
    if (existing) continue;
    const callNode = await createOrGet(ctx.workspace, prevMsg.path, {
      name,
      node_type: 'raisin:AIToolCall',
      properties: {
        tool_call_id: tr.call_id,
        function_name: call.name,
        arguments: call.args || {},
        status: failed ? 'failed' : 'completed',
        run_id: ctx.runId,
        synthetic: tr.synthetic === true,
      },
    });
    if (!callNode || !callNode.path) continue;
    await createOrGet(ctx.workspace, callNode.path, {
      name: 'result',
      node_type: 'raisin:AIToolResult',
      properties: failed ? { result: tr.content, error } : { result: tr.content },
    });
    written += 1;
    await emitConversationEvent('conversation:tool_call_completed', {
      type: 'tool_call_completed',
      toolCallId: tr.call_id,
      functionName: call.name,
      status: failed ? 'failed' : 'completed',
      synthetic: tr.synthetic === true,
      runId: ctx.runId,
      timestamp: nowIso(),
    }, ctx.chatPath, ctx.streamChannel);
  }
  return written;
}

/**
 * Persist one model turn as the assistant message `name`, with its output
 * (so a re-dispatched turn returns it instead of calling the model again).
 */
export async function writeAssistantTurn(ctx, name, output, meta = {}) {
  const text = (output.message && output.message.text) || '';
  const node = await createOrGet(ctx.workspace, ctx.chatPath, {
    name,
    node_type: 'raisin:Message',
    properties: {
      role: 'assistant',
      body: { content: text, message_text: text },
      content: text,
      sender_id: meta.senderId || 'ai-assistant',
      sender_display_name: meta.senderName || 'AI Assistant',
      message_type: 'chat',
      status: 'delivered',
      created_at: nowIso(),
      model: output.model || null,
      finish_reason: output.finish_reason || 'stop',
      tokens: (output.usage && (output.usage.input_tokens + output.usage.output_tokens)) || 0,
      run_id: ctx.runId,
      operation_id: ctx.operationId,
      run_tool_calls: output.tool_calls,
      run_turn_output: output,
      // A run's messages never look like a legacy in-flight turn.
      dispatch_phase: 'terminal',
      turn_terminal_done_emitted: true,
      turn_terminal_outbox_sent: output.tool_calls.length > 0,
      ...(meta.parentMessagePath ? { parent_message_path: meta.parentMessagePath } : {}),
    },
  });
  await emitConversationEvent('conversation:message_saved', {
    type: 'message_saved', messagePath: node && node.path, role: 'assistant', runId: ctx.runId, timestamp: nowIso(),
  }, ctx.chatPath, ctx.streamChannel);
  for (const call of output.tool_calls) {
    await emitConversationEvent('conversation:tool_call_started', {
      type: 'tool_call_started', toolCallId: call.call_id, functionName: call.name, arguments: call.args,
      runId: ctx.runId, timestamp: nowIso(),
    }, ctx.chatPath, ctx.streamChannel);
  }
  return node;
}

/**
 * STEERING, visible: mark user messages whose steer the runtime consumed (or
 * discarded) since the last look, and tell the UI. Reads the run's durable
 * events after a cursor kept on the conversation.
 */
export async function syncSteers(ctx) {
  if (!raisin.agentRuns || typeof raisin.agentRuns.events !== 'function') return;
  const cursor = ctx.chat.properties && ctx.chat.properties.agent_run_event_cursor;
  const after = cursor && cursor.run_id === ctx.runId ? Number(cursor.seq) || 0 : 0;
  let events;
  try {
    events = await raisin.agentRuns.events({ run_id: ctx.runId, after_seq: after, limit: 1000 });
  } catch (err) {
    log.warn('run-transcript', 'Could not read run events', { error: String(err && err.message) });
    return;
  }
  if (!Array.isArray(events) || events.length === 0) return;
  for (const ev of events) {
    const kind = ev.kind || {};
    if (kind.type !== 'steer_consumed') continue;
    const messagePath = steerMessagePath(kind.input);
    if (!messagePath) continue;
    await markSteer(ctx, messagePath, 'consumed', kind.steer_id);
  }
  const last = events[events.length - 1].seq;
  await raisin.nodes.updateProperty(ctx.workspace, ctx.chatPath, 'agent_run_event_cursor', { run_id: ctx.runId, seq: last });
}

/** The transcript message a steer names: a user's, or a parent run's (`parent_message` / `parent_steer`). */
function steerMessagePath(input) {
  if (!input || typeof input !== 'object') return null;
  const inner = input.type === 'parent_message' ? input.message : input.type === 'parent_steer' ? input.input : input;
  return inner && typeof inner === 'object' && typeof inner.message_path === 'string' ? inner.message_path : null;
}

/** Set a user message's steer state and announce it. */
export async function markSteer(ctx, messagePath, state, steerId = null) {
  try {
    await raisin.nodes.updateProperty(ctx.workspace, messagePath, 'run_steer_state', state);
  } catch (err) {
    log.warn('run-transcript', 'Could not mark steer', { path: messagePath, error: String(err && err.message) });
  }
  await emitConversationEvent(`conversation:steer_${state}`, {
    type: `steer_${state}`, messagePath, steerId, runId: ctx.runId, timestamp: nowIso(),
  }, ctx.chatPath, ctx.streamChannel);
}

/** User messages of this run still queued as steers (not yet in context). */
export async function queuedSteerPaths(ctx) {
  const rows = await raisin.sql.query(`
    SELECT path, properties FROM '${ctx.workspace}'
    WHERE CHILD_OF($1) AND node_type = 'raisin:Message' AND properties->>'run_steer_state' = 'queued'
  `, [ctx.chatPath]);
  return new Set((Array.isArray(rows) ? rows : [])
    .filter((r) => r.properties && r.properties.agent_run_id === ctx.runId)
    .map((r) => r.path));
}

/** A compaction node name that is stable per model-turn operation. */
export function compactionName(runId, operationId) {
  return `compaction-run-${shortRun(runId)}-${safeName(String(operationId).split('/').pop())}`;
}
