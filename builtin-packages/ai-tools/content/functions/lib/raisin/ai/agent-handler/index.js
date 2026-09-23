/**
 * Agent Handler — the entry of every agent conversation.
 *
 * Triggered by `messaging-agent-chat` when a raisin:Message is delivered to an
 * agent's inbox. EVERY conversation runs as an AgentRun (RaisinDB core's
 * durable run): this handler only STARTS the conversation's run or STEERS the
 * live one (see agent-shared/run-entry.js). The run owns the lifecycle —
 * model turns, tool calls, stop, steer, approval, recovery after a worker
 * dies — so nothing here calls a model, dispatches a tool or keeps turn state
 * on a node.
 *
 * Flow:
 *   1. Parse the trigger input → message path, chat path
 *   2. Accept only an inbound user turn (skip agent echoes, intermediates)
 *   3. Mark the inbox message read
 *   4. Load the conversation and its agent
 *   5. Route the message to the run (start / steer / duplicate)
 *
 * A server that cannot start a run answers the user with an honest error
 * turn instead of an answer produced some other way.
 */

import { log, setContext } from '../agent-shared/logger.js';
import { resolveAgentOutboxContext } from '../agent-shared/outbox.js';
import { resolveStreamChannel, emitAssistantTurnError } from '../agent-shared/streaming.js';
import { loadSkillGrant } from '../agent-shared/skills.js';
import { agentRunsAvailable } from '../agent-shared/run-control.js';
import { routeToAgentRun } from '../agent-shared/run-entry.js';

const NO_RUNTIME = 'This server cannot run agent conversations (the agent run runtime or its reducer is not available).';

export async function handleUserMessage(context) {
  const input = context.flow_input ?? context;
  const workspace = input.workspace ?? 'ai';
  const event = input.event ?? (input.node ? {
    type: 'Created',
    node_id: input.node.id,
    node_type: input.node.node_type,
    node_path: input.node.path,
  } : null);

  if (!event?.node_path) {
    console.error('[agent-handler] Missing event or node_path');
    return;
  }

  const messagePath = event.node_path;
  const chatPath = messagePath.split('/').slice(0, -1).join('/');
  if (!chatPath.startsWith('/agents/')) {
    log.debug('handler', 'Skipping non-agent conversation path', { chat: chatPath });
    return;
  }
  setContext({ chat: chatPath });
  log.info('handler', 'Triggered', { msg: messagePath, workspace });

  const message = (input.node?.path === messagePath)
    ? input.node
    : await raisin.nodes.get(workspace, messagePath);
  if (!message) {
    log.warn('handler', 'Message not found', { path: messagePath });
    return;
  }
  if (!isInboundUserTurn(message)) {
    log.debug('handler', 'Skipping: not an inbound user turn');
    return;
  }

  await markAsRead(workspace, chatPath, message);

  const chat = await raisin.nodes.get(workspace, chatPath);
  if (!chat?.properties?.agent_ref) {
    throw new Error(`Chat not found or missing agent_ref: ${chatPath}`);
  }
  const streamChannel = resolveStreamChannel(chatPath, chat);
  setContext({ channel: streamChannel, chat: chatPath });
  const outboxCtx = await resolveAgentOutboxContext(workspace, chatPath, chat);

  const agentRef = chat.properties.agent_ref;
  const agentPath = typeof agentRef === 'string' ? agentRef : agentRef['raisin:path'];
  const agentWorkspace = typeof agentRef === 'object'
    ? (agentRef['raisin:workspace'] ?? 'functions')
    : 'functions';
  const agent = await raisin.nodes.get(agentWorkspace, agentPath);
  if (!agent) throw new Error(`Agent not found: ${agentPath}`);
  const agentProps = agent.properties ?? {};

  const replyName = `reply-to-${messagePath.split('/').pop()}`;
  let routed = null;
  let failure = null;
  if (agentRunsAvailable()) {
    try {
      routed = await routeToAgentRun({
        workspace, chatPath, chat, message, agentProps, agentPath, agentWorkspace,
        outboxCtx, streamChannel, hasSkills: (await loadAgentSkills(agentProps)).length > 0,
      });
    } catch (err) {
      failure = String((err && err.message) || err);
    }
  }
  if (routed) {
    log.info('handler', 'Routed to agent run', routed);
    return routed;
  }
  const reason = failure || NO_RUNTIME;
  log.error('handler', 'No agent run for this message', { error: reason });
  await emitAssistantTurnError(workspace, chatPath, replyName, reason, outboxCtx, streamChannel);
  return { mode: 'failed', error: reason };
}

/** Explicit role=user, or a chat-type message without a role, never an agent's own echo. */
function isInboundUserTurn(message) {
  const props = message.properties ?? {};
  const role = props.role;
  const msgType = props.message_type;
  const senderId = typeof props.sender_id === 'string' ? props.sender_id : '';
  const isChatType = msgType === 'chat' || msgType === 'direct_message';
  return (role === 'user' || (isChatType && !role)) && !senderId.startsWith('agent:');
}

async function markAsRead(workspace, chatPath, message) {
  if (workspace !== 'ai' || !message?.path) return;
  try {
    if (message.properties?.status !== 'read') {
      await raisin.nodes.updateProperty(workspace, message.path, 'status', 'read');
      await raisin.nodes.updateProperty(workspace, message.path, 'read_at', new Date().toISOString());
    }
  } catch (e) {
    log.warn('handler', 'Failed to mark as read', { error: e.message });
  }
  try {
    const chatNode = await raisin.nodes.get(workspace, chatPath);
    if (Number(chatNode?.properties?.unread_count) > 0) {
      await raisin.nodes.updateProperty(workspace, chatPath, 'unread_count', 0);
    }
  } catch (e) {
    log.warn('handler', 'Failed to reset unread count', { error: e.message });
  }
}

/**
 * The skills this agent was given (its `skills:` plus the global layers).
 * Only their presence matters here: it decides whether `load-skill` is
 * offered. An unreadable skill counts as absent.
 */
async function loadAgentSkills(agentProps) {
  return loadSkillGrant({
    agentProps,
    stepSkills: [],
    getNode: (ws, path) => raisin.nodes.get(ws, path),
    getNodeById: typeof raisin.nodes.getById === 'function' ? (ws, id) => raisin.nodes.getById(ws, id) : undefined,
    getChildren: (ws, path) => raisin.nodes.getChildren(ws, path),
    onReadError: ({ path, id, error }) => {
      log.warn('handler', 'Could not read skill', { path: path || id, error: error && error.message });
    },
  }).catch(() => []);
}
