/**
 * User control of a conversation's AgentRun — stop, pause, resume, steer,
 * approve, answer — routed to the RUNTIME, never written onto the chat node.
 *
 * Every control carries a deterministic `control_id`, so a retried click or a
 * redelivered trigger is acknowledged as a duplicate instead of applied twice.
 * The ack says what happened: `applied`, `duplicate`, or `rejected` with the
 * runtime's reason (for instance `run_terminal`).
 */

/** Whether this server runs agent runs at all. */
export function agentRunsAvailable() {
  return typeof raisin !== 'undefined' && !!raisin.agentRuns
    && typeof raisin.agentRuns.create === 'function'
    && typeof raisin.agentRuns.control === 'function';
}

/** The run a conversation node points at (live or most recent). */
export function runIdOfChat(chat) {
  const props = (chat && chat.properties) || {};
  return props.active_agent_run_id || null;
}

/** Every request still open on a run, from its record's state. */
export function openRequests(view) {
  const state = view && view.run && view.run.state;
  if (!state) return [];
  const open = state.open || (state.activity && state.activity.open) || [];
  return Array.isArray(open) ? open : [];
}

function kindOf(req) {
  const k = req && req.kind;
  return typeof k === 'string' ? k : (k && k.kind) || null;
}

/** The open approval request of a run, with the digest an approve must name. */
export function openApproval(view) {
  const req = openRequests(view).find((r) => kindOf(r) === 'approval');
  if (!req) return null;
  const k = typeof req.kind === 'object' ? req.kind : req;
  return { request_id: req.request_id, subject_digest: k.subject_digest, summary: k.summary || null };
}

/** The open input request of a run. */
export function openInput(view) {
  const req = openRequests(view).find((r) => kindOf(r) === 'input');
  return req ? { request_id: req.request_id } : null;
}

/** Submit one control command; returns the ack. */
export async function controlRun(runId, command, controlId) {
  return raisin.agentRuns.control({ run_id: runId, control_id: controlId, command });
}

/** Read a run (null when it does not exist or cannot be read). */
export async function getRun(runId) {
  try {
    return await raisin.agentRuns.get({ run_id: runId });
  } catch (_) {
    return null;
  }
}

const TERMINAL = new Set(['completed', 'failed', 'stopped']);

/** Whether a run view is finished. */
export function isTerminal(view) {
  return !view || TERMINAL.has(view.status);
}

/**
 * Build a control command from a UI action.
 * `action`: stop | pause | resume | steer | approve | reject | answer.
 */
export function commandFor(action, { reason, text, input, requestId, subjectDigest, value } = {}) {
  switch (action) {
    case 'stop': return { command: 'stop', reason: reason || 'Stopped by the user' };
    case 'pause': return { command: 'pause' };
    case 'resume': return { command: 'resume' };
    case 'steer': return { command: 'steer', input: input || { text: String(text || '') } };
    case 'approve': return { command: 'approve', request_id: requestId, decision: { decision: 'approve' }, subject_digest: subjectDigest };
    case 'reject': return { command: 'approve', request_id: requestId, decision: { decision: 'reject', reason: reason || null }, subject_digest: subjectDigest };
    case 'answer': return { command: 'provide_input', request_id: requestId, value };
    default: return null;
  }
}
