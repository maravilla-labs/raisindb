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

export const STALE_TOOL = 'tool no longer available on the page — re-read with page.get_location / page.get_document';

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
    pushResult({ call_id: call.call_id, synthetic: true, content: { ok: false, error: STALE_TOOL } });
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

/** The page answered (`envelope.source === 'client'`): apply what it reports, answer the call. */
export function applyClientAnswer(state, env, awaiting) {
  if (Array.isArray(env.client_tools)) setClient(state, { tools: env.client_tools });
  if (env.client_context && typeof env.client_context === 'object') setClient(state, { context: env.client_context });
  const ok = env.status === 'succeeded';
  if (ok) {
    state.progress.tool_ok += 1;
    state.turn_stats.ok += 1;
  } else {
    state.progress.tool_failed += 1;
  }
  const content = contentOfClientEnvelope(env);
  if (content.content !== undefined) content.content = boundedContent(content.content, state.cfg.max_result_chars, null);
  state.results.push({ call_id: awaiting.call_id, synthetic: false, content });
  state.awaiting = null;
  return ok;
}

/** The wait for a page tool's answer ended without one (expired, or closed): the page is unavailable. */
export function closeClientCall(state) {
  const aw = state.awaiting;
  if (!aw || aw.kind !== 'tool' || !aw.client) return false;
  state.progress.tool_failed += 1;
  state.results.push({ call_id: aw.call_id, synthetic: true, content: { ok: false, error: 'client_unavailable' } });
  state.awaiting = null;
  return true;
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
