/**
 * Unit tests for the Vue 3 integration.
 *
 * Uses a fake VueLike object (plain refs, getter-based computed, collected
 * onUnmounted callbacks) so the pure snapshot-binding logic is tested
 * without Vue itself — same approach as the React/Svelte adapter tests.
 */
import { describe, expect, it, vi } from 'vitest';
import { createRaisinVue } from './index';
import type { VueLike } from './types';
import type { RaisinClient } from '../../client';
import type { Database } from '../../database';
import type { ChatEvent } from '../../types/chat';

// ---------------------------------------------------------------------------
// Fake Vue
// ---------------------------------------------------------------------------

function createFakeVue() {
  const unmountCallbacks: (() => void)[] = [];
  const vue: VueLike = {
    ref: (value: unknown) => ({ value }),
    computed: <T>(getter: () => T) => ({
      get value() { return getter(); },
    }),
    onUnmounted: (fn: () => void) => { unmountCallbacks.push(fn); },
  };
  return {
    vue,
    unmount: () => {
      for (const fn of unmountCallbacks) fn();
      unmountCallbacks.length = 0;
    },
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

// ---------------------------------------------------------------------------
// useAuth
// ---------------------------------------------------------------------------

describe('vue useAuth', () => {
  function fakeAuthClient() {
    let authCallback: ((change: any) => void) | null = null;
    const client = {
      getStoredUser: () => null,
      onAuthStateChange: (cb: (change: any) => void) => {
        authCallback = cb;
        return () => { authCallback = null; };
      },
      onUserChange: () => () => {},
      loginWithEmail: vi.fn().mockResolvedValue({ email: 'a@b.c' }),
    } as unknown as RaisinClient;
    return { client, signIn: (user: any) => authCallback?.({ event: 'SIGNED_IN', session: { user } }) };
  }

  it('exposes reactive auth state that updates on SIGNED_IN', () => {
    const { vue } = createFakeVue();
    const { useAuth } = createRaisinVue(vue);
    const { client, signIn } = fakeAuthClient();

    const auth = useAuth(client);
    expect(auth.user.value).toBeNull();
    expect(auth.isAuthenticated.value).toBe(false);

    signIn({ email: 'alice@example.com' });
    expect(auth.user.value).toEqual({ email: 'alice@example.com' });
    expect(auth.isAuthenticated.value).toBe(true);
  });

  it('stops reacting after unmount', () => {
    const { vue, unmount } = createFakeVue();
    const { useAuth } = createRaisinVue(vue);
    const { client, signIn } = fakeAuthClient();

    const auth = useAuth(client);
    unmount();

    signIn({ email: 'late@example.com' });
    expect(auth.user.value).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// useConnection
// ---------------------------------------------------------------------------

describe('vue useConnection', () => {
  it('tracks connection and ready state changes', () => {
    const { vue } = createFakeVue();
    const { useConnection } = createRaisinVue(vue);

    let stateCallback: ((state: string) => void) | null = null;
    let readyCallback: ((ready: boolean) => void) | null = null;
    const client = {
      getConnectionState: () => 'disconnected',
      isReady: () => false,
      onConnectionStateChange: (cb: (state: string) => void) => {
        stateCallback = cb;
        return () => {};
      },
      onReadyStateChange: (cb: (ready: boolean) => void) => {
        readyCallback = cb;
        return () => {};
      },
    } as unknown as RaisinClient;

    const conn = useConnection(client);
    expect(conn.isConnected.value).toBe(false);
    expect(conn.isReady.value).toBe(false);

    stateCallback!('connected');
    readyCallback!(true);
    expect(conn.state.value).toBe('connected');
    expect(conn.isConnected.value).toBe(true);
    expect(conn.isReady.value).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// useSql
// ---------------------------------------------------------------------------

describe('vue useSql', () => {
  it('loads data and exposes it reactively', async () => {
    const { vue } = createFakeVue();
    const { useSql } = createRaisinVue(vue);

    const database = {
      executeSql: vi.fn().mockResolvedValue({ rows: [{ title: 'Hello' }] }),
    } as unknown as Database;

    const result = useSql<{ title: string }>(database, "SELECT * FROM 'content'");
    expect(result.isLoading.value).toBe(true);

    await flush();
    expect(result.data.value).toEqual([{ title: 'Hello' }]);
    expect(result.isLoading.value).toBe(false);
    expect(result.error.value).toBeNull();
  });

  it('captures query errors', async () => {
    const { vue } = createFakeVue();
    const { useSql } = createRaisinVue(vue);

    const database = {
      executeSql: vi.fn().mockRejectedValue(new Error('boom')),
    } as unknown as Database;

    const result = useSql(database, 'SELECT 1');
    await flush();
    expect(result.error.value?.message).toBe('boom');
    expect(result.data.value).toBeNull();
  });

  it('refetches on demand', async () => {
    const { vue } = createFakeVue();
    const { useSql } = createRaisinVue(vue);

    const executeSql = vi.fn().mockResolvedValue({ rows: [] });
    const database = { executeSql } as unknown as Database;

    const result = useSql(database, 'SELECT 1');
    await flush();
    expect(executeSql).toHaveBeenCalledTimes(1);

    await result.refetch();
    expect(executeSql).toHaveBeenCalledTimes(2);
  });
});

// ---------------------------------------------------------------------------
// useSubscription
// ---------------------------------------------------------------------------

describe('vue useSubscription', () => {
  function fakeEventsDatabase() {
    let eventCallback: ((event: unknown) => void) | null = null;
    const unsubscribe = vi.fn();
    const database = {
      events: () => ({
        subscribe: (_filters: unknown, cb: (event: unknown) => void) => {
          eventCallback = cb;
          return Promise.resolve({ unsubscribe });
        },
      }),
    } as unknown as Database;
    return { database, emit: (event: unknown) => eventCallback?.(event), unsubscribe };
  }

  it('delivers events to the callback', async () => {
    const { vue } = createFakeVue();
    const { useSubscription } = createRaisinVue(vue);
    const { database, emit } = fakeEventsDatabase();

    const received: unknown[] = [];
    useSubscription(database, { workspace: 'content' }, (event) => received.push(event));
    await flush();

    emit({ event_type: 'node:created' });
    expect(received).toHaveLength(1);
  });

  it('cleans up the server subscription on unmount and stops delivering events', async () => {
    const { vue, unmount } = createFakeVue();
    const { useSubscription } = createRaisinVue(vue);
    const { database, emit, unsubscribe } = fakeEventsDatabase();

    const received: unknown[] = [];
    useSubscription(database, { workspace: 'content' }, (event) => received.push(event));
    await flush();

    unmount();
    expect(unsubscribe).toHaveBeenCalled();

    emit({ event_type: 'node:created' });
    expect(received).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// useConversation
// ---------------------------------------------------------------------------

describe('vue useConversation', () => {
  function fakeConversationDatabase() {
    let eventHandler: ((event: ChatEvent) => void) | null = null;
    const unsubscribe = vi.fn();
    const database = {
      conversations: {
        subscribe: (_path: string, onEvent: (event: ChatEvent) => void) => {
          eventHandler = onEvent;
          return { unsubscribe, waitUntilConnected: async () => {} };
        },
        getMessages: vi.fn().mockResolvedValue([]),
        getActiveToolCalls: vi.fn().mockResolvedValue([]),
      },
    } as unknown as Database;
    return { database, emit: (event: ChatEvent) => eventHandler?.(event), unsubscribe };
  }

  it('mirrors streaming events into reactive state', () => {
    const { vue } = createFakeVue();
    const { useConversation } = createRaisinVue(vue);
    const { database, emit } = fakeConversationDatabase();

    const chat = useConversation({ database, conversationPath: '/users/u/inbox/chats/c1' });
    expect(chat.messages.value).toEqual([]);
    expect(chat.conversationPath.value).toBe('/users/u/inbox/chats/c1');

    emit({ type: 'text_chunk', text: 'Hel', timestamp: 't' } as ChatEvent);
    emit({ type: 'text_chunk', text: 'lo', timestamp: 't' } as ChatEvent);
    expect(chat.streamingText.value).toBe('Hello');

    emit({
      type: 'assistant_message',
      message: { role: 'assistant', content: 'Hello', timestamp: 't' },
      timestamp: 't',
    } as ChatEvent);
    expect(chat.messages.value).toHaveLength(1);
    expect(chat.streamingText.value).toBe('');
  });

  it('tracks tool calls through their lifecycle', () => {
    const { vue } = createFakeVue();
    const { useConversation } = createRaisinVue(vue);
    const { database, emit } = fakeConversationDatabase();

    const chat = useConversation({ database, conversationPath: '/users/u/inbox/chats/c1' });

    emit({
      type: 'tool_call_started',
      toolCallId: 'tc1',
      functionName: 'get_weather',
      arguments: { city: 'Basel' },
      timestamp: 't',
    } as ChatEvent);
    expect(chat.activeToolCalls.value).toHaveLength(1);
    expect(chat.activeToolCalls.value[0].status).toBe('running');

    emit({
      type: 'tool_call_completed',
      toolCallId: 'tc1',
      result: { temp: 21 },
      durationMs: 12,
      timestamp: 't',
    } as ChatEvent);
    expect(chat.activeToolCalls.value[0].status).toBe('completed');
  });

  it('destroys the store and SSE subscription on unmount', () => {
    const { vue, unmount } = createFakeVue();
    const { useConversation } = createRaisinVue(vue);
    const { database, emit, unsubscribe } = fakeConversationDatabase();

    const chat = useConversation({ database, conversationPath: '/users/u/inbox/chats/c1' });
    unmount();
    expect(unsubscribe).toHaveBeenCalled();

    emit({ type: 'text_chunk', text: 'late', timestamp: 't' } as ChatEvent);
    expect(chat.streamingText.value).toBe('');
  });
});

// ---------------------------------------------------------------------------
// useConversationList
// ---------------------------------------------------------------------------

describe('vue useConversationList', () => {
  it('loads conversations on creation and computes unread counts', async () => {
    const { vue } = createFakeVue();
    const { useConversationList } = createRaisinVue(vue);

    const items = [
      { id: '1', type: 'ai_chat', conversationPath: '/c/1', conversationWorkspace: 'w', unreadCount: 2 },
      { id: '2', type: 'ai_chat', conversationPath: '/c/2', conversationWorkspace: 'w', unreadCount: 1 },
    ];
    const database = {
      conversations: { list: vi.fn().mockResolvedValue(items) },
    } as unknown as Database;

    const list = useConversationList({ database });
    await flush();

    expect(list.conversations.value).toHaveLength(2);
    expect(list.totalUnreadCount.value).toBe(3);
    expect(list.isLoading.value).toBe(false);
  });

  it('delegates createConversation to the store and prepends the new item', async () => {
    const { vue } = createFakeVue();
    const { useConversationList } = createRaisinVue(vue);

    const create = vi.fn().mockResolvedValue({
      id: 'new',
      type: 'ai_chat',
      conversationPath: '/c/new',
      conversationWorkspace: 'w',
    });
    const database = {
      conversations: { list: vi.fn().mockResolvedValue([]), create },
    } as unknown as Database;

    const list = useConversationList({ database });
    await flush();

    const item = await list.createConversation({ participant: '/agents/support' });
    expect(create).toHaveBeenCalledWith({ participant: '/agents/support' });
    expect(item.conversationPath).toBe('/c/new');
    expect(list.conversations.value[0].conversationPath).toBe('/c/new');
  });
});
