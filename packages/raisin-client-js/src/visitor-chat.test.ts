import { afterEach, describe, expect, it, vi } from 'vitest';
import { ConversationManager } from './conversations';
import { EventHandler } from './events';
import type { AuthManager } from './auth';
import type { EventMessage } from './protocol';
import type { ChatEvent } from './types/chat';
import { VisitorChat, VisitorChatError, type VisitorSessionStore } from './visitor-chat';

const fakeAuth = { getAccessToken: () => null } as unknown as AuthManager;
const SECRET = 'a'.repeat(64);

function memoryStore(): VisitorSessionStore & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return {
    data,
    get: (k) => data.get(k) ?? null,
    set: (k, v) => void data.set(k, v),
    remove: (k) => void data.delete(k),
  };
}

function startResponse(conversationId: string, token: string | null = SECRET) {
  return {
    session_token: token,
    session_home: '/visitors/k1',
    conversation_id: conversationId,
    conversation_path: `/visitors/k1/inbox/chats/${conversationId}`,
    conversation_workspace: 'raisin:access_control',
    channel: `chat:${conversationId}`,
    subscription_id: `visitor-chat:${conversationId}`,
    agent: '/agents/site',
    limits: { max_messages: 20, max_message_chars: 2000 },
  };
}

function event(subscriptionId: string, payload: Record<string, unknown>): EventMessage {
  return {
    event_id: `${Math.random()}`,
    subscription_id: subscriptionId,
    event_type: String(payload.type),
    payload: payload as never,
    timestamp: new Date().toISOString(),
  };
}

function setup(send: (payload: any, type: string) => Promise<unknown>) {
  const events = new EventHandler(vi.fn(async () => ({ subscription_id: 'x' })) as never);
  let reconnect: (() => void) | null = null;
  const visitor = new VisitorChat('website', send, events, (cb) => {
    reconnect = cb;
    return () => {};
  });
  const manager = new ConversationManager(
    'http://localhost:8081',
    'website',
    fakeAuth,
    {},
    vi.fn(),
    visitor,
  );
  return { manager, events, reconnect: () => reconnect?.() };
}

const handlers: EventHandler[] = [];
afterEach(() => {
  for (const h of handlers.splice(0)) h.destroy();
});

