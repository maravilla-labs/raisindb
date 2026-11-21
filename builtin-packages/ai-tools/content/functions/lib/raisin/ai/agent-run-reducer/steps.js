/**
 * The generic loop's next step: run the next queued tool, project the plan,
 * ask the model, or finish — decided from state alone.
 *
 * PURE (deterministic execution policy).
 */

import { PROJECT_FUNCTION } from '../agent-shared/run-names.js';
import { allowedClientTools, pageToolName } from '../agent-shared/client-tools.js';
import { PLAN_SCHEMAS, planCardView, planStats, approvalSubject } from './plan.js';

const OPEN = new Set(['pending', 'in_progress']);
import { toolContext, countFingerprint, note } from './state.js';
import { childFacts } from './children.js';

/** The `tools_offered` of a model turn. Plan tools are domain tools. */
export function offers(state) {
  if (state.after_turn === 'summarise') return [];
  const mine = state.tools.map((t) => (t.kind === 'domain'
    ? {
      name: t.name, kind: 'domain', function_path: null,
      description: t.description || `Planning: ${t.domain_op}`,
      schema: PLAN_SCHEMAS[t.domain_op] || { type: 'object' },
    }
    : {
      name: t.name, kind: 'function', function_path: t.function_path,
      ...(t.description ? { description: t.description } : {}),
      // The model-turn function resolves the real schema from the function node.
      schema: { type: 'object' },
    }));
  return mine.concat(clientOffers(state));
}

/**
 * The page's tools the agent's policy allows, as `page.<name>` (contract §3).
 * They carry their own schema: no function node holds it.
 */
export function clientOffers(state) {
  return allowedClientTools(state.client).map((t) => ({
    // `domain` is the contract's kind for a tool core never executes (the call
    // comes back to the reducer, which relays it); `client_tool` marks it a page
    // tool. The Rust ToolKind has no other value: a new kind here is
    // `response_undecodable` (v0.7.36).
    name: pageToolName(t.name), kind: 'domain', function_path: null, client_tool: t.name,
    ...(t.description ? { description: t.description } : {}),
    schema: t.inputSchema || { type: 'object' },
  }));
}

/** Authoritative facts the model turn renders into context (never prose). */
export function facts(state) {
  return {
    objective: state.objective,
    plan: planCardView(state),
    plan_stats: planStats(state.plan),
    progress: state.progress,
    recent_writes: state.writes.slice(-20).map((w) => ({ workspace: w.workspace, path: w.path, action: w.action })),
    pending_steer: state.steer_pending,
    ...(state.client && state.client.policy && state.client.context ? { client_context: state.client.context } : {}),
    diagnostics: state.diagnostics.slice(-5),
    last_turn_op: state.last_turn ? state.last_turn.op_id : null,
    ...(childFacts(state) ? { children: childFacts(state) } : {}),
  };
}

const STOP_INSTRUCTIONS = {
  no_progress: 'The last steps did not work and I have to stop. Tell the person plainly what was done, what is still open, and why you could not continue. Do not start anything new; no tools are offered in this turn.',
  loop_detected: 'You kept repeating the same step and I have to stop. Tell the person plainly what was done, what is still open, and what you would need to continue. Do not start anything new; no tools are offered in this turn.',
  page_unanswered: 'The person\'s page did not answer twice in a row, so I have to stop. Tell the person plainly what was done, what may still be waiting on their page, and that they can ask you to continue. Do not start anything new; no tools are offered in this turn.',
};

function instructionsFor(state) {
  const parts = [];
  let restate = false;
  if (state.cfg.instructions) parts.push(state.cfg.instructions);
  if (state.after_turn === 'summarise') {
    const stop = state.pending_stop && STOP_INSTRUCTIONS[state.pending_stop.reason];
    parts.push(stop || 'Stop here: summarise what was done and what remains for the user. Do not start another task; no tools are offered in this turn.');
    if (stop) restate = true;
  }
  if (state.steer_pending) {
    parts.push('The user sent a new message while you were working. Read it and take it into account before anything else.');
  }
  if (Array.isArray(state.steer_texts) && state.steer_texts.length) {
    parts.push(`New input from the user:\n${state.steer_texts.map((t) => `- ${t}`).join('\n')}`);
    state.steer_texts = [];
  }
  if (state.loop_notice) {
    parts.push(`Runtime notice: ${state.loop_notice}`);
    state.loop_notice = null;
    restate = true;
  }
  if (state.nudge_pending) {
    parts.push(`Runtime notice: ${state.nudge_pending}`);
    state.nudge_pending = null;
    restate = true;
  }
  state.restate_next = restate;
  return parts.length ? parts.join('\n\n') : undefined;
}

