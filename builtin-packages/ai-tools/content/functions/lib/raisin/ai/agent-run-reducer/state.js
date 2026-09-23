/**
 * The generic agent-loop reducer's state, config and effect builder.
 *
 * PURE (deterministic execution policy): no host calls, no clock, no entropy.
 */

import { DIGEST_ALG, canonicalJson, normToolName, turnMessagePath } from '../agent-shared/run-names.js';

export const STATE_VERSION = 1;
export const DEFAULTS = Object.freeze({
  loop_limit: 3,
  loop_hits_limit: 3,
  no_progress_limit: 4,
  checkpoint_every: 8,
  max_result_chars: 8000,
  model_retry_limit: 2,
});
const MAX_WRITES = 40;
const MAX_FINGERPRINTS = 200;

/** Effects of one response, with ids `{rev}:{i}` (contract rule R3). */
export class Effects {
  constructor(rev) {
    this.rev = rev;
    this.list = [];
  }
  push(kind, body) {
    const effect = { effect_id: `${this.rev}:${this.list.length}`, kind, ...body };
    this.list.push(effect);
    return effect.effect_id;
  }
  hasOperation() {
    return this.list.some((e) => ['call_tool', 'request_model_turn', 'request_approval', 'ask_user'].includes(e.kind));
  }
}

function num(v, fallback) {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : fallback;
}

/**
 * The run's own input. A CHILD run's input is core's delegation envelope
 * (`{objective, context, input, parent_run_id, child_no}`); what the spawning
 * tool gave the reducer is its `input`.
 */
export function runInput(input) {
  const src = input && typeof input === 'object' ? input : {};
  if (src.parent_run_id && src.objective && src.input && typeof src.input === 'object') return src.input;
  return src;
}

/** A fresh state from `run_started.data.input`. */
export function initState(input, seq) {
  const src = runInput(input);
  const cfgIn = src.config && typeof src.config === 'object' ? src.config : {};
  const tools = (Array.isArray(src.tools) ? src.tools : [])
    .filter((t) => t && typeof t.name === 'string' && t.name)
    .map((t) => ({
      name: t.name,
      kind: t.kind === 'domain' ? 'domain' : 'function',
      function_path: t.function_path || null,
      domain_op: t.domain_op || null,
      mutating: t.mutating !== false,
      replay_safe: t.replay_safe === true,
      description: typeof t.description === 'string' ? t.description.slice(0, 1000) : undefined,
    }));
  return {
    v: STATE_VERSION,
    last_event_seq: seq,
    objective: typeof src.text === 'string' ? src.text.slice(0, 2000) : '',
    cfg: {
      execution_mode: cfgIn.execution_mode || 'automatic',
      requires_approval: cfgIn.requires_approval === true,
      loop_limit: num(cfgIn.loop_limit, DEFAULTS.loop_limit),
      loop_hits_limit: num(cfgIn.loop_hits_limit, DEFAULTS.loop_hits_limit),
      no_progress_limit: num(cfgIn.no_progress_limit, DEFAULTS.no_progress_limit),
      checkpoint_every: num(cfgIn.checkpoint_every, DEFAULTS.checkpoint_every),
      max_result_chars: num(cfgIn.max_result_chars, DEFAULTS.max_result_chars),
      model_retry_limit: num(cfgIn.model_retry_limit, DEFAULTS.model_retry_limit),
      instructions: typeof cfgIn.instructions === 'string' ? cfgIn.instructions.slice(0, 4000) : null,
      // A delegated run's write grant (`[{workspace, path}]`); null = unrestricted.
      write_scope: Array.isArray(cfgIn.write_scope) ? cfgIn.write_scope.filter((s) => s && s.workspace && s.path).slice(0, 20) : null,
      // A structured answer (a flow step's response_format): the final model
      // turn answers with ONE JSON value matching it.
      output_schema: cfgIn.output_schema && typeof cfgIn.output_schema === 'object' ? cfgIn.output_schema : null,
    },
    ctx: src.context && typeof src.context === 'object' ? src.context : {},
    tools,
    awaiting: null,
    queue: [],
    results: [],
    last_turn: null,
    last_text: '',
    plan: null,
    plan_dirty: false,
    plan_seq: 0,
    progress: {
      model_turns: 0, tool_calls: 0, tool_ok: 0, tool_failed: 0, refused: 0,
      writes: 0, loop_hits: 0, no_progress: 0, steers: 0,
    },
    turn_stats: { calls: 0, ok: 0 },
    fingerprints: {},
    writes: [],
    children: [],
    steer_pending: false,
    after_turn: null,
    finish: null,
    model_retries: 0,
    diagnostics: [],
  };
}

