/**
 * The generic agent loop as a `raisin.agent-run.reducer/1` reducer.
 *
 *   (persisted state, authoritative event) -> (state', effects[])
 *
 * Every conversation run of every ai-tools agent is driven by this reducer
 * unless the agent names another (`run_reducer`). It is an ordinary RaisinDB
 * function and runs under the DETERMINISTIC execution policy: the same request
 * always yields the same response, which is what makes re-delivery after a
 * crash a no-op.
 *
 * Guards (contract §C.9):
 * - SEQ GUARD: `seq <= last_event_seq` returns the state unchanged, the same
 *   `state_rev`, and no effects.
 * - EFFECT GUARD: a result for an effect the run is not awaiting changes
 *   nothing but a `stale_result` diagnostic.
 * - TERMINAL: once the run is terminal (or on `stopped`) the reducer only
 *   ingests; the one effect it may emit is a `checkpoint` (R7).
 */

import { REDUCER_CONTRACT } from '../agent-shared/run-names.js';
import { Effects, initState, runInput, note } from './state.js';
import { adoptPlan, projection } from './plan.js';
import { onModelTurn, onToolResult, onOperationFailed } from './turn.js';
import { askModel, dispatchNext, terminal } from './steps.js';
import { interruptWait } from './children.js';

const TERMINAL = new Set(['completed', 'failed', 'stopped']);
const GUARDED = new Set(['model_turn_completed', 'tool_result', 'operation_failed', 'operation_cancelled']);

function respond(req, state, rev, fx) {
  const out = {
    contract: REDUCER_CONTRACT,
    state,
    state_rev: rev,
    effects: fx ? fx.list : [],
  };
  const p = projection(state);
  if (p) out.projection = p;
  if (state.diagnostics.length) out.diagnostics = state.diagnostics.slice(-3);
  return out;
}

/** One reducer call. */
export function reduce(req) {
  const ev = req.event || {};
  const run = req.run || {};
  let state = req.state && typeof req.state === 'object' ? req.state : null;
  if (!state) {
    if (ev.kind !== 'run_started') {
      return {
        contract: REDUCER_CONTRACT, state: {}, state_rev: req.state_rev, effects: [],
        refused: { code: 'state_missing', message: `'${ev.kind}' arrived before run_started` },
      };
    }
  } else if (ev.seq <= state.last_event_seq) {
    return { contract: REDUCER_CONTRACT, state, state_rev: req.state_rev, effects: [] };
  }

  const rev = req.state_rev + 1;
  const fx = new Effects(rev);
  if (!state) {
    const input = runInput(ev.data && ev.data.input);
    state = initState(input, ev.seq);
    state.plan = adoptPlan(input.plan);
    if (state.plan) state.plan_seq = state.plan.no;
  }
  state.last_event_seq = ev.seq;

  if (TERMINAL.has(run.status) || ev.kind === 'stopped') {
    ingestTerminal(state, ev);
    if (ev.kind === 'stopped') fx.push('checkpoint', { reason: 'stopped', summary: 'run stopped' });
    return respond(req, state, rev, fx);
  }

  if (GUARDED.has(ev.kind) && !awaited(state, ev)) {
    note(state, 'stale_result', `ignored ${ev.kind} for effect ${ev.effect_id || '?'}`);
    return respond(req, state, rev, fx);
  }

  switch (ev.kind) {
    case 'run_started':
      askModel(state, fx);
      break;
    case 'model_turn_completed':
      onModelTurn(state, ev, run, fx);
      break;
    case 'tool_result':
      onToolResult(state, ev, run, fx);
      break;
    case 'operation_failed':
      onOperationFailed(state, ev, run, fx);
      break;
    case 'operation_cancelled':
      state.awaiting = null;
      break;
    case 'user_input':
      onUserInput(state, ev, run, fx);
      break;
    case 'request_resolved':
      onRequestResolved(state, ev, run, fx);
      break;
    case 'request_closed':
      onRequestResolved(state, { ...ev, data: { ...(ev.data || {}), decision: 'reject', reason: `request ${ev.data && ev.data.reason}` } }, run, fx);
      break;
    case 'resumed':
      if (!state.awaiting) dispatchNext(state, fx, run);
      break;
    case 'budget_exceeded':
      note(state, 'budget_exceeded', JSON.stringify(ev.data || {}), 'warning');
      break;
    default:
      note(state, 'unknown_event', `ignored event kind '${ev.kind}'`);
  }
  return respond(req, state, rev, fx);
}

