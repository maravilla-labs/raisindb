/**
 * Svelte store for messages (inbox, outbox, sent).
 */
import { writable, derived, get } from 'svelte/store';
import { browser } from '$app/environment';
import { query, subscribeToPath, ACCESS_CONTROL_WORKSPACE, type MessageNode } from '$lib/raisin';
import { user } from './auth';

interface MessagesState {
  inbox: MessageNode[];
  outbox: MessageNode[];
  sent: MessageNode[];
  loading: boolean;
  error: string | null;
}

function createMessagesStore() {
  const { subscribe, set, update } = writable<MessagesState>({
    inbox: [],
    outbox: [],
    sent: [],
    loading: false,
    error: null,
  });

  let inboxUnsubscribe: (() => void) | null = null;
  let outboxUnsubscribe: (() => void) | null = null;
  let sentUnsubscribe: (() => void) | null = null;

  function getUserHomePath(): string | null {
    const currentUser = get(user);
    if (!currentUser?.home) return null;
    // user.home is like /raisin:access_control/users/abc123
    return currentUser.home.replace(`/${ACCESS_CONTROL_WORKSPACE}`, '');
  }

  async function loadInbox() {
    const homePath = getUserHomePath();
    if (!homePath) return;

    try {
      const inboxPath = `${homePath}/inbox`;
      const messages = await query<MessageNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE DESCENDANT_OF('${inboxPath}')
          AND node_type = 'raisin:Message'
        ORDER BY properties->>'created_at' DESC
      `);
      update((s) => ({ ...s, inbox: messages }));
    } catch (err) {
      console.error('[messages] Failed to load inbox:', err);
    }
  }

  async function loadOutbox() {
    const homePath = getUserHomePath();
    if (!homePath) return;

    try {
      const outboxPath = `${homePath}/outbox`;
      const messages = await query<MessageNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE CHILD_OF('${outboxPath}')
          AND node_type = 'raisin:Message'
        ORDER BY properties->>'created_at' DESC
      `);
      update((s) => ({ ...s, outbox: messages }));
    } catch (err) {
      console.error('[messages] Failed to load outbox:', err);
    }
  }

  async function loadSent() {
    const homePath = getUserHomePath();
    if (!homePath) return;

    try {
      const sentPath = `${homePath}/sent`;
      const messages = await query<MessageNode>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL_WORKSPACE}'
        WHERE CHILD_OF('${sentPath}')
          AND node_type = 'raisin:Message'
        ORDER BY properties->>'created_at' DESC
      `);
      update((s) => ({ ...s, sent: messages }));
    } catch (err) {
      console.error('[messages] Failed to load sent:', err);
    }
  }

  async function setupSubscriptions() {
    if (!browser) return;

    const homePath = getUserHomePath();
    if (!homePath) return;

    try {
      // Subscribe to inbox changes
      inboxUnsubscribe = await subscribeToPath(`${homePath}/inbox`, () => {
        loadInbox();
      });

      // Subscribe to outbox changes
      outboxUnsubscribe = await subscribeToPath(`${homePath}/outbox`, () => {
        loadOutbox();
      });

      // Subscribe to sent changes
      sentUnsubscribe = await subscribeToPath(`${homePath}/sent`, () => {
        loadSent();
      });
    } catch (err) {
      console.error('[messages] Failed to setup subscriptions:', err);
    }
  }

  function cleanup() {
    inboxUnsubscribe?.();
    outboxUnsubscribe?.();
    sentUnsubscribe?.();
    inboxUnsubscribe = null;
    outboxUnsubscribe = null;
    sentUnsubscribe = null;
  }

  return {
    subscribe,

    async init() {
      update((s) => ({ ...s, loading: true, error: null }));

      try {
        await Promise.all([loadInbox(), loadOutbox(), loadSent()]);
        await setupSubscriptions();
        update((s) => ({ ...s, loading: false }));
      } catch (err) {
        console.error('[messages] Init error:', err);
        update((s) => ({
          ...s,
          loading: false,
          error: err instanceof Error ? err.message : 'Failed to load messages',
        }));
      }
    },

    async refresh() {
      await Promise.all([loadInbox(), loadOutbox(), loadSent()]);
    },

    reset() {
      cleanup();
      set({
        inbox: [],
        outbox: [],
        sent: [],
        loading: false,
        error: null,
      });
    },

    cleanup,
  };
}

export const messagesStore = createMessagesStore();

// Derived stores
export const inbox = derived(messagesStore, ($s) => $s.inbox);
export const outbox = derived(messagesStore, ($s) => $s.outbox);
export const sent = derived(messagesStore, ($s) => $s.sent);
export const messagesLoading = derived(messagesStore, ($s) => $s.loading);
export const messagesError = derived(messagesStore, ($s) => $s.error);
