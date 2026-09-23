/**
 * What the generic loop does with a model turn and with an operation's result:
 * the tool gate, loop detection, progress accounting and evidence.
 *
 * PURE (deterministic execution policy).
 */

import {
  findTool, fingerprint, recordWrites, boundedContent, contentOfEnvelope, note, toolContext,
} from './state.js';
import { applyPlanCall, attributeEvidence } from './plan.js';
import { dispatchNext, askModel, finishRun, terminal, requestApproval } from './steps.js';
import { trackChildren, scopeRefusal, scopeViolations, continueWait, handbackOf } from './children.js';

function refuse(state, callId, message, extra = {}) {
  state.progress.refused += 1;
  state.results.push({ call_id: callId, synthetic: true, content: { status: 'refused', error: message, ...extra } });
}

/** A model turn completed: gate every call, answer domain calls, queue the rest. */
export function onModelTurn(state, ev, run, fx) {
  const d = ev.data || {};
  const calls = Array.isArray(d.tool_calls) ? d.tool_calls : [];
  state.progress.model_turns += 1;
  state.model_retries = 0;
  state.results = [];
  state.queue = [];
  state.awaiting = null;
  state.last_text = d.message && typeof d.message.text === 'string' ? d.message.text : '';
  state.last_turn = { op_id: ev.operation_id || null, calls: calls.map((c) => c.call_id).filter(Boolean) };

  if (calls.length === 0 && state.after_turn !== 'summarise' && shouldNudge(state)) {
    state.nudges = (state.nudges || 0) + 1;
    const open = state.plan.tasks.filter((t) => t.status === 'pending' || t.status === 'in_progress').map((t) => `${t.key} "${t.title}"`);
    state.nudge_pending = `The plan still has open tasks (${open.join(', ')}) and runs in ${state.cfg.execution_mode} mode. Continue with them using your tools, or explain why they cannot be done.`;
    return askModel(state, fx);
  }
  if (calls.length === 0 || state.after_turn === 'summarise') {
    for (const c of calls) refuse(state, c.call_id, 'No tools are offered in this turn.');
    return finishRun(state, fx, { reason: state.after_turn === 'summarise' ? 'step_complete' : 'model_finished' });
  }

  let approval = false;
  for (const call of calls) {
    const id = call.call_id;
    if (!id) continue;
    if (approval) {
      refuse(state, id, 'Not run: the plan is waiting for the user\'s approval.');
      continue;
    }
    const tool = findTool(state, call.name);
    if (!tool) {
      const names = state.tools.map((t) => t.name).sort().join(', ') || 'none';
      refuse(state, id, `\`${call.name}\` is not a tool offered to you, so it was not run. The tools offered to you are: ${names}.`);
      continue;
    }
    if (tool.kind === 'domain') {
      const r = applyPlanCall(state, tool.domain_op, call.args, id, ev.operation_id);
      if (r.approval) {
        approval = true;
        continue;
      }
      state.results.push({ call_id: id, synthetic: true, content: r.content });
      if (r.completed && state.cfg.execution_mode === 'step_by_step') state.after_turn = 'summarise';
      continue;
    }
    const fp = fingerprint(tool.name, call.args);
    const seen = state.fingerprints[fp] || 0;
    const queuedTwin = state.queue.some((q) => q.fp === fp);
    if (seen >= state.cfg.loop_limit || queuedTwin) {
      state.progress.loop_hits += 1;
      const msg = queuedTwin
        ? `\`${tool.name}\` was called twice with identical arguments in one turn; it runs once.`
        : `\`${tool.name}\` has already run ${seen} time(s) with exactly these arguments; its result will not change. Use what it returned, change the approach, or finish.`;
      note(state, 'loop_detected', msg, 'warning');
      refuse(state, id, msg, { loop_detected: true });
      continue;
    }
    const outOfScope = scopeRefusal(state, tool, call.args);
    if (outOfScope) {
      note(state, 'write_scope_refused', outOfScope, 'warning');
      refuse(state, id, outOfScope, { write_scope: true });
      continue;
    }
    state.queue.push({
      call_id: id, name: tool.name, function_path: tool.function_path, args: call.args || {},
      mutating: tool.mutating, replay_safe: tool.replay_safe, fp,
    });
  }

  if (state.progress.loop_hits >= state.cfg.loop_hits_limit && state.queue.length === 0) {
    return finishRun(state, fx, { outcome: 'blocked', reason: 'loop_detected' });
  }
  if (approval) {
    state.queue = [];
    state.plan_dirty = true;
  }
  return dispatchNext(state, fx, run);
}

