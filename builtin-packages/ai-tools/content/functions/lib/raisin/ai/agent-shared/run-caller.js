/**
 * The run that called a tool, PROVEN.
 *
 * A tool that runs in a system context (delegation writes into another
 * agent's inbox; memory is stored where the user has no grant) must never act
 * on whatever `__raisin_context` a caller hands it. It first proves, from the
 * run record core keeps, that the named run exists, is not finished, and is
 * executing exactly this operation of exactly this tool — and then takes the
 * user and the agent it acts for from that RECORD, never from the arguments.
 */

import { runContextOf } from './tool-envelope.js';

const TERMINAL = new Set(['completed', 'failed', 'stopped']);

function fail(message, cls) {
  return Object.assign(new Error(message), { error_class: cls });
}

/** Read a run (null when absent or unreadable). */
export async function getRunView(runId) {
  try {
    return await raisin.agentRuns.get({ run_id: runId });
  } catch (_) {
    return null;
  }
}

/** The active operation of a run record, if one is in flight. */
export function activeOp(rec) {
  const state = (rec && rec.state) || {};
  const act = state.activity || {};
  return act.op || state.op || null;
}

/** The user a run acts for (a child acts for its parent's user). */
export function actingUser(rec) {
  const p = (rec && rec.principal) || {};
  if (p.kind === 'user') return p.id || null;
  return p.on_behalf_of || null;
}

/** `functions:/agents/x` → `{workspace, path}` of the run's agent. */
export function runAgent(rec) {
  const ref = String((rec && rec.agent_ref) || '');
  const idx = ref.indexOf(':/');
  return idx > 0 ? { workspace: ref.slice(0, idx), path: ref.slice(idx + 1) } : { workspace: 'functions', path: ref || null };
}

/**
 * Prove the calling run. Returns `{ ctx, view, rec, runId, opId, workspace,
 * chatPath }`; throws (with an `error_class`) outside a run or for any
 * operation that is not the run's active one.
 */
export async function requireRunOperation(input, functionPath, { what = 'This tool' } = {}) {
  const ctx = runContextOf(input);
  if (!ctx) throw fail(`${what} works only inside an agent run.`, 'unsupported');
  if (typeof raisin === 'undefined' || !raisin.agentRuns || typeof raisin.agentRuns.get !== 'function') {
    throw fail('Agent runs are not available on this server.', 'unsupported');
  }
  const view = await getRunView(ctx.run_id);
  const rec = view && view.run;
  if (!rec) throw fail(`run not found: ${ctx.run_id}`, 'not_found');
  if (TERMINAL.has(view.status)) throw fail('the calling run has finished', 'conflict');
  const op = activeOp(rec);
  if (!op || op.op_id !== ctx.operation_id) {
    throw fail(`operation ${ctx.operation_id} is not the run's active operation`, 'permission_denied');
  }
  const tool = op.input && op.input.tool;
  if (functionPath && tool && tool !== functionPath) {
    throw fail(`operation ${ctx.operation_id} runs ${tool}, not ${functionPath}`, 'permission_denied');
  }
  const subject = rec.subject || {};
  return {
    ctx,
    view,
    rec,
    runId: ctx.run_id,
    opId: ctx.operation_id,
    workspace: subject.workspace || 'ai',
    chatPath: subject.path || null,
  };
}
