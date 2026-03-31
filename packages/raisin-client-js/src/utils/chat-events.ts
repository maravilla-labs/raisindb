import type { ChatDoneEvent, ChatEvent } from '../types/chat';

/**
 * Detect safety-net done events emitted by backend recovery paths.
 *
 * These events can appear while async tool calls are still running.
 * They carry `recovered: true` and an empty content payload.
 */
export function isRecoveredDoneEvent(event: ChatEvent): event is ChatDoneEvent & { recovered: true } {
  if (event.type !== 'done') return false;
  const done = event as ChatDoneEvent & { recovered?: unknown };
  if (done.recovered !== true) return false;
  return typeof done.content !== 'string' || done.content.trim().length === 0;
}

