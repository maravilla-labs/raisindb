/**
 * Client tools in the generic loop: a call to a page tool is dispatched to the
 * relay, answered by the page (or by the clock), and the page's tool set is
 * replaced by what it reports. See agent-shared/client-tools.js and
 * docs/webmcp-agent-contract.md.
 *
 * PURE (deterministic execution policy).
 */

import {
  CLIENT_TOOL_RELAY, CLIENT_RESUME_PREFIX, allowedClientTools, bareClientName, pageToolName,
} from '../agent-shared/client-tools.js';
import { setClient, boundedContent, contentOfClientEnvelope } from './state.js';

export const EXPIRED_MESSAGE = 'The page did not answer in time. The action may still be waiting for the person, or may already have run. Read the page again before retrying, and never repeat a create without checking.';
export const CANCELLED_MESSAGE = 'The run was stopped before this action was confirmed. Nothing was done.';
export const DECLINED_MESSAGE = 'The person declined this action. Ask them what they want instead; do not retry it.';

/** The stale-tool answer: names the page tools that exist now. */
export function staleToolMessage(state) {
  const names = allowedClientTools(state.client).slice(0, 15).map((t) => pageToolName(t.name));
  return `tool no longer available on the page; re-read the page. Page tools now: ${names.length ? names.join(', ') : 'none'}.`;
}

/**
 * A model call that names no offered function tool: when it names a PAGE tool
 * (`page.x` / `page__x`) and the agent has a client policy, answer it here.
 * Returns true when the call was handled (queued or answered), false when it
 * is not a page call at all (the caller refuses it as an unknown tool).
 */
export function clientCall(state, call, pushResult) {
  const bare = bareClientName(call.name);
  if (bare === null || !state.client || !state.client.policy) return false;
  const tool = allowedClientTools(state.client).find((t) => t.name === bare);
  if (!tool) {
    // Gone since the model last read the page (or never allowed): answered at
    // once, so the model re-reads the page instead of retrying.
    state.progress.refused += 1;
    pushResult({ call_id: call.call_id, synthetic: true, content: { ok: false, error: staleToolMessage(state) } });
    return true;
  }
  return { tool, bare };
}

/** The queue entry of an allowed page call (`fp` and the gate are the caller's). */
export function clientQueueEntry(state, call, found, fp) {
  const readOnly = !!(found.tool.annotations && found.tool.annotations.readOnlyHint === true);
  return {
    call_id: call.call_id,
    name: pageToolName(found.bare),
    function_path: CLIENT_TOOL_RELAY,
    args: call.args || {},
    // The relay announces the call to the page; a re-dispatch would announce it
    // AGAIN and a mutating page tool would run twice. Only a page tool that
    // declares itself read-only may be re-dispatched; a mutating one is
    // reported to the model as "outcome unknown" on recovery, which is true.
    mutating: !readOnly,
    replay_safe: readOnly,
    fp,
    client: {
      call_id: call.call_id,
      resume_key: `${CLIENT_RESUME_PREFIX}${call.call_id}`,
      name: found.bare,
      args: call.args && typeof call.args === 'object' ? call.args : {},
      client_id: state.client.client_id,
      timeout_ms: state.client.policy.timeout_ms,
    },
  };
}

/** `declined` / `expired` when the page's answer says so (also the bare `error` form), else null. */
function refusalCode(env) {
  const p = env && env.payload && typeof env.payload === 'object' ? env.payload : {};
  // The Studio page nests it: payload.content.code; the bare forms are also read.
  const inner = p.content && typeof p.content === 'object' ? p.content : {};
  for (const v of [inner.code, p.code, p.error]) if (v === 'declined' || v === 'expired' || v === 'cancelled') return v;
  return null;
}

/**
 * The page answered (`envelope.source === 'client'`): apply what it reports,
 * answer the call. A declined or expired answer is told apart for the model
 * and is NEUTRAL: it is neither a failure nor evidence, and does not feed the
 * no-progress stop.
 */
