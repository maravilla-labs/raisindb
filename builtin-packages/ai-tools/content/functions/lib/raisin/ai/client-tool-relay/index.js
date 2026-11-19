/**
 * client-tool-relay — the engine half of a PAGE tool call.
 *
 * The run's reducer dispatches a model's call of `page.<name>` here, with the
 * call described in `client_call` (never mixed into the model's own args).
 * This function:
 *   1. verifies the run exists and is about the conversation named in the
 *      run context (it runs as system, so it must not trust a caller);
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
import { CLIENT_RESUME_PREFIX, DEFAULT_TIMEOUT_MS, MAX_TIMEOUT_MS } from '../agent-shared/client-tools.js';

async function verifyRun(runId, rc) {
  if (!raisin.agentRuns || typeof raisin.agentRuns.get !== 'function') return;
  const view = await raisin.agentRuns.get({ run_id: runId });
  const subject = view && view.run && view.run.subject;
  if (!subject || subject.path !== rc.chat_path || (subject.workspace || 'ai') !== (rc.workspace || 'ai')) {
    throw new Error('client-tool-relay: the run is not about this conversation');
  }
}

export async function handler(input = {}) {
  const run = runContextOf(input);
  const rc = (input && input.__raisin_context) || {};
  const opId = run ? run.operation_id : 'unknown';
  const call = input && input.client_call;
  try {
    if (!run) throw new Error('client-tool-relay runs only inside an agent run');
    if (!call || typeof call.call_id !== 'string' || !call.call_id || typeof call.name !== 'string' || !call.name) {
      throw new Error('client-tool-relay needs client_call { call_id, name }');
    }
    if (!rc.chat_path) throw new Error('client-tool-relay needs the conversation path');
    await verifyRun(run.run_id, rc);
    const base = await loadRunChat({ workspace: rc.workspace || 'ai', path: rc.chat_path });
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