/** Answers for every call of the last model turn (contract rule R12). */
function toolResults(state) {
  const calls = state.last_turn ? state.last_turn.calls : [];
  return calls.map((id) => {
    const r = state.results.find((x) => x.call_id === id);
    return r || { call_id: id, synthetic: true, content: { status: 'not_run', message: 'This call was not executed.' } };
  });
}

/** Ask the model for its next turn. */
export function askModel(state, fx) {
  const ts = state.turn_stats;
  // A declined or expired page answer is neutral: it neither resets nor feeds the stall count.
  if (ts.calls > 0) {
    if (ts.ok > 0) state.progress.no_progress = 0;
    else if (ts.calls - (ts.neutral || 0) > 0) state.progress.no_progress += 1;
  }
  state.turn_stats = { calls: 0, ok: 0, neutral: 0 };
  if (state.after_turn !== 'summarise' && state.progress.no_progress >= state.cfg.no_progress_limit) {
    note(state, 'no_progress', `${state.progress.no_progress} consecutive turns made no successful tool call`, 'warning');
    return stopOrSummarise(state, fx, { outcome: 'blocked', reason: 'no_progress' });
  }
  const results = toolResults(state);
  const nextTurn = state.progress.model_turns + 1;
  if (state.cfg.checkpoint_every > 0 && nextTurn > 1 && (nextTurn - 1) % state.cfg.checkpoint_every === 0) {
    fx.push('checkpoint', { reason: 'periodic', summary: `after ${nextTurn - 1} model turns` });
  }
  const instructions = instructionsFor(state);
  const effectId = fx.push('request_model_turn', {
    tools_offered: offers(state),
    ...(instructions ? { instructions } : {}),
    tool_results: results,
    context: { checkpoint: true, facts: facts(state), ...(state.restate_next && instructions ? { restate_instructions: true } : {}) },
    ...(state.cfg.output_schema ? { output_schema: state.cfg.output_schema } : {}),
  });
  state.steer_pending = false;
  state.restate_next = false;
  state.awaiting = { kind: 'model', effect_id: effectId };
  return effectId;
}

/** Issue the projection operation; `then` says what follows its result. */
export function project(state, fx, run, reason, then) {
  state.plan_dirty = false;
  const closed = state.closed_plans;
  state.closed_plans = [];
  const effectId = fx.push('call_tool', {
    tool: PROJECT_FUNCTION,
    args: {
      reason,
      plan: planCardView(state),
      ...(closed.length ? { closed_plans: closed } : {}),
      finish: state.finish,
      last_text: state.last_text,
      progress: state.progress,
      __raisin_context: toolContext(state, run),
    },
    mutating: true,
    replay_safe: true,
    interruptible: true,
  });
  state.awaiting = { kind: 'project', effect_id: effectId, then };
  return effectId;
}

/** Run the next queued tool, or move on. */
export function dispatchNext(state, fx, run) {
  if (state.steer_pending && state.queue.length) {
    for (const q of state.queue) {
      state.results.push({ call_id: q.call_id, synthetic: true, content: { status: 'not_run', message: 'Not run: the user sent a new message first.' } });
    }
    state.queue = [];
  }
  const next = state.queue.shift();
  if (next) {
    countFingerprint(state, next.fp);
    state.progress.tool_calls += 1;
    state.turn_stats.calls += 1;
    // A page tool's call goes to the relay with the call described apart from
    // the model's own arguments (which never share a level with ours).
    const args = next.client
      ? { client_call: next.client, __raisin_context: toolContext(state, run) }
      : { ...(next.args || {}), __raisin_context: toolContext(state, run) };
    const effectId = fx.push('call_tool', {
      tool: next.function_path,
      args,
      mutating: next.mutating,
      replay_safe: next.replay_safe,
      interruptible: true,
      for_call_id: next.call_id,
    });
    // The call is kept with the wait, so a wait for child runs can be
    // re-issued for the same call when a hand-back answers it (children.js).
    state.awaiting = {
      kind: 'tool', effect_id: effectId, call_id: next.call_id, name: next.name,
      function_path: next.function_path, args: next.args || {},
      mutating: next.mutating !== false,
      label: next.label || null,
      ...(next.client ? { client: true, resume_key: next.client.resume_key } : {}),
    };
    return;
  }
  if (state.plan && state.plan.status === 'pending_approval' && state.plan.approval && !state.plan.approval.request_open) {
    if (state.plan_dirty) return project(state, fx, run, 'approval', 'approval');
    return requestApproval(state, fx);
  }
  if (state.plan_dirty || state.closed_plans.length) return project(state, fx, run, 'plan', 'continue');
  return askModel(state, fx);
}

