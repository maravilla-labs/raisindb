/**
 * The person's copy of a run's TOOL STEPS.
 *
 * The engine keeps each tool call under the agent's own message
 * (`raisin:AIToolCall` + `raisin:AIToolResult`), a tree the chatting person's
 * row-level security hides. So after a run ends or the page reloads there is
 * nothing a client can read back to show "Read the agenda ✓". This module
 * builds the one small `ai_tool_call` message a FINISHED call leaves in the
 * person's conversation, the same way the `ai_plan` card gets there: through
 * the agent's outbox (the messaging trigger mirrors it, with a stable name
 * derived from the dedupe key, so a replay never writes it twice).
 *
 * Bounded on purpose: the copy is a display record, not the audit trail
 * (that stays on the agent side). Arguments are truncated, the result is a
 * short summary, and values under secret-looking keys are masked.
 */

import { planOpOf, toolCallMirrorSlug, safeName } from './run-names.js';
import { sendAgentOutboxMessage } from './outbox.js';
import { log } from './logger.js';

export const SUMMARY_MAX_CHARS = 500;
export const ARGS_MAX_CHARS = 4000;
const STRING_MAX_CHARS = 300;
const ARRAY_MAX_ITEMS = 20;
const OBJECT_MAX_KEYS = 40;
const MAX_DEPTH = 5;
const SECRET_KEY = /(pass(word|phrase)?|secret|token|api[_-]?key|authorization|credential|private[_-]?key|cookie)/i;
const REFUSALS = ['declined', 'expired'];

/** Calls the person already sees another way (plan card, question card), or that are engine-internal. */
export function isMirroredCall(name) {
  const text = String(name || '');
  if (!text) return false;
  if (planOpOf(text)) return false;
  if (text.includes('agent-run-project')) return false;
  return true;
}

/** A value cut down to display size: long strings, wide objects and deep nesting are shortened; secret-keyed values masked. */
export function boundValue(value, depth = 0) {
  if (value === null || value === undefined) return value === undefined ? null : null;
  if (typeof value === 'string') {
    return value.length > STRING_MAX_CHARS ? `${value.slice(0, STRING_MAX_CHARS)}… (+${value.length - STRING_MAX_CHARS} chars)` : value;
  }
  if (typeof value !== 'object') return value;
  if (depth >= MAX_DEPTH) return '…';
  if (Array.isArray(value)) {
    const items = value.slice(0, ARRAY_MAX_ITEMS).map((v) => boundValue(v, depth + 1));
    if (value.length > ARRAY_MAX_ITEMS) items.push(`… (+${value.length - ARRAY_MAX_ITEMS} more)`);
    return items;
  }
  const out = {};
  const keys = Object.keys(value);
  for (const k of keys.slice(0, OBJECT_MAX_KEYS)) {
    out[k] = SECRET_KEY.test(k) ? '[hidden]' : boundValue(value[k], depth + 1);
  }
  if (keys.length > OBJECT_MAX_KEYS) out._truncated = `+${keys.length - OBJECT_MAX_KEYS} more keys`;
  return out;
}

/** Arguments bounded to {@link ARGS_MAX_CHARS} of JSON; past that only a preview survives. */
export function boundArguments(args) {
  const base = args && typeof args === 'object' && !Array.isArray(args) ? args : {};
  const bounded = boundValue(base);
  let json = '';
  try { json = JSON.stringify(bounded); } catch (_) { json = ''; }
  if (json.length <= ARGS_MAX_CHARS) return bounded;
  return { truncated: true, preview: json.slice(0, ARGS_MAX_CHARS / 2) };
}

const clip = (text) => {
  const t = String(text);
  return t.length > SUMMARY_MAX_CHARS ? `${t.slice(0, SUMMARY_MAX_CHARS - 1)}…` : t;
};

