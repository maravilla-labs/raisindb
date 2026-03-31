/**
 * SSE event emission and terminal side-effect helpers for agent handlers.
 *
 * Manages the lifecycle of conversation events that the frontend consumes:
 *   conversation:message_saved  – new message persisted
 *   conversation:waiting        – agent paused (e.g. plan approval)
 *   conversation:done           – turn finished, UI can re-enable input
 *
 * Terminal markers (turn_terminal_*) ensure each side effect fires at most
 * once, even if the handler is re-entered due to retries.
 */

import { log } from './logger.js';
import { readAssistantContent, updateAssistantContent, countPendingToolCalls, hasCompletedToolCalls, TERMINAL_FALLBACK_TEXT } from './utils.js';
import { sendAgentOutboxMessage } from './outbox.js';

/**
 * Determine the SSE channel for a conversation.
 * Prefers an explicit stream_channel property, falls back to chat:{id}.
 */
function resolveStreamChannel(chatPath, chatNode) {
  const props = chatNode?.properties || {};
  if (props.stream_channel) return props.stream_channel;
  const conversationId = props.conversation_id || chatPath.split('/').pop();
  return conversationId ? `chat:${conversationId}` : null;
}

/**
 * Emit a conversation SSE event with standard metadata.
 */
async function emitConversationEvent(eventType, payload, chatPath, streamChannel) {
  log.debug('streaming', 'Emitting SSE event', { type: eventType, channel: streamChannel });
  await raisin.events.emit(eventType, {
    ...payload,
    conversationPath: chatPath,
    ...(streamChannel ? { channel: streamChannel } : {}),
  });
}

/**
 * Persist a terminal marker flag on an assistant message node.
 * Silently swallows errors to avoid breaking the main handler flow.
 */
async function setTerminalMarker(workspace, messagePath, marker, value) {
  try {
    await raisin.nodes.updateProperty(workspace, messagePath, marker, value);
  } catch (e) {
    log.warn('streaming', 'Failed to set terminal marker', { marker, path: messagePath, error: e.message });
  }
}

/**
 * Resume any terminal side effects that were not yet completed for an
 * assistant message.  Called after the last tool result arrives or when
 * the handler detects a terminal state.
 *
 * Side effects (idempotent via markers):
 *   1. Outbox delivery → turn_terminal_outbox_sent
 *   2. Done event      → turn_terminal_done_emitted
 */
async function resumeTerminalSideEffects(workspace, chatPath, assistantNode, outboxCtx, streamChannel) {
  if (!assistantNode || assistantNode.node_type !== 'raisin:Message') return;
  const props = assistantNode.properties || {};
  if (props.role !== 'assistant') return;

  log.debug('streaming', 'Resuming terminal side effects', { path: assistantNode.path, finish_reason: props.finish_reason });

  // If waiting for plan approval, emit waiting event and stop
  if (props.finish_reason === 'awaiting_plan_approval') {
    if (props.turn_waiting_emitted !== true) {
      await emitConversationEvent('conversation:waiting', {
        type: 'waiting',
        reason: 'awaiting_plan_approval',
        timestamp: new Date().toISOString(),
      }, chatPath, streamChannel);
      await setTerminalMarker(workspace, assistantNode.path, 'turn_waiting_emitted', true);
    }
    return;
  }

  // If orchestration has not reached terminal phase yet, do not emit done/outbox.
  if (typeof props.dispatch_phase === 'string' && props.dispatch_phase !== 'terminal') {
    return;
  }

  // If tool calls are still pending, do nothing yet
  const pendingCount = await countPendingToolCalls(workspace, assistantNode.path);
  if (pendingCount > 0) return;

  // Ensure the message has non-empty content (but don't inject fallback on
  // tool-call-only responses where empty content is expected)
  let content = readAssistantContent(props);
  if (!content.trim()) {
    const hasTCs = await hasCompletedToolCalls(workspace, assistantNode.path);
    if (!hasTCs) {
      content = TERMINAL_FALLBACK_TEXT;
      await updateAssistantContent(workspace, assistantNode, content);
    }
  }

  // 1. Outbox delivery
  if (outboxCtx && props.turn_terminal_outbox_sent !== true) {
    await sendAgentOutboxMessage(workspace, outboxCtx, content, 'chat', {
      model: props.model,
      finish_reason: props.finish_reason || 'stop',
      tokens: props.tokens,
    }, {
      dedupe_key: `chat_terminal:${assistantNode.path}`,
    });
    await setTerminalMarker(workspace, assistantNode.path, 'turn_terminal_outbox_sent', true);
  }

  // 2. Done event
  if (props.turn_terminal_done_emitted !== true) {
    await emitConversationEvent('conversation:done', {
      type: 'done',
      content: content,
      role: 'assistant',
      senderDisplayName: props.sender_display_name || null,
      finishReason: props.finish_reason || 'stop',
      timestamp: new Date().toISOString(),
    }, chatPath, streamChannel);
    await setTerminalMarker(workspace, assistantNode.path, 'turn_terminal_done_emitted', true);
  }
}

