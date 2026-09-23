import { agentRunsAvailable, runIdOfChat, getRun, isTerminal, controlRun } from '../agent-shared/run-control.js';
import { emitAssistantTurnStopped, resolveStreamChannel } from '../agent-shared/streaming.js';
import { resolveAgentOutboxContext } from '../agent-shared/outbox.js';
import { shortRun } from '../agent-shared/run-names.js';
import { latestOpenPlan, upsertPlanCard, syncPlanCards } from '../agent-shared/run-plan-card.js';

const USER_WORKSPACE = 'raisin:access_control';
const AGENT_WORKSPACE = 'ai';
const SAFE_CHANNEL = /^chat:[A-Za-z0-9._:-]+$/;

function str(value) {
  return typeof value === 'string' ? value.trim() : '';
}

/**
 * A capability-style stop request.
 *
 * Function invocations currently execute as the system actor, so the selected
 * path alone is not sufficient authorization. The caller must also present the
 * stream channel it read from that conversation. We then require the same
 * conversation id + channel on the agent-side mirror before writing anything.
 */
export async function handler(input = {}) {
  const conversationPath = str(input.conversation_path);
  const streamChannel = str(input.stream_channel);
  const reason = str(input.reason).slice(0, 500) || 'Stopped by the user';

  if (!conversationPath.startsWith('/') || !SAFE_CHANNEL.test(streamChannel)) {
    throw new Error('request-conversation-stop: invalid conversation capability');
  }

  const userConversation = await raisin.nodes.get(USER_WORKSPACE, conversationPath);
  const userProps = userConversation?.properties || {};
  if (!userConversation || userConversation.node_type !== 'raisin:Conversation') {
    throw new Error('request-conversation-stop: conversation not found');
  }
  if (str(userProps.stream_channel) !== streamChannel) {
    throw new Error('request-conversation-stop: stream channel does not match conversation');
  }

  const conversationId = str(userProps.conversation_id) || conversationPath.split('/').pop();
  const rows = await raisin.sql.query(`
    SELECT path, properties
    FROM '${AGENT_WORKSPACE}'
    WHERE node_type = 'raisin:Conversation'
      AND properties->>'conversation_id'::STRING = $1
      AND properties->>'stream_channel'::STRING = $2
    LIMIT 2
  `, [conversationId, streamChannel]);

  if (!Array.isArray(rows) || rows.length !== 1) {
    return {
      accepted: false,
      message: rows?.length > 1
        ? 'The agent conversation is ambiguous; no stop request was written.'
        : 'There is no active agent conversation to stop.',
    };
  }

  /* The conversation is stopped THROUGH ITS RUN: the runtime cancels the
   * in-flight operation, starts nothing new, acknowledges the stop, and stops
   * the run's child runs with it. */
  const routed = await stopAgentRun(rows[0], reason, conversationPath);
  if (routed) return routed;
  return { accepted: false, message: 'The agent is not currently running.' };
}

/** Stop the conversation's live agent run; null when it has none. */
async function stopAgentRun(agentChatRow, reason, sourcePath) {
  if (!agentRunsAvailable()) return null;
  const runId = runIdOfChat(agentChatRow);
  if (!runId) return null;
  const view = await getRun(runId);
  if (!view || isTerminal(view)) return null;
  const requestId = await raisin.crypto.uuid();
  const ack = await controlRun(runId, { command: 'stop', reason }, `stop:${requestId}`);
  const accepted = !!ack && ack.ack !== 'rejected';
  if (accepted) {
    const chat = await raisin.nodes.get(AGENT_WORKSPACE, agentChatRow.path);
    const channel = resolveStreamChannel(agentChatRow.path, chat);
    const outbox = await resolveAgentOutboxContext(AGENT_WORKSPACE, agentChatRow.path, chat);
    await emitAssistantTurnStopped(
      AGENT_WORKSPACE, agentChatRow.path, `run-${shortRun(runId)}-stopped`,
      { request_id: requestId, reason, source_conversation_path: sourcePath }, outbox, channel,
    );
    await releasePlanCard({ workspace: AGENT_WORKSPACE, chatPath: agentChatRow.path, chat, runId }, outbox);
  }
  return {
    accepted,
    request_id: requestId,
    run_id: runId,
    ack,
    requested_at: new Date().toISOString(),
    message: accepted
      ? 'Stopped. The run will not start another model or tool round; work already completed is preserved.'
      : `The run did not accept the stop: ${ack && ack.reason}`,
  };
}

/**
 * No stopped run shows an active task: the card's in-progress tasks go back
 * to pending (the next run adopts the plan and may pick them up again).
 */
async function releasePlanCard(ctx, outbox) {
  try {
    const plan = await latestOpenPlan(ctx.workspace, ctx.chatPath);
    if (!plan || !plan.tasks.some((t) => t.status === 'in_progress')) return;
    const released = {
      ...plan,
      tasks: plan.tasks.map((t) => (t.status === 'in_progress' ? { ...t, status: 'pending', interrupted: true } : t)),
    };
    const path = await upsertPlanCard(ctx, released, null);
    if (path) await syncPlanCards(ctx, outbox, released, path);
  } catch (err) {
    console.log('[request-conversation-stop] plan card release failed:', err && err.message);
  }
}