/** The offered tool a model's call names, by `-`/`_`-insensitive name. */
export function findTool(state, name) {
  const want = normToolName(name);
  return state.tools.find((t) => normToolName(t.name) === want) || null;
}

/** Context every function tool receives (core adds run_id/operation_id). */
export function toolContext(state, run) {
  const chatPath = state.ctx.chat_path || (run.subject && run.subject.path) || null;
  const msgPath = chatPath && state.last_turn ? turnMessagePath(chatPath, run.run_id, state.last_turn.op_id) : null;
  return {
    ...state.ctx,
    chat_path: chatPath,
    conversation_path: chatPath,
    msg_path: msgPath,
    execution_mode: state.cfg.execution_mode,
    orchestration_mode: state.cfg.execution_mode,
    orchestration_round: state.progress.model_turns,
  };
}

/** Fingerprint of a call: tool + canonical args, run context excluded. */
export function fingerprint(name, args) {
  const clean = args && typeof args === 'object' && !Array.isArray(args) ? { ...args } : { value: args };
  delete clean.__raisin_context;
  return `${normToolName(name)}:${canonicalJson(clean)}`;
}

/** Count one executed call (bounded map). */
export function countFingerprint(state, fp) {
  const keys = Object.keys(state.fingerprints);
  if (!(fp in state.fingerprints) && keys.length >= MAX_FINGERPRINTS) delete state.fingerprints[keys[0]];
  state.fingerprints[fp] = (state.fingerprints[fp] || 0) + 1;
}

/** Record writes a native envelope reported (bounded, deduplicated). */
export function recordWrites(state, writes, opId) {
  if (!Array.isArray(writes)) return 0;
  let n = 0;
  for (const w of writes) {
    const loc = w && w.locator;
    if (!loc || !loc.path) continue;
    n += 1;
    const key = `${loc.workspace || ''}:${loc.path}`;
    state.writes = state.writes.filter((x) => x.key !== key);
    state.writes.push({ key, workspace: loc.workspace || null, path: loc.path, action: w.action || 'updated', op: opId });
    if (state.writes.length > MAX_WRITES) state.writes.shift();
  }
  state.progress.writes += n;
  return n;
}

/** A value the model reads back for a tool call, bounded to `max` characters. */
export function boundedContent(value, max, ref) {
  let text;
  try {
    text = JSON.stringify(value === undefined ? null : value);
  } catch (_) {
    text = String(value);
  }
  if (text.length <= max) return value === undefined ? null : value;
  return {
    truncated: true,
    original_chars: text.length,
    preview: text.slice(0, max),
    full_result_ref: ref || null,
    note: 'The result was larger than the context budget; the preview is the start of it.',
  };
}

/** What the model reads for a tool envelope. */
export function contentOfEnvelope(env) {
  if (!env || typeof env !== 'object') return { status: 'failed', error: 'the tool returned nothing' };
  if (env.legacy) return env.payload === undefined ? null : env.payload;
  const out = { status: env.status };
  if (env.payload !== undefined && env.payload !== null) out.result = env.payload;
  if (Array.isArray(env.diagnostics) && env.diagnostics.length) out.diagnostics = env.diagnostics.slice(0, 10);
  if (Array.isArray(env.suggested_next_actions) && env.suggested_next_actions.length) {
    out.suggested_next_actions = env.suggested_next_actions.slice(0, 5);
  }
  if (Array.isArray(env.writes) && env.writes.length) {
    out.writes = env.writes.slice(0, 10).map((w) => ({ action: w.action, ...(w.locator || {}) }));
  }
  if (env.retry_policy && env.retry_policy.retryable) out.retry_policy = env.retry_policy;
  return out;
}

/** Keep the last few diagnostics for the response and the context facts. */
export function note(state, code, message, severity = 'info') {
  state.diagnostics.push({ code, severity, message: String(message).slice(0, 500) });
  if (state.diagnostics.length > 8) state.diagnostics.shift();
}

export { DIGEST_ALG };