/** A tool (or the projection) returned. */
export function onToolResult(state, ev, run, fx) {
  const d = ev.data || {};
  const awaiting = state.awaiting;
  if (awaiting.kind === 'project') return afterProjection(state, fx, run, awaiting.then);
  const env = d.envelope;
  // A child's hand-back answered a wait: learn it, then wait for the rest.
  if (handbackOf(env)) {
    trackChildren(state, env);
    if (continueWait(state, fx, env, toolContext(state, run))) return undefined;
  }
  const status = env && env.status ? env.status : 'failed';
  const ok = status === 'succeeded';
  const writes = ok && env && !env.legacy ? recordWrites(state, env.writes, ev.operation_id) : 0;
  if (ok) trackChildren(state, env);
  const violations = ok && env && !env.legacy ? scopeViolations(state, env.writes) : [];
  if (ok) {
    state.progress.tool_ok += 1;
    state.turn_stats.ok += 1;
  } else {
    state.progress.tool_failed += 1;
  }
  attributeEvidence(state, ev.operation_id, ok, writes);
  state.results.push({
    call_id: awaiting.call_id || d.call_id,
    synthetic: false,
    content: boundedContent(contentOfEnvelope(env), state.cfg.max_result_chars, ev.operation_id),
  });
  state.awaiting = null;
  if (violations.length) {
    return finishRun(state, fx, { outcome: 'blocked', reason: 'write_scope_violation', message: violations.join(', ') });
  }
  return dispatchNext(state, fx, run);
}

/** An operation failed: tools answer with the error, model turns retry. */
export function onOperationFailed(state, ev, run, fx) {
  const d = ev.data || {};
  const awaiting = state.awaiting;
  if (awaiting.kind === 'project') return afterProjection(state, fx, run, awaiting.then);
  if (awaiting.kind === 'model') {
    state.awaiting = null;
    if (d.retryable && state.model_retries < state.cfg.model_retry_limit) {
      state.model_retries += 1;
      note(state, 'model_retry', `model turn failed (${d.error_class}); retry ${state.model_retries}`, 'warning');
      return askModel(state, fx);
    }
    return finishRun(state, fx, {
      outcome: 'blocked', reason: 'model_failed', code: 'model_turn_failed',
      message: typeof d.message === 'string' ? d.message : String(d.error_class || 'provider'),
    });
  }
  state.progress.tool_failed += 1;
  attributeEvidence(state, ev.operation_id, false, 0);
  const content = {
    status: 'failed',
    error_class: d.error_class || 'tool_error',
    error: typeof d.message === 'string' ? d.message : (d.message ? JSON.stringify(d.message) : 'the tool failed'),
    ...(d.retryable ? { retryable: true } : {}),
    ...(d.outcome_unknown ? { outcome_unknown: true, note: 'The runtime lost this operation; it may or may not have written. Re-read before trying again.' } : {}),
    ...(d.envelope && Array.isArray(d.envelope.diagnostics) ? { diagnostics: d.envelope.diagnostics.slice(0, 10) } : {}),
  };
  state.results.push({ call_id: awaiting.call_id || d.call_id, synthetic: false, content: boundedContent(content, state.cfg.max_result_chars, ev.operation_id) });
  state.awaiting = null;
  return dispatchNext(state, fx, run);
}

function afterProjection(state, fx, run, then) {
  state.awaiting = null;
  if (then === 'terminal') {
    // The user wrote while the final answer was being delivered: that input
    // is consumed, so answer it instead of ending the run.
    if (state.steer_pending) {
      state.finish = null;
      return askModel(state, fx);
    }
    return terminal(state, fx);
  }
  if (then === 'approval' && state.plan && state.plan.approval) return requestApproval(state, fx);
  return dispatchNext(state, fx, run);
}

/** One nudge per run when an auto-mode plan still has open tasks. */
function shouldNudge(state) {
  const plan = state.plan;
  if (!plan || plan.status !== 'active' || (state.nudges || 0) >= 1) return false;
  if (!['automatic', 'approve_then_auto'].includes(state.cfg.execution_mode)) return false;
  return plan.tasks.some((t) => t.status === 'pending' || t.status === 'in_progress');
}
