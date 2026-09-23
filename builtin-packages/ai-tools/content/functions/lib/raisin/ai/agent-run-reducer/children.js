/**
 * Child runs as seen by the generic loop: which children this run started
 * (from the delegation tools' results and core's hand-backs), what the model
 * is told about them, the wait that continues until the children it names are
 * done, the interrupted wait when the user steers, and the WRITE SCOPE a
 * parent granted a child.
 *
 * Core owns the lineage itself: it delivers each hand-back into the waiting
 * operation and stops live children when this run ends, so nothing here
 * issues a cascade.
 *
 * PURE (deterministic execution policy): everything is derived from events.
 */

import { note } from './state.js';

const WAIT_FUNCTION = '/lib/raisin/ai/wait-for-agents';
const TERMINAL = new Set(['completed', 'failed', 'stopped']);
const MAX_CHILDREN = 24;
const MAX_REWAITS = 32;

function upsert(state, c) {
  if (!c || !c.run_id) return;
  state.children = state.children || [];
  const i = state.children.findIndex((x) => x.run_id === c.run_id);
  const prev = i >= 0 ? state.children[i] : {};
  const next = {
    run_id: c.run_id,
    key: c.key || prev.key || null,
    agent_ref: c.agent_ref || prev.agent_ref || null,
    task_id: c.task_id || prev.task_id || null,
    status: c.status || prev.status || 'queued',
    outcome: c.outcome || prev.outcome || null,
    accepted: c.accepted === undefined ? (prev.accepted === undefined ? null : prev.accepted) : c.accepted,
    handed_back: !!(c.handed_back || prev.handed_back),
  };
  if (i >= 0) state.children[i] = next;
  else state.children.push(next);
  if (state.children.length > MAX_CHILDREN) state.children.shift();
}

/** Core's hand-back envelope (what a wait receives when a child finishes). */
export function handbackOf(env) {
  const p = env && !env.legacy && env.payload && typeof env.payload === 'object' ? env.payload : null;
  return p && typeof p.child_run_id === 'string' && p.contract && typeof p.contract === 'object' ? p : null;
}

/** Learn about children from a delegation tool's payload or a hand-back. */
export function trackChildren(state, env) {
  const hb = handbackOf(env);
  if (hb) {
    upsert(state, {
      run_id: hb.child_run_id,
      status: hb.status,
      outcome: hb.outcome && hb.outcome.kind,
      handed_back: true,
    });
    return;
  }
  const p = env && !env.legacy && env.payload && typeof env.payload === 'object' ? env.payload : null;
  if (!p) return;
  if (p.child && typeof p.child === 'object') upsert(state, p.child);
  if (!Array.isArray(p.children)) return;
  for (const c of p.children) {
    if (c && c.run_id && (c.ack === 'applied' || c.ack === 'duplicate')) upsert(state, { run_id: c.run_id, status: 'stopped' });
    else upsert(state, c);
  }
}

/** Children still working (not terminal, not handed back). */
export function liveChildren(state) {
  return (state.children || []).filter((c) => !TERMINAL.has(c.status) && !c.handed_back && c.status !== 'stopped');
}

/** What the model reads about its children (authoritative facts). */
export function childFacts(state) {
  const list = state.children || [];
  if (!list.length) return null;
  return list.map((c) => ({
    key: c.key, run_id: c.run_id, agent: c.agent_ref, task: c.task_id,
    status: c.handed_back && !TERMINAL.has(c.status) ? 'handed_back' : c.status,
    outcome: c.outcome, accepted: c.accepted,
  }));
}

/**
 * A child's hand-back answered a waiting `wait_for_agents`: call the wait
 * again (same call, same arguments) so it answers with every child it names —
 * at once when they are done, or by waiting on the next one. The model gets
 * ONE answer for its wait. Returns the effect id, or null when this result is
 * not a hand-back into a wait.
 */
export function continueWait(state, fx, env, toolContext) {
  const aw = state.awaiting;
  if (!aw || aw.kind !== 'tool' || aw.function_path !== WAIT_FUNCTION || !handbackOf(env)) return null;
  const rewaits = (aw.rewaits || 0) + 1;
  if (rewaits > MAX_REWAITS) {
    note(state, 'wait_bounded', 'the wait was re-issued too often; answering with the last hand-back', 'warning');
    return null;
  }
  const effectId = fx.push('call_tool', {
    tool: aw.function_path,
    args: { ...(aw.args || {}), __raisin_context: toolContext },
    mutating: false,
    replay_safe: true,
    interruptible: true,
    for_call_id: aw.call_id,
  });
  state.awaiting = { ...aw, effect_id: effectId, rewaits };
  return effectId;
}

/**
 * A steer while this run waits on its children: the wait's external request
 * is withdrawn (the reducer asks for it), and the waiting call is answered
 * here — the children keep working.
 */
export function interruptWait(state, run) {
  const aw = state.awaiting;
  if (!aw || aw.kind !== 'tool') return false;
  const open = Array.isArray(run.open_requests) ? run.open_requests : [];
  const parked = open.some((r) => r && r.kind === 'external' && r.effect_id === aw.effect_id);
  if (!parked) return false;
  state.results.push({
    call_id: aw.call_id,
    synthetic: true,
    content: {
      status: 'interrupted',
      message: 'The wait was interrupted by a new message from the user. The child runs keep working; wait for them again, inspect, message or interrupt them.',
    },
  });
  state.awaiting = null;
  return true;
}

// ── Write scope (a child's grant) ───────────────────────────────────────────

const PATH_KEYS = ['path', 'node_path', 'target_path', 'parent_path', 'parent', 'destination', 'to_path'];

function inScope(scope, ws, path) {
  const p = String(path || '');
  return scope.some((s) => s.workspace === ws
    && (p === s.path || s.path === '/' || p.startsWith(`${String(s.path).replace(/\/$/, '')}/`)));
}

/** The write scope of this run, or null when unrestricted. */
export function writeScope(state) {
  return Array.isArray(state.cfg.write_scope) ? state.cfg.write_scope : null;
}

/**
 * Refuse a MUTATING call whose arguments name a location outside the grant.
 * Returns a refusal message, or null. Arguments without a recognisable
 * location pass; the envelope's reported writes are checked afterwards.
 */
export function scopeRefusal(state, tool, args) {
  const scope = writeScope(state);
  if (!scope || !tool.mutating) return null;
  if (!scope.length) return `\`${tool.name}\` writes, and this delegated run is read-only.`;
  const a = args && typeof args === 'object' ? args : {};
  const ws = typeof a.workspace === 'string' ? a.workspace : null;
  if (!ws) return null;
  for (const k of PATH_KEYS) {
    const v = a[k];
    if (typeof v === 'string' && v.startsWith('/') && !inScope(scope, ws, v)) {
      return `\`${tool.name}\` would write ${ws}:${v}, outside what this delegated run may write (${scope.map((s) => `${s.workspace}:${s.path}`).join(', ')}).`;
    }
  }
  return null;
}

/** Writes an envelope reported outside the grant (after the fact). */
export function scopeViolations(state, writes) {
  const scope = writeScope(state);
  if (!scope || !Array.isArray(writes)) return [];
  const out = [];
  for (const w of writes) {
    const loc = w && w.locator;
    if (loc && loc.path && !inScope(scope, loc.workspace, loc.path)) out.push(`${loc.workspace}:${loc.path}`);
  }
  if (out.length) note(state, 'write_scope_violation', `wrote outside the granted scope: ${out.join(', ')}`, 'error');
  return out;
}
