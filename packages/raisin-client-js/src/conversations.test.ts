import { describe, expect, it, vi } from 'vitest';
import { ConversationManager } from './conversations';
import type { AuthManager } from './auth';
import type { SqlResult } from './protocol';
import type { ChatEvent } from './types/chat';

const fakeAuth = {
  getAccessToken: () => 'test-token',
} as unknown as AuthManager;

const encoder = new TextEncoder();

function sseFrame(event: string, data: unknown): Uint8Array {
  return encoder.encode(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
}

/** SSE response that emits the given frames and then stalls forever. */
function stallingSseResponse(frames: Uint8Array[]): Response {
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const frame of frames) controller.enqueue(frame);
      // never closes — simulates a stream that dies mid-turn
    },
  });
  return new Response(stream, {
    status: 200,
    headers: { 'content-type': 'text/event-stream' },
  });
}

/**
 * Fake executeSql covering the queries sendMessage issues:
 * current user lookup, conversation properties, and message insert.
 */
function makeExecuteSql(insertedPaths: string[] = []) {
  return vi.fn(async (sql: string, params?: unknown[]): Promise<SqlResult> => {
    if (sql.includes('RAISIN_CURRENT_USER()')) {
      return {
        columns: ['home', 'user_id'],
        rows: [
          {
            // Workspace-prefixed home: must be normalized by the SDK
            home: '/raisin:access_control/users/internal/alice',
            user_id: 'u1',
          },
        ],
        row_count: 1,
      };
    }
    if (sql.includes('SELECT properties FROM')) {
      return {
        columns: ['properties'],
        rows: [
          {
            properties: {
              stream_channel: 'chat:c1',
              conversation_id: 'c1',
              participants: ['u1', 'agent:support'],
            },
          },
        ],
        row_count: 1,
      };
    }
    if (sql.includes('INSERT INTO')) {
      if (params?.[0]) insertedPaths.push(params[0] as string);
      return { columns: ['id'], rows: [{ id: 'm1' }], row_count: 1 };
    }
    return { columns: [], rows: [], row_count: 0 };
  });
}

describe('ConversationManager.sendMessage inactivity timeout', () => {
  it('ends the turn with a synthetic waiting event when the SSE stream stalls', async () => {
    const fetchMock = vi.fn(async () =>
      stallingSseResponse([
        sseFrame('conversation-event', {
          type: 'text_chunk',
          text: 'hel',
          timestamp: 't',
        }),
      ]),
    );
    const insertedPaths: string[] = [];
    const manager = new ConversationManager(
      'http://localhost:8081',
      'demo',
      fakeAuth,
      { fetch: fetchMock as unknown as typeof fetch },
      makeExecuteSql(insertedPaths),
    );

    const seen: ChatEvent[] = [];
    const start = Date.now();
    for await (const event of manager.sendMessage(
      '/users/internal/alice/inbox/chats/c1',
      'Hello!',
      { inactivityTimeoutMs: 60 },
    )) {
      seen.push(event);
    }

    // The streamed event arrives, then the turn ends with `waiting`
    // instead of hanging forever.
    expect(seen.map((e) => e.type)).toEqual(['text_chunk', 'waiting']);
    expect(Date.now() - start).toBeLessThan(3000);

    // SSE was opened against the conversation channel
    const sseCall = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(sseCall[0]).toBe('http://localhost:8081/api/conversations/demo/events');
    expect(JSON.parse(sseCall[1].body as string).channel).toBe('chat:c1');

    // The user message was created under the normalized (prefix-stripped) home
    expect(insertedPaths[0]).toMatch(/^\/users\/internal\/alice\/outbox\/msg-/);
  });

  it('ends with waiting when the SSE stream never produces events', async () => {
    const fetchMock = vi.fn(async () => stallingSseResponse([]));
    const manager = new ConversationManager(
      'http://localhost:8081',
      'demo',
      fakeAuth,
      { fetch: fetchMock as unknown as typeof fetch },
      makeExecuteSql(),
    );

    const seen: ChatEvent[] = [];
    for await (const event of manager.sendMessage(
      '/users/internal/alice/inbox/chats/c1',
      'Hello!',
      { inactivityTimeoutMs: 50 },
    )) {
      seen.push(event);
    }

    expect(seen.map((e) => e.type)).toEqual(['waiting']);
  });

  it('still terminates on a real done event before the timeout', async () => {
    const fetchMock = vi.fn(async () =>
      stallingSseResponse([
        sseFrame('conversation-event', { type: 'text_chunk', text: 'hi', timestamp: 't' }),
        sseFrame('conversation-event', { type: 'done', timestamp: 't' }),
      ]),
    );
    const manager = new ConversationManager(
      'http://localhost:8081',
      'demo',
      fakeAuth,
      { fetch: fetchMock as unknown as typeof fetch },
      makeExecuteSql(),
    );

    const seen: ChatEvent[] = [];
    for await (const event of manager.sendMessage(
      '/users/internal/alice/inbox/chats/c1',
      'Hello!',
      { inactivityTimeoutMs: 5000 },
    )) {
      seen.push(event);
    }

    expect(seen.map((e) => e.type)).toEqual(['text_chunk', 'done']);
  });
});

describe('ConversationManager flow-owned conversations', () => {
  it('resumes the owning flow instead of creating an outbox message', async () => {
    const sql = vi.fn(async (query: string): Promise<SqlResult> => {
      if (query.includes('SELECT properties FROM')) {
        return {
          columns: ['properties'],
          rows: [{ properties: { flow_instance_id: 'flow-123' } }],
          row_count: 1,
        };
      }
      throw new Error(`Unexpected SQL: ${query}`);
    });
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({
      instance_id: 'flow-123',
      job_id: 'job-1',
      status: 'resumed',
    }), { status: 200, headers: { 'content-type': 'application/json' } }));
    const manager = new ConversationManager(
      'http://localhost:8081',
      'demo',
      fakeAuth,
      { fetch: fetchMock as unknown as typeof fetch },
      sql,
    );

    await manager.createUserMessage(
      '/users/internal/alice/conversations/flow-123-chat-1',
      'Please use the human route',
    );

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, request] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('http://localhost:8081/api/flows/demo/instances/flow-123/resume');
    expect(request.method).toBe('POST');
    expect(request.headers).toMatchObject({
      Authorization: 'Bearer test-token',
      'Content-Type': 'application/json',
    });
    expect(JSON.parse(request.body as string)).toEqual({
      resume_data: { message: 'Please use the human route' },
    });
    expect(sql).toHaveBeenCalledTimes(1);
  });
});
