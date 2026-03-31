import { describe, expect, it } from 'vitest';
import type { ChatEvent } from '../types/chat';
import { isRecoveredDoneEvent } from './chat-events';

describe('isRecoveredDoneEvent', () => {
  it('matches recovered done events with empty content', () => {
    const event: ChatEvent = {
      type: 'done',
      recovered: true,
      finishReason: 'error',
      content: '',
      timestamp: '2026-02-18T18:00:00.000Z',
    };
    expect(isRecoveredDoneEvent(event)).toBe(true);
  });

  it('does not match normal terminal done events', () => {
    const event: ChatEvent = {
      type: 'done',
      finishReason: 'stop',
      content: 'Final response',
      timestamp: '2026-02-18T18:00:00.000Z',
    };
    expect(isRecoveredDoneEvent(event)).toBe(false);
  });

  it('does not match non-done events', () => {
    const event: ChatEvent = {
      type: 'waiting',
      timestamp: '2026-02-18T18:00:00.000Z',
    };
    expect(isRecoveredDoneEvent(event)).toBe(false);
  });
});