export function applyClientAnswer(state, env, awaiting) {
  if (Array.isArray(env.client_tools)) setClient(state, { tools: env.client_tools });
  if (env.client_context && typeof env.client_context === 'object') setClient(state, { context: env.client_context });
  const code = refusalCode(env);
  const ok = clientSucceeded(env);
  let content;
  if (code) {
    state.turn_stats.neutral = (state.turn_stats.neutral || 0) + 1;
    if (code === 'expired') state.expiry_streak = (state.expiry_streak || 0) + 1;
    else state.expiry_streak = 0;
    content = { ok: false, error: code, code, message: code === 'declined' ? DECLINED_MESSAGE : code === 'cancelled' ? CANCELLED_MESSAGE : EXPIRED_MESSAGE };
  } else {
    if (ok) {
      state.progress.tool_ok += 1;
      state.turn_stats.ok += 1;
      state.expiry_streak = 0;
    } else {
      state.progress.tool_failed += 1;
    }
    content = contentOfClientEnvelope(env);
    if (content.content !== undefined) content.content = boundedContent(content.content, state.cfg.max_result_chars, null);
  }
  state.results.push({ call_id: awaiting.call_id, synthetic: false, content });
  state.awaiting = null;
  return { ok, neutral: !!code };
}

/** The page tool ran AND did what it was asked: a refusal or failure arrives as `payload.ok: false` (core also sends status `failed`). A plan task's evidence. */
export function clientSucceeded(env) {
  return !!env && env.status === 'succeeded' && !(env.payload && typeof env.payload === 'object' && env.payload.ok === false);
}

/**
 * Is the closed request the one the run is waiting on? An event that names
 * the effect decides; one that names only a request is matched through the
 * run's open requests; one that names nothing is taken to be the awaited one.
 */
function closedIsAwaited(aw, ev, run) {
  const d = ev.data || {};
  const open = Array.isArray(run && run.open_requests) ? run.open_requests : [];
  const eid = ev.effect_id || d.effect_id;
  if (eid) return eid === aw.effect_id;
  const rid = d.request_id;
  if (rid) {
    const mine = open.find((r) => r && r.effect_id === aw.effect_id);
    if (mine && mine.request_id !== rid) return false;
    const other = open.find((r) => r && r.request_id === rid);
    if (other) return other.effect_id === aw.effect_id;
  }
  return true;
}

/**
 * The wait for a page tool's answer ended without one. Returns 'ignored' when
 * the closed request is not the awaited one, 'expired' / 'closed' when the
 * wait ended, false when the run is not waiting on a page tool at all.
 */
export function closeClientCall(state, ev, run) {
  const aw = state.awaiting;
  if (!aw || aw.kind !== 'tool' || !aw.client) return false;
  if (!closedIsAwaited(aw, ev || {}, run || {})) return 'ignored';
  const reason = ev && ev.data && ev.data.reason;
  if (reason === 'expired') {
    state.turn_stats.neutral = (state.turn_stats.neutral || 0) + 1;
    state.expiry_streak = (state.expiry_streak || 0) + 1;
    state.results.push({ call_id: aw.call_id, synthetic: true, content: { ok: false, error: 'expired', code: 'expired', message: EXPIRED_MESSAGE } });
    state.awaiting = null;
    return 'expired';
  }
  state.progress.tool_failed += 1;
  state.results.push({ call_id: aw.call_id, synthetic: true, content: { ok: false, error: 'client_unavailable' } });
  state.awaiting = null;
  return 'closed';
}

/**
 * A new user message while a page call is parked: the runtime withdraws the
 * request, so the call is answered here. The page action may already have run.
 */
export function interruptClientCall(state, run) {
  const aw = state.awaiting;
  if (!aw || aw.kind !== 'tool' || !aw.client) return false;
  const open = Array.isArray(run.open_requests) ? run.open_requests : [];
  if (!open.some((r) => r && r.kind === 'external' && r.effect_id === aw.effect_id)) return false;
  state.results.push({
    call_id: aw.call_id,
    synthetic: true,
    content: {
      ok: false,
      error: 'superseded',
      message: 'The user sent a new message before the page answered; the page action may or may not have run. Re-read the page (page.get_location / page.get_document) before relying on it.',
    },
  });
  state.awaiting = null;
  return true;
}