function awaited(state, ev) {
  return !!state.awaiting && !!ev.effect_id && state.awaiting.effect_id === ev.effect_id;
}

/** A late result on a terminal run: record what it wrote, emit nothing. */
function ingestTerminal(state, ev) {
  if (ev.kind === 'tool_result' && ev.data && ev.data.envelope && Array.isArray(ev.data.envelope.writes)) {
    for (const w of ev.data.envelope.writes) {
      if (w && w.locator && w.locator.path) {
        state.writes.push({ key: `${w.locator.workspace}:${w.locator.path}`, workspace: w.locator.workspace, path: w.locator.path, action: w.action, op: ev.operation_id, late: true });
      }
    }
  }
  if (ev.kind === 'stopped' && state.plan) state.plan_dirty = true;
  state.awaiting = null;
  state.queue = [];
}

/**
 * Steering: the user's message is in the transcript; the next model turn
 * reads it. Open requests are withdrawn first (R11) — a reply instead of an
 * approval supersedes the pending plan.
 */
function onUserInput(state, ev, run, fx) {
  state.progress.steers += 1;
  state.steer_pending = true;
  // Input that is not a transcript message (a steer sent straight to the
  // runtime) reaches the model through the next turn's instructions. A parent
  // run's message or redirect (core's `parent_message` / `parent_steer`)
  // carries the same shape one level down.
  const inp = steerInput(ev.data && ev.data.input);
  const text = inp && typeof inp === 'object' ? inp.text : (typeof inp === 'string' ? inp : null);
  if (text && !(inp && inp.message_path)) {
    state.steer_texts = (state.steer_texts || []).concat(String(text).slice(0, 4000)).slice(-5);
  }
  const open = Array.isArray(run.open_requests) ? run.open_requests : [];
  for (const r of open) fx.push('withdraw_request', { request_id: r.request_id });
  if (state.plan && state.plan.status === 'pending_approval' && state.plan.approval) {
    state.results.push({
      call_id: state.plan.approval.call_id,
      synthetic: true,
      content: { success: false, status: 'superseded', message: 'The user replied instead of approving the plan. Read their message and revise the plan.' },
    });
    state.plan.status = 'rejected';
    state.plan.approval = null;
    state.plan_dirty = true;
  }
  // Waiting on child runs: the withdraw above releases the wait; answer it.
  interruptWait(state, run);
  if (state.awaiting && state.awaiting.kind !== 'approval' && state.awaiting.kind !== 'input') return;
  state.awaiting = null;
  dispatchNext(state, fx, run);
}

function steerInput(raw) {
  if (!raw || typeof raw !== 'object') return raw;
  if (raw.type === 'parent_message') return raw.message;
  if (raw.type === 'parent_steer') return raw.input;
  return raw;
}

function onRequestResolved(state, ev, run, fx) {
  const d = ev.data || {};
  const plan = state.plan;
  if (plan && plan.approval && (d.kind === 'approval' || d.decision)) {
    const approved = d.decision === 'approve';
    const mode = state.cfg.execution_mode;
    plan.status = approved ? 'active' : 'rejected';
    state.results.push({
      call_id: plan.approval.call_id,
      synthetic: true,
      content: approved
        ? {
          success: true, status: 'approved', plan_id: `plan-${plan.no}`,
          tasks: plan.tasks.map((t) => ({ task_id: t.key, title: t.title, status: t.status })),
          message: mode === 'manual'
            ? 'The plan was approved. Do not start any task: ask the user which task to do first.'
            : mode === 'step_by_step'
              ? `The plan was approved. Do exactly ONE task now, starting with ${plan.tasks[0].key}.`
              : `The plan was approved. Execute it now, task by task, starting with ${plan.tasks[0].key}.`,
        }
        : { success: false, status: 'rejected', reason: d.reason || null, message: 'The user rejected the plan. Ask what to change or propose a revised plan.' },
    });
    plan.approval = null;
    if (approved && mode === 'manual') state.after_turn = 'summarise';
    state.plan_dirty = true;
  } else if (d.kind === 'input') {
    note(state, 'input_received', 'the user answered a question');
  }
  state.awaiting = null;
  dispatchNext(state, fx, run);
}

export { askModel, terminal };
