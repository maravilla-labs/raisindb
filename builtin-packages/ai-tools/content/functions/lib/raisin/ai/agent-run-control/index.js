/**
 * agent-run-control — the user's controls for a conversation's agent run:
 * stop, pause, resume, steer, approve, reject, answer, and status.
 *
 * Every command goes to the RUNTIME (`raisin.agentRuns.control`), which
 * applies it atomically and answers with an ack: `applied`, `duplicate` (the
 * same `control_id` again) or `rejected` with the reason. Nothing here writes
 * lifecycle state onto a node.
 *
 * Address the run either by `run_id` (the runtime authorizes the caller as
 * the user the run acts for) or by the user's conversation + its private
 * stream channel (the same capability `request-conversation-stop` checks).
 */

import {
  agentRunsAvailable, getRun, controlRun, commandFor, openApproval, openInput, runIdOfChat,
} from '../agent-shared/run-control.js';

const USER_WORKSPACE = 'raisin:access_control';
const AGENT_WORKSPACE = 'ai';
const SAFE_CHANNEL = /^chat:[A-Za-z0-9._:-]+$/;
const ACTIONS = new Set(['stop', 'pause', 'resume', 'steer', 'approve', 'reject', 'answer', 'status']);

const str = (v) => (typeof v === 'string' ? v.trim() : '');

/** The agent-side conversation a user's conversation + channel names. */
async function agentChatFor(conversationPath, streamChannel) {
  if (!conversationPath.startsWith('/') || !SAFE_CHANNEL.test(streamChannel)) {
    throw new Error('agent-run-control: invalid conversation capability');
  }
  const conv = await raisin.nodes.get(USER_WORKSPACE, conversationPath);
  const props = (conv && conv.properties) || {};
  if (!conv || conv.node_type !== 'raisin:Conversation') throw new Error('agent-run-control: conversation not found');
  if (str(props.stream_channel) !== streamChannel) {
    throw new Error('agent-run-control: stream channel does not match conversation');
  }
  const conversationId = str(props.conversation_id) || conversationPath.split('/').pop();
  const rows = await raisin.sql.query(`
    SELECT path, properties FROM '${AGENT_WORKSPACE}'
    WHERE node_type = 'raisin:Conversation'
      AND properties->>'conversation_id' = $1
      AND properties->>'stream_channel' = $2
    LIMIT 2
  `, [conversationId, streamChannel]);
  if (!Array.isArray(rows) || rows.length !== 1) return null;
  return rows[0];
}

function statusOf(view) {
  if (!view) return null;
  const rec = view.run || {};
  return {
    run_id: rec.run_id,
    status: view.status,
    status_reason: rec.status_reason || null,
    projection: view.projection || null,
    open_approval: openApproval(view),
    open_input: openInput(view),
    queued_steers: Array.isArray(rec.steer_queue) ? rec.steer_queue.length : 0,
    usage: rec.usage || null,
  };
}

export async function handler(input = {}) {
  const action = str(input.action);
  if (!ACTIONS.has(action)) throw new Error(`agent-run-control: unknown action '${action}'`);
  if (!agentRunsAvailable()) return { accepted: false, message: 'Agent runs are not available on this server.' };

  let runId = str(input.run_id);
  if (!runId) {
    const chat = await agentChatFor(str(input.conversation_path), str(input.stream_channel));
    runId = chat ? runIdOfChat(chat) : '';
    if (!runId) return { accepted: false, message: 'This conversation has no agent run.' };
  }
  const view = await getRun(runId);
  if (!view) return { accepted: false, run_id: runId, message: 'The run was not found.' };
  if (action === 'status') return { accepted: true, ...statusOf(view) };

  let requestId = str(input.request_id);
  let digest = str(input.subject_digest);
  if (action === 'approve' || action === 'reject') {
    const open = openApproval(view);
    if (!open) return { accepted: false, run_id: runId, message: 'Nothing is waiting for approval.' };
    requestId = requestId || open.request_id;
    // The UI should send the digest it showed; without one, the open request's.
    digest = digest || open.subject_digest;
  }
  if (action === 'answer') {
    const open = openInput(view);
    requestId = requestId || (open && open.request_id) || '';
    if (!requestId) return { accepted: false, run_id: runId, message: 'Nothing is waiting for an answer.' };
  }

  const command = commandFor(action, {
    reason: str(input.reason) || undefined,
    text: input.text,
    requestId,
    subjectDigest: digest,
    value: input.value,
  });
  const controlId = str(input.control_id) || `${action}:${await raisin.crypto.uuid()}`;
  const ack = await controlRun(runId, command, controlId);
  const accepted = !!ack && ack.ack !== 'rejected';
  // A stopped run's child runs are stopped by the runtime (its cascade).
  return {
    accepted,
    run_id: runId,
    control_id: controlId,
    ack,
    // A steer is QUEUED until the loop reaches a safe boundary; the run's
    // events (and `conversation:steer_consumed`) say when it was consumed.
    ...(action === 'steer' && accepted ? { steer_state: 'queued' } : {}),
    message: accepted ? `${action} ${ack.ack}` : `${action} rejected: ${ack && ack.reason}`,
  };
}