/** Open the approval request for the pending plan. */
export function requestApproval(state, fx) {
  const effectId = fx.push('request_approval', { subject: approvalSubject(state.plan) });
  state.plan.approval.request_open = true;
  state.awaiting = { kind: 'approval', effect_id: effectId };
  return effectId;
}

const RUNTIME_STOPS = new Set(['no_progress', 'loop_detected', 'page_unanswered', 'model_failed', 'write_scope_violation']);
const STOP_REASON_TEXT = {
  no_progress: 'my last steps did not work',
  loop_detected: 'I kept repeating the same step',
  page_unanswered: 'the page did not answer',
  model_failed: 'the assistant could not continue',
  write_scope_violation: 'this task tried to change something it was not allowed to',
};

function titles(tasks, max) {
  const names = tasks.slice(0, max).map((t) => t.title);
  return names.join(', ') + (tasks.length > max ? `, and ${tasks.length - max} more` : '');
}

/**
 * Stop because the runtime decided to, not the model: one tools-free turn
 * first, so the person reads the model's own words; the stop is delivered when
 * that turn returns. A stop that was already announced, or a summary turn that
 * is already the current one, finishes directly.
 */
export function stopOrSummarise(state, fx, stop) {
  if (state.after_turn === 'summarise' || state.pending_stop) {
    state.pending_stop = null;
    return finishRun(state, fx, stop);
  }
  state.pending_stop = stop;
  state.after_turn = 'summarise';
  state.queue = [];
  return askModel(state, fx);
}

/**
 * End the run honestly. The outcome comes from the plan, never from the
 * model's prose: a plan with open tasks is `partial`, a loop or a stall is
 * `blocked`. The projection runs first so the transcript shows the result.
 * The status sentence is plain words for the person; the facts it states are
 * also kept as data (`stop_reason`, `tasks_done`, `tasks_open`).
 */
export function finishRun(state, fx, { outcome, reason, code, message, summarised } = {}) {
  const s = planStats(state.plan);
  let out = outcome;
  if (!out) out = !s || s.open === 0 ? 'succeeded' : 'partial';
  const plan = state.plan;
  const done = plan ? plan.tasks.filter((t) => t.status === 'completed').map((t) => ({ key: t.key, title: t.title })) : [];
  const open = plan ? plan.tasks.filter((t) => OPEN.has(t.status)).map((t) => ({ key: t.key, title: t.title })) : [];
  const why = reason || 'model_finished';
  const stopped = RUNTIME_STOPS.has(why);
  if (why === 'model_failed' && message) note(state, 'model_failed', message, 'warning');
  const parts = [];
  if (stopped) parts.push('I stopped before finishing.');
  if (plan) {
    parts.push(`Done: ${done.length} of ${plan.tasks.length} steps${done.length ? ` (${titles(done, 5)})` : ''}.`);
    if (open.length) parts.push(`Still open: ${titles(open, 5)}.`);
  }
  if (stopped) parts.push(`Reason: ${STOP_REASON_TEXT[why]}.`, 'You can ask me to continue.');
  state.finish = {
    outcome: out,
    reason: why,
    failed: code ? { code, message: message || code } : null,
    status_statement: parts.join(' ') || null,
    runtime_stop: stopped,
    summarised: summarised === true,
    tasks_done: done,
    tasks_open: open,
  };
  state.queue = [];
  // Children still working are stopped by the runtime when this run ends.
  return projectFinal(state, fx);
}

/** The final projection: the answer in the transcript, with the runtime's status. */
export function projectFinal(state, fx) {
  const effectId = fx.push('call_tool', {
    tool: PROJECT_FUNCTION,
    args: {
      reason: 'final',
      plan: planCardView(state),
      ...(state.closed_plans.length ? { closed_plans: state.closed_plans } : {}),
      finish: state.finish,
      last_text: state.last_text,
      progress: state.progress,
      __raisin_context: { ...state.ctx, chat_path: state.ctx.chat_path || null },
    },
    mutating: true,
    replay_safe: true,
    interruptible: false,
  });
  state.awaiting = { kind: 'project', effect_id: effectId, then: 'terminal' };
  return effectId;
}

/** The terminal effect after the final projection. */
export function terminal(state, fx) {
  state.awaiting = null;
  const f = state.finish || { outcome: 'succeeded' };
  if (f.failed) {
    fx.push('fail', { code: f.failed.code, message: f.failed.message });
    return;
  }
  fx.push('complete', {
    outcome: f.outcome,
    ...(f.status_statement || state.last_text ? { summary: [state.last_text, f.status_statement].filter(Boolean).join('\n\n').slice(0, 4000) } : {}),
    // `kind` + `locator` are what core matches a child's expected artifacts on.
    artifacts: state.writes.slice(-20).map((w) => ({ kind: 'node', locator: { workspace: w.workspace, path: w.path }, action: w.action, op: w.op })),
  });
}
