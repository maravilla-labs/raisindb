/**
 * client-tool-relay — the engine half of a PAGE tool call.
 *
 * The run's reducer dispatches a model's call of `page.<name>` here, with the
 * call described in `client_call` (never mixed into the model's own args).
 * This function:
 *   1. proves the call against the run's RECORD (see provenCall) and takes the
 *      call's name, args, client_id and timeout from it, never from its input;
 *   2. emits the `client_tool_call` conversation event on the conversation's
 *      stream channel (docs/webmcp-agent-contract.md §4) — the page's SDK
 *      executes the tool when `client_id` is its own;
 *   3. answers `waiting` under resume key `client:<call_id>` with
 *      `expires_in_ms` = the call's timeout. The page's answer arrives through
 *      `POST /api/agent-runs/{repo}/{run}/client-result`, the deadline through
 *      the runtime's expiry sweep; either way the reducer answers the model.
 *
 * Replay: the event is NOT idempotent from the page's point of view (a
 * mutating page tool would run twice). The reducer therefore dispatches a
 * mutating page tool as replay_safe:false — after a crash the runtime reports
 * the operation as "outcome unknown" instead of re-running it — and a
 * read-only one as replay_safe:true (a second event only re-reads).
 */

import { log } from '../agent-shared/logger.js';
import { emitConversationEvent } from '../agent-shared/streaming.js';
import { loadRunChat } from '../agent-shared/run-transcript.js';
import { buildEnvelope, errorEnvelope, runContextOf } from '../agent-shared/tool-envelope.js';
import { CLIENT_TOOL_RELAY, CLIENT_RESUME_PREFIX, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS } from '../agent-shared/client-tools.js';

/**
 * The relay runs as SYSTEM and its input is whatever the caller sent, so it
 * proves the call against the RUN RECORD before it emits anything, and takes
 * what it emits from that record, never from its input:
 *  - the run must exist (no runtime = refuse: it fails CLOSED);
 *  - the invoking operation must be the run's ACTIVE operation, a tool call
 *    to this very function, answering the call_id the input names;
 *  - name, args, client_id and the timeout come from the arguments the
 *    reducer recorded on that operation (`client_call`); the conversation
 *    (workspace, path) from the run's subject.
 * Anyone else invoking this function directly finds no such operation.
 */
async function provenCall(runId, opId, claimedCallId) {
  if (!raisin.agentRuns || typeof raisin.agentRuns.get !== 'function') {
    throw new Error('client-tool-relay: the agent run runtime is not available');
  }
  const view = await raisin.agentRuns.get({ run_id: runId });
  const rec = view && view.run;
  const op = rec && rec.state && rec.state.activity && rec.state.activity.op;
  if (!rec || !op || op.op_id !== opId) {
    throw new Error('client-tool-relay: not the run\'s active operation');
  }
  const recorded = op.input && typeof op.input === 'object' ? op.input : {};
  if (recorded.tool !== CLIENT_TOOL_RELAY) {
    throw new Error('client-tool-relay: the active operation is not a page tool call');
  }
  const call = recorded.args && recorded.args.client_call;
  const forCall = typeof op.for_call_id === 'string' ? op.for_call_id : null;
  if (!call || typeof call.call_id !== 'string' || !call.call_id
    || call.call_id !== claimedCallId || (forCall && forCall !== call.call_id)) {
    throw new Error('client-tool-relay: the call is not the one the run recorded');
  }
  const subject = rec.subject;
  if (!subject || !subject.path) throw new Error('client-tool-relay: the run has no conversation');
  return { call, workspace: subject.workspace || 'ai', chatPath: subject.path };
}

export async function handler(input = {}) {
  const run = runContextOf(input);
  const opId = run ? run.operation_id : 'unknown';
  const claimedCallId = input && input.client_call && input.client_call.call_id;
  try {
    if (!run) throw new Error('client-tool-relay runs only inside an agent run');
    const { call, workspace, chatPath } = await provenCall(run.run_id, opId, claimedCallId);
    const base = await loadRunChat({ workspace, path: chatPath });
    const timeout = Math.min(Math.max(Number(call.timeout_ms) || DEFAULT_TIMEOUT_MS, 1000), MAX_TIMEOUT_MS);
    const resumeKey = `${CLIENT_RESUME_PREFIX}${call.call_id}`;
    await emitConversationEvent('conversation:client_tool_call', {
      type: 'client_tool_call',
      run_id: run.run_id,
      call_id: call.call_id,
      resume_key: resumeKey,
      name: call.name,
      args: call.args && typeof call.args === 'object' ? call.args : {},
      client_id: call.client_id || null,
      expires_at_ms: Date.now() + timeout,
    }, base.chatPath, base.streamChannel);
    log.info('client-tool-relay', 'Page tool call sent', { run: run.run_id, call: call.call_id, tool: call.name });
    const env = buildEnvelope({
      operationId: opId,
      status: 'waiting',
      resumeKey,
      payload: { waiting_for: 'page', tool: call.name, timeout_ms: timeout },
    });
    env.expires_in_ms = timeout;
    return env;
  } catch (err) {
    log.error('client-tool-relay', 'Could not relay the page tool call', { error: String(err && err.message) });
    return errorEnvelope(opId, err);
  }
}