describe('anonymous visitor chat', () => {
  it('starts a conversation without login and keeps the session for the tab', async () => {
    const send = vi.fn(async (_p: unknown, type: string) =>
      type === 'visitor_chat_start' ? startResponse('vchat-1') : {},
    );
    const { manager, events } = setup(send);
    handlers.push(events);
    const store = memoryStore();

    const convo = await manager.startAnonymous('agent:site', { store });
    expect(send).toHaveBeenCalledWith(
      { agent: '/agents/site', session_token: undefined, conversation_id: undefined },
      'visitor_chat_start',
    );
    expect(convo.anonymous).toBe(true);
    expect(convo.conversationPath).toBe('/visitors/k1/inbox/chats/vchat-1');
    expect(store.data.get('raisindb:visitor:website:/agents/site:session')).toBe(SECRET);
    expect(store.data.get('raisindb:visitor:website:/agents/site:conversation')).toBe('vchat-1');
    expect(manager.isAnonymous(convo.conversationPath)).toBe(true);

    // A reload resumes the same session and conversation.
    const { manager: again, events: e2 } = setup(send);
    handlers.push(e2);
    await again.startAnonymous('/agents/site', { store });
    expect(send).toHaveBeenLastCalledWith(
      { agent: '/agents/site', session_token: SECRET, conversation_id: 'vchat-1' },
      'visitor_chat_start',
    );
  });

  it('streams only its own conversation\'s events, until done', async () => {
    let events!: EventHandler;
    const send = vi.fn(async (p: any, type: string) => {
      if (type === 'visitor_chat_start') return startResponse('vchat-1');
      queueMicrotask(() => {
        // Another conversation's event on the same connection must not leak in.
        events.handleEvent(event('visitor-chat:vchat-2', { type: 'text_chunk', text: 'NOT MINE' }));
        events.handleEvent(event('visitor-chat:vchat-1', { type: 'text_chunk', text: 'Hel' }));
        events.handleEvent(event('visitor-chat:vchat-1', { type: 'text_chunk', text: 'lo' }));
        events.handleEvent(event('visitor-chat:vchat-1', { type: 'done', conversationPath: 'x', timestamp: 't' }));
      });
      expect(p).toEqual({ conversation_id: 'vchat-1', text: 'Hi' });
      return { accepted: true, message_id: 'm1' };
    });
    const s = setup(send);
    events = s.events;
    handlers.push(events);
    const convo = await s.manager.startAnonymous('/agents/site', { store: false });

    const got: ChatEvent[] = [];
    for await (const ev of s.manager.sendMessage(convo.conversationPath, 'Hi')) got.push(ev);
    expect(got.map((e) => e.type)).toEqual(['text_chunk', 'text_chunk', 'done']);
    expect(got.map((e) => (e as { text?: string }).text).join('')).not.toContain('NOT MINE');
  });

  it('ends the turn with failed and the server\'s code when a limit refuses it', async () => {
    const send = vi.fn(async (_p: unknown, type: string) => {
      if (type === 'visitor_chat_start') return startResponse('vchat-1');
      throw Object.assign(new Error('too many messages'), { code: 'RATE_LIMITED' });
    });
    const { manager, events } = setup(send);
    handlers.push(events);
    const convo = await manager.startAnonymous('/agents/site', { store: false });
    const got: ChatEvent[] = [];
    for await (const ev of manager.sendMessage(convo.conversationPath, 'Hi')) got.push(ev);
    expect(got).toHaveLength(1);
    expect(got[0]).toMatchObject({ type: 'failed', code: 'RATE_LIMITED' });

    await expect(manager.createUserMessage(convo.conversationPath, 'Hi')).rejects.toBeInstanceOf(
      VisitorChatError,
    );
  });

  it('proves the session again when the connection was replaced, then retries once', async () => {
    let sends = 0;
    const send = vi.fn(async (_p: unknown, type: string) => {
      if (type === 'visitor_chat_start') return startResponse('vchat-1', null);
      sends += 1;
      if (sends === 1) throw Object.assign(new Error('start first'), { code: 'NO_SESSION' });
      return { accepted: true, message_id: 'm2' };
    });
    const { manager, events } = setup(send);
    handlers.push(events);
    const convo = await manager.startAnonymous('/agents/site', {
      store: false,
      sessionToken: SECRET,
    });
    await manager.createUserMessage(convo.conversationPath, 'Hi');
    const types = send.mock.calls.map((c) => c[1]);
    expect(types).toEqual([
      'visitor_chat_start',
      'visitor_chat_send',
      'visitor_chat_start',
      'visitor_chat_send',
    ]);
    expect(send.mock.calls[2][0]).toEqual({
      agent: '/agents/site',
      session_token: SECRET,
      conversation_id: 'vchat-1',
    });
  });

  it('re-binds every conversation after a reconnect', async () => {
    const send = vi.fn(async () => startResponse('vchat-1'));
    const { manager, events, reconnect } = setup(send);
    handlers.push(events);
    await manager.startAnonymous('/agents/site', { store: false });
    send.mockClear();
    reconnect();
    await new Promise((r) => setTimeout(r, 0));
    expect(send).toHaveBeenCalledWith(
      { agent: '/agents/site', session_token: SECRET, conversation_id: 'vchat-1' },
      'visitor_chat_start',
    );
  });

  it('starts afresh when the remembered conversation is gone', async () => {
    const store = memoryStore();
    store.set('raisindb:visitor:website:/agents/site:session', SECRET);
    store.set('raisindb:visitor:website:/agents/site:conversation', 'vchat-old');
    const send = vi.fn(async (p: any) => {
      if (p.conversation_id === 'vchat-old') {
        throw Object.assign(new Error('gone'), { code: 'UNKNOWN_CONVERSATION' });
      }
      return startResponse('vchat-new');
    });
    const { manager, events } = setup(send);
    handlers.push(events);
    const convo = await manager.startAnonymous('/agents/site', { store });
    expect(convo.conversationId).toBe('vchat-new');
    expect(store.data.get('raisindb:visitor:website:/agents/site:conversation')).toBe('vchat-new');
  });
});

describe('direct event listeners survive a reconnect restore', () => {
  it('are not re-subscribed as filter subscriptions', async () => {
    const sendRequest = vi.fn(async () => ({ subscription_id: 'new' }));
    const events = new EventHandler(sendRequest as never);
    handlers.push(events);
    const got: string[] = [];
    events.addFlowEventListener('visitor-chat:vchat-1', (e) => got.push(e.event_type));
    await events.restoreSubscriptions();
    expect(sendRequest).not.toHaveBeenCalled();
    events.handleEvent(event('visitor-chat:vchat-1', { type: 'text_chunk', text: 'x' }));
    expect(got).toEqual(['text_chunk']);
  });
});

describe('ConversationStore without login', () => {
  it('starts a visitor conversation on the first message and streams the reply into its state', async () => {
    const { ConversationStore } = await import('./stores/conversation-store');
    let events!: EventHandler;
    const send = vi.fn(async (_p: unknown, type: string) => {
      if (type === 'visitor_chat_start') return startResponse('vchat-9');
      queueMicrotask(() => {
        events.handleEvent(event('visitor-chat:vchat-9', { type: 'text_chunk', text: 'Moin', timestamp: 't' }));
        events.handleEvent(
          event('visitor-chat:vchat-9', {
            type: 'done', conversationPath: 'x', content: 'Moin', role: 'assistant', timestamp: 't',
          }),
        );
      });
      return { accepted: true, message_id: 'm1' };
    });
    const s = setup(send);
    events = s.events;
    handlers.push(events);
    const store = new ConversationStore({
      database: { conversations: s.manager } as never,
      createOptions: { participant: '/agents/site', anonymous: { store: false } },
      watchdogIntervalMs: 60_000,
    });
    await store.sendMessage('Hallo');
    await new Promise((r) => setTimeout(r, 0));
    const snap = store.getSnapshot();
    expect(snap.conversationPath).toBe('/visitors/k1/inbox/chats/vchat-9');
    expect(snap.isStreaming).toBe(false);
    expect(snap.messages.map((m) => [m.role, m.content])).toEqual([
      ['user', 'Hallo'],
      ['assistant', 'Moin'],
    ]);
    store.destroy();
  });
});