/**
 * Create or update an error assistant message and fire terminal side effects.
 * Used when the AI call fails or an unrecoverable error occurs during a turn.
 */
async function emitAssistantTurnError(workspace, chatPath, messageName, errorMessage, outboxCtx, streamChannel) {
  log.error('streaming', 'Emitting error response', { chat: chatPath, error: errorMessage });

  const content = `Error: ${errorMessage}`;
  const senderId = outboxCtx ? outboxCtx.agentUserId : 'ai-assistant';
  const senderName = outboxCtx ? outboxCtx.agentDisplayName : 'AI Assistant';

  // Check if a message already exists at the expected path
  let targetName = messageName;
  let existing = null;
  try {
    existing = await raisin.nodes.get(workspace, `${chatPath}/${messageName}`);
  } catch (_) {
    existing = null;
  }

  // If message exists and already has real content, create a separate error node
  if (existing && (readAssistantContent(existing.properties || {}).trim() || existing.properties?.finish_reason !== 'error')) {
    targetName = `error-${messageName}`;
  }

  let errorNode = existing && targetName === messageName ? existing : null;

  if (!errorNode) {
    try {
      errorNode = await raisin.nodes.create(workspace, chatPath, {
        name: targetName,
        node_type: 'raisin:Message',
        properties: {
          role: 'assistant',
          body: { content, message_text: content },
          content,
          sender_id: senderId,
          sender_display_name: senderName,
          message_type: 'chat',
          status: 'delivered',
          created_at: new Date().toISOString(),
          finish_reason: 'error',
          turn_terminal_outbox_sent: false,
          turn_terminal_done_emitted: false,
          error_details: {
            type: 'execution_error',
            message: errorMessage,
            timestamp: new Date().toISOString(),
          },
        },
      });
    } catch (createError) {
      // Handle race: node created between our check and create
      if (!String(createError?.message || '').includes('already exists')) {
        throw createError;
      }
      errorNode = await raisin.nodes.get(workspace, `${chatPath}/${targetName}`);
    }
  } else {
    await updateAssistantContent(workspace, errorNode, content);
    await raisin.nodes.updateProperty(workspace, errorNode.path, 'finish_reason', 'error');
  }

  await emitConversationEvent('conversation:message_saved', {
    type: 'message_saved',
    messagePath: errorNode.path,
    role: 'assistant',
    timestamp: new Date().toISOString(),
  }, chatPath, streamChannel);

  await resumeTerminalSideEffects(workspace, chatPath, errorNode, outboxCtx, streamChannel);
}

export {
  resolveStreamChannel,
  emitConversationEvent,
  setTerminalMarker,
  resumeTerminalSideEffects,
  emitAssistantTurnError,
};