/** `completed` | `failed` | `declined` | `expired` | `stopped` for a tool answer. */
export function mirrorStatus(content) {
  if (content && typeof content === 'object') {
    for (const v of [content.code, content.error]) if (REFUSALS.includes(v)) return v;
    if (content.code === 'stopped') return 'stopped';
    if (content.code === 'cancelled' || content.error === 'cancelled') return 'declined';
    if (content.status === 'failed' || content.status === 'refused' || content.status === 'blocked'
      || content.status === 'not_run' || content.success === false || content.ok === false) return 'failed';
  }
  return 'completed';
}

/** A short, plain statement of what the call answered; never the raw payload. */
export function resultSummary(content) {
  if (content === null || content === undefined) return '';
  if (typeof content === 'string') return clip(content);
  if (typeof content !== 'object') return clip(content);
  for (const k of ['error', 'message', 'summary', 'status_statement']) {
    if (typeof content[k] === 'string' && content[k].trim()) return clip(content[k].trim());
  }
  const inner = content.content !== undefined ? content.content : content;
  if (Array.isArray(inner)) return `${inner.length} item${inner.length === 1 ? '' : 's'}`;
  try { return clip(JSON.stringify(boundValue(inner))); } catch (_) { return ''; }
}

/** The `data` of the person's `ai_tool_call` message. */
export function toolCallData(call, tr, runId) {
  const content = tr.content;
  return {
    tool_call_id: tr.call_id,
    tool_name: call.name,
    function_name: call.name,
    arguments: boundArguments(call.args),
    status: mirrorStatus(content),
    result_summary: resultSummary(content),
    run_id: runId,
  };
}

/** An ISO timestamp the call is known to have finished at (the result's own), else null. */
export function finishedAtOf(tr) {
  for (const v of [tr && tr.finished_at, tr && tr.completed_at]) {
    if (typeof v === 'string' && !Number.isNaN(Date.parse(v))) return new Date(v).toISOString();
  }
  return null;
}

/**
 * Deliver one finished call to the person. Idempotent: the outbox name is a
 * function of the run and the call id, so a re-dispatched operation is a no-op.
 * The message is stamped with when the call finished when that is known, else
 * with now (+ `seq` ms, so one batch keeps its order). Never throws: a display
 * copy must not stop a run.
 */
export async function mirrorToolCall(ctx, outboxCtx, call, tr, seq = 0) {
  if (!outboxCtx || !isMirroredCall(call.name)) return false;
  try {
    const data = toolCallData(call, tr, ctx.runId);
    await sendAgentOutboxMessage(ctx.workspace, outboxCtx, call.name, 'ai_tool_call', data, {
      slug: toolCallMirrorSlug(ctx.runId, tr.call_id),
      created_at: finishedAtOf(tr) || new Date(Date.now() + seq).toISOString(),
    });
    return true;
  } catch (err) {
    log.warn('run-tool-mirror', 'Could not mirror a tool call', { call: tr.call_id, error: String(err && err.message) });
    return false;
  }
}

/**
 * A stop (the Stop button) ends the run through the runtime, so the reducer
 * never gets to project the last turn's answers. The calls of that turn with no
 * recorded answer are shown as stopped steps instead, so the person's history
 * still lists what the agent was doing. Same names as the live mirror: a step
 * that is mirrored later (or earlier) is never written twice.
 */
export async function mirrorUnrecordedCalls(ctx, outboxCtx, lastTurnMsg) {
  if (!outboxCtx || !lastTurnMsg) return 0;
  const calls = Array.isArray(lastTurnMsg.properties && lastTurnMsg.properties.run_tool_calls)
    ? lastTurnMsg.properties.run_tool_calls : [];
  let n = 0;
  for (const call of calls) {
    if (!call || !call.call_id) continue;
    const recorded = await raisin.nodes.get(ctx.workspace, `${lastTurnMsg.path}/tool-call-${safeName(call.call_id)}`);
    if (recorded) continue;
    const tr = { call_id: call.call_id, content: { code: 'stopped', message: 'Stopped before this step finished.' } };
    if (await mirrorToolCall(ctx, outboxCtx, call, tr, n)) n += 1;
  }
  return n;
}
