/**
 * Unified Messaging Store - Single Source of Truth
 *
 * This store manages all messaging data (conversations, messages) and provides:
 * - Real-time updates via WebSocket subscriptions with surgical updates
 * - Optimistic UI for sent messages
 * - Auto-refresh on reconnection
 * - Lazy loading of conversation messages
 *
 * Event handling strategy:
 * - WebSocket events contain node_id
 * - Fast SQL lookup by ID to get the changed node
 * - Surgical insert/update in store (no invalidateAll, no full refresh)
 *
 * All components (ChatList, ChatPopup, Inbox) read from this store.
 */
import { writable, derived, get } from 'svelte/store';
import { browser } from '$app/environment';
import {
  getDatabase,
  onReconnected,
  query,
} from '$lib/raisin';
import { user } from './auth';
import {
  getConversations,
  getConversationMessages,
  getUnreadConversationCount,
  sendReply,
  markConversationAsRead as markConvAsReadApi,
  markDirectMessageAsRead,
  type Conversation,
  type Message,
} from './messaging';

const ACCESS_CONTROL = 'raisin:access_control';

// ============================================================================
// Types
// ============================================================================

interface MessagingState {
  // Conversation metadata (always loaded)
  conversations: Map<string, Conversation>;

  // Messages per conversation (lazy loaded when conversation opened)
  messages: Map<string, Message[]>;

  // Flat list of all inbox messages (for "All Messages" tab)
  inboxMessages: Message[];

  // Track which conversations have their messages loaded
  loadedConversations: Set<string>;

  // Aggregates
  unreadCount: number;

  // UI state
  loading: boolean;
  error: string | null;

  // Subscription state
  subscribed: boolean;
  initialized: boolean;
}

// ============================================================================
// Store Implementation
// ============================================================================

const initialState: MessagingState = {
  conversations: new Map(),
  messages: new Map(),
  inboxMessages: [],
  loadedConversations: new Set(),
  unreadCount: 0,
  loading: false,
  error: null,
  subscribed: false,
  initialized: false,
};

let unsubscribeEvents: (() => void) | null = null;
let unsubscribeReconnected: (() => void) | null = null;

// Debouncing infrastructure for markAsRead
const pendingMarkAsRead = new Set<string>();
const markAsReadDebounceTimers = new Map<string, ReturnType<typeof setTimeout>>();
const MARK_AS_READ_DEBOUNCE_MS = 300;

function createMessagingStore() {
  const { subscribe, set, update } = writable<MessagingState>({ ...initialState });

  /**
   * Parse event path to extract conversation ID and determine event type.
   * Path patterns:
   *   /users/{userId}/inbox/chats/{convId}              -> conversation node
   *   /users/{userId}/inbox/chats/{convId}/msg-{id}     -> message node
   *   /users/{userId}/inbox/requests/req-{id}           -> request node
   *   /users/{userId}/inbox/notifications/notif-{id}    -> notification node
   */
  function parseEventPath(path: string): { convId: string | null; isMessage: boolean; isConversation: boolean; isInboxItem: boolean } {
    const parts = path.split('/');
    const inboxIndex = parts.indexOf('inbox');

    if (inboxIndex === -1 || parts.length <= inboxIndex + 1) {
      return { convId: null, isMessage: false, isConversation: false, isInboxItem: false };
    }

    const bucket = parts[inboxIndex + 1];

    if (bucket === 'chats') {
        const convId = parts[inboxIndex + 2];
        const isConversation = parts.length === inboxIndex + 3; // Exactly at convId level
        // Any child of a conversation is a message (dm-*, msg-*)
        const isMessage = parts.length > inboxIndex + 3; 
        return { convId, isMessage, isConversation, isInboxItem: false };
    } else {
        // requests or notifications
        // These are "Inbox Items"
        return { convId: null, isMessage: false, isConversation: false, isInboxItem: true };
    }
  }

  // ... (getNodeById omitted for brevity as it was unused in original) ...

  /**
   * Handle incoming WebSocket events with surgical updates.
   * Uses node data from payload (includeNode: true) for instant updates without extra queries.
   */
  function handleEvent(event: any) {
    if (!event) return;

    const payload = event.payload;
    const kind = payload?.kind as 'Created' | 'Updated' | 'Deleted';
    const path = payload?.path as string;
    const node = payload?.node; 

    if (!path) return;

    const { convId, isMessage, isConversation, isInboxItem } = parseEventPath(path);
    
    // Ignore read receipts in general stream if they happen to appear
    if (node?.properties?.message_type === 'read_receipt') return;

    const currentUser = get(user);
    if (!currentUser?.home) return;

    // Handle based on event kind - use node from payload directly
    if (kind === 'Created' && node) {
      if (isMessage && convId) {
        handleNewMessage(convId, node);
        // Also add to flat inbox list
        handleNewInboxItem(node);
      } else if (isConversation) {
        handleNewConversation(node, currentUser.id);
      } else if (isInboxItem) {
        handleNewInboxItem(node);
      }
    } else if (kind === 'Updated' && node) {
      if (isMessage && convId) {
        handleMessageUpdate(convId, node);
        handleInboxItemUpdate(node);
      } else if (isConversation) {
        handleConversationUpdate(node);
      } else if (isInboxItem) {
        handleInboxItemUpdate(node);
      }
    } else if (kind === 'Deleted') {
      if (isMessage && convId) {
        handleMessageDeleted(convId, path);
        handleInboxItemDeleted(path);
      } else if (isInboxItem) {
        handleInboxItemDeleted(path);
      }
    }
  }

  // --- Inbox Item Handlers ---

  function handleNewInboxItem(node: any) {
      // Convert to Message format
      const message: Message = {
        id: node.id,
        path: node.path,
        name: node.name,
        node_type: node.node_type,
        properties: node.properties,
      };

      update(state => ({
          ...state,
          inboxMessages: [message, ...state.inboxMessages]
      }));
  }

  function handleInboxItemUpdate(node: any) {
      update(state => ({
          ...state,
          inboxMessages: state.inboxMessages.map(m => 
              m.id === node.id ? { ...m, properties: { ...m.properties, ...node.properties } } : m
          )
      }));
  }

  function handleInboxItemDeleted(path: string) {
      update(state => ({
          ...state,
          inboxMessages: state.inboxMessages.filter(m => m.path !== path)
      }));
  }

  /**
   * Handle new message: append to messages array, update conversation preview.
   * NOTE: Does NOT increment unread count - server is source of truth.
   * The handleConversationUpdate will receive the correct unread_count from server.
   */
  function handleNewMessage(convId: string, node: any) {
    update(state => {
      // Check if we already have this message (dedup)
      const existingMessages = state.messages.get(convId) || [];
      if (existingMessages.some(m => m.id === node.id || m.path === node.path)) {
        return state;
      }

      // Remove any optimistic message that matches this client_id
      const clientId = node.properties?.client_id;
      
      const filteredMessages = existingMessages.filter(m => {
        // If message has _optimistic flag
        if ((m as any)._optimistic) {
            // Match by client_id if available (Robust)
            if (clientId && m.properties?.client_id === clientId) return false;
            // Fallback to ID match if optimistic message used same ID (unlikely here)
            if (m.id === node.id) return false;
        }
        return true;
      });

      // Construct message object
      const message: Message = {
        id: node.id,
        path: node.path,
        name: node.name,
        node_type: node.node_type,
        properties: node.properties,
      };

      // Append new message
      const newMessages = new Map(state.messages);
      newMessages.set(convId, [...filteredMessages, message]);

      // Update conversation's lastMessage preview (but NOT unread count - server handles that)
      const conversations = new Map(state.conversations);
      const conv = conversations.get(convId);
      if (conv) {
        conversations.set(convId, {
          ...conv,
          // Don't increment unreadCount here - let handleConversationUpdate set it from server
          lastMessage: message,
          lastMessageAt: message.properties?.created_at || new Date().toISOString(),
        });
      }

      return {
        ...state,
        messages: newMessages,
        conversations,
        // Don't change unreadCount - let handleConversationUpdate handle it
      };
    });
  }

  // ... (handleNewConversation, handleMessageUpdate, handleConversationUpdate, handleMessageDeleted same as before) ...
  /**
   * Handle new conversation node created.
   */
  function handleNewConversation(node: any, currentUserId: string) {
    if (!node?.properties) return;

    // Build conversation object from node
    const participants = node.properties.participants || [];
    const otherId = participants.find((p: string) => p !== currentUserId) || participants[0];
    const lm = node.properties.last_message;

    // Get other participant's details - NOT the last message sender
    const otherDetails = node.properties.participant_details?.[otherId];
    const otherDisplayName = otherDetails?.display_name ||
                             (lm?.sender_id === otherId ? lm?.sender_display_name : lm?.recipient_display_name) ||
                             otherId ||
                             'User';

    const conversation: Conversation = {
      id: node.name,
      participantId: otherId,
      participantDisplayName: otherDisplayName,
      lastMessage: lm ? {
        id: 'latest',
        path: node.path,
        name: 'latest',
        node_type: 'raisin:Message',
        properties: {
          message_type: 'chat',
          status: 'read',
          created_at: lm.created_at,
          body: {
            message_text: lm.content || lm.message_text || '',
            content: lm.content || lm.message_text || '',
            sender_display_name: lm.sender_display_name,
          },
        },
      } : undefined,
      lastMessageAt: node.properties.updated_at || new Date().toISOString(),
      unreadCount: node.properties.unread_count || 0,
    };

    update(state => {
      const conversations = new Map(state.conversations);
      conversations.set(conversation.id, conversation);
      return { ...state, conversations };
    });
  }

  function handleMessageUpdate(convId: string, node: any) {
    if (!node?.id) return;

    update(state => {
      const messages = state.messages.get(convId);
      if (!messages) return state;

      const newMessages = messages.map(m =>
        m.id === node.id ? { ...m, properties: { ...m.properties, ...node.properties } } : m
      );

      const newMessagesMap = new Map(state.messages);
      newMessagesMap.set(convId, newMessages);

      return { ...state, messages: newMessagesMap };
    });
  }

  function handleConversationUpdate(node: any) {
    if (!node?.name) return;

    update(state => {
      const conv = state.conversations.get(node.name);
      if (!conv) return state;

      const conversations = new Map(state.conversations);
      conversations.set(node.name, {
        ...conv,
        unreadCount: node.properties?.unread_count ?? conv.unreadCount,
        lastMessageAt: node.properties?.updated_at ?? conv.lastMessageAt,
      });

      // Recalculate total unread
      let unreadCount = 0;
      conversations.forEach(c => { unreadCount += c.unreadCount; });

      return { ...state, conversations, unreadCount };
    });
  }

  function handleMessageDeleted(convId: string, path: string) {
    update(state => {
      const messages = state.messages.get(convId);
      if (!messages) return state;

      const newMessages = messages.filter(m => m.path !== path);
      const newMessagesMap = new Map(state.messages);
      newMessagesMap.set(convId, newMessages);

      return { ...state, messages: newMessagesMap };
    });
  }

  // ... (refreshConversations, reloadConversationMessages) ...
  async function refreshConversations() {
    try {
      const [conversations, unread] = await Promise.all([
        getConversations(),
        getUnreadConversationCount(),
      ]);

      const conversationsMap = new Map<string, Conversation>();
      for (const conv of conversations) {
        conversationsMap.set(conv.id, conv);
      }

      update(s => ({
        ...s,
        conversations: conversationsMap,
        unreadCount: unread,
      }));
    } catch (err) {
      console.error('[messaging-store] Failed to refresh conversations:', err);
    }
  }

  async function reloadConversationMessages(convId: string) {
    try {
      const messages = await getConversationMessages(convId);
      update(s => {
        const newMessages = new Map(s.messages);
        newMessages.set(convId, messages);
        return { ...s, messages: newMessages };
      });
    } catch (err) {
      console.error('[messaging-store] Failed to reload messages:', err);
    }
  }

  /**
   * Setup WebSocket subscription for ALL inbox events.
   */
  async function setupSubscription() {
    const currentUser = get(user);
    if (!currentUser?.home) {
      return;
    }

    const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
    const inboxPath = `${homePath}/inbox`;

    try {
      const db = await getDatabase();
      const ws = db.workspace(ACCESS_CONTROL);

      // Subscribe to all changes in INBOX bucket (recursive) with includeNode for instant updates
      // This covers: inbox/chats, inbox/requests, inbox/notifications
      const subscription = await ws.events().subscribeToPath(
        `${inboxPath}/**`,
        (event) => {
          try {
            handleEvent(event);
          } catch (err) {
            console.error('[messaging-store] Error handling event:', err);
          }
        },
        { includeNode: true }
      );

      unsubscribeEvents = () => subscription.unsubscribe();
      update(state => ({ ...state, subscribed: true }));
    } catch (err) {
      console.error('[messaging-store] Subscription failed:', err);
    }
  }

  /**
   * Setup reconnection listener.
   *
   * The SDK now automatically handles:
   * 1. WebSocket reconnection with exponential backoff
   * 2. Re-authentication with stored token
   * 3. Restoration of all event subscriptions
   *
   * We only need to refresh our data queries here.
   */
  function setupReconnectedListener() {
    unsubscribeReconnected = onReconnected(() => {
      console.log('[messaging-store] Reconnected, syncing data...');
      syncAfterReconnect();
    });
  }

  async function syncAfterReconnect() {
    const state = get({ subscribe });

    // Refresh conversations
    const freshConversations = await getConversations();
    const freshUnread = await getUnreadConversationCount();

    const conversationsMap = new Map<string, Conversation>();
    for (const conv of freshConversations) {
      conversationsMap.set(conv.id, conv);
    }

    // Reload messages for conversations that were loaded
    const newMessages = new Map<string, Message[]>();
    for (const convId of state.loadedConversations) {
      const messages = await getConversationMessages(convId);
      newMessages.set(convId, messages);
    }

    update(s => ({
      ...s,
      conversations: conversationsMap,
      messages: newMessages,
      unreadCount: freshUnread,
    }));
  }

  return {
    subscribe,

    /**
     * Initialize the store: load conversations, setup subscriptions.
     * Call this once when user is authenticated.
     */
    async init() {
      const state = get({ subscribe });
      if (state.initialized) return;
      if (!browser) return;

      update(s => ({ ...s, loading: true, error: null }));

      try {
        const currentUser = get(user);
        if (!currentUser?.home) {
          // User not fully loaded yet, skip initialization
          update(s => ({ ...s, loading: false }));
          return;
        }
        const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
        const inboxPath = `${homePath}/inbox`;

        // Load initial data
        const [conversations, unreadCount, inboxItems] = await Promise.all([
          getConversations(),
          getUnreadConversationCount(),
          // Load generic inbox items for "All Messages"
          query<Message>(`
            SELECT id, path, name, node_type, properties
            FROM '${ACCESS_CONTROL}'
            WHERE DESCENDANT_OF('${inboxPath}')
              AND node_type = 'raisin:Message'
              AND properties->>'message_type'::STRING NOT IN ('read_receipt')
            ORDER BY properties->>'created_at' DESC
          `)
        ]);

        const conversationsMap = new Map<string, Conversation>();
        for (const conv of conversations) {
          conversationsMap.set(conv.id, conv);
        }

        update(s => ({
          ...s,
          conversations: conversationsMap,
          unreadCount,
          inboxMessages: inboxItems,
          loading: false,
          initialized: true,
        }));

        // Setup subscriptions and reconnection listener
        await setupSubscription();
        setupReconnectedListener();
      } catch (err) {
        console.error('[messaging-store] Init failed:', err);
        update(s => ({
          ...s,
          loading: false,
          error: 'Failed to load conversations',
        }));
      }
    },

    /**
     * Ensure store is initialized (idempotent).
     */
    async ensureInitialized() {
      const state = get({ subscribe });
      if (!state.initialized) {
        await this.init();
      }
    },

    /**
     * Load messages for a specific conversation (lazy loading).
     */
    async loadConversationMessages(conversationId: string) {
      const state = get({ subscribe });

      // If already loaded, just return current messages
      if (state.loadedConversations.has(conversationId)) {
        return state.messages.get(conversationId) || [];
      }

      try {
        const messages = await getConversationMessages(conversationId);

        update(s => {
          const newMessages = new Map(s.messages);
          newMessages.set(conversationId, messages);

          const newLoaded = new Set(s.loadedConversations);
          newLoaded.add(conversationId);

          return {
            ...s,
            messages: newMessages,
            loadedConversations: newLoaded,
          };
        });

        return messages;
      } catch (err) {
        console.error('[messaging-store] Failed to load messages:', err);
        return [];
      }
    },

    /**
     * Get messages for a conversation (sync, returns cached).
     */
    getMessages(conversationId: string): Message[] {
      const state = get({ subscribe });
      return state.messages.get(conversationId) || [];
    },

    /**
     * Send a message with optimistic UI.
     */
    async sendMessage(conversationId: string, content: string): Promise<boolean> {
      console.log('[messaging-store] sendMessage called', { conversationId, content });
      const state = get({ subscribe });
      const conversation = state.conversations.get(conversationId);

      if (!conversation) {
        console.error('[messaging-store] sendMessage: Conversation not found', conversationId);
        return false;
      }

      if (!conversation.participantId) {
        console.error('[messaging-store] sendMessage: Missing participant details', conversation);
        return false;
      }

      const currentUser = get(user);
      if (!currentUser?.home) return false;
      const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');

      const tempId = `temp-${Date.now()}`;
      const clientId = crypto.randomUUID(); // Generated client_id for robust dedup
      const now = new Date().toISOString();

      // Optimistic update - show message immediately
      const optimisticMessage: Message & { _optimistic: boolean } = {
        id: tempId,
        path: `${homePath}/outbox/${tempId}`,
        name: tempId,
        node_type: 'raisin:Message',
        properties: {
          message_type: 'chat',
          status: 'sending',
          created_at: now,
          client_id: clientId, // Store client_id in optimistic message
          sender_id: currentUser.id,
          recipient_id: conversation.participantId,
          body: {
            message_text: content,
            content,
          },
        },
        _optimistic: true,
      };

      update(s => {
        const messages = s.messages.get(conversationId) || [];
        const newMessages = new Map(s.messages);
        newMessages.set(conversationId, [...messages, optimisticMessage]);

        // Update conversation preview
        const conversations = new Map(s.conversations);
        const conv = conversations.get(conversationId);
        if (conv) {
          conversations.set(conversationId, {
            ...conv,
            lastMessage: optimisticMessage,
            lastMessageAt: now,
          });
        }

        return { ...s, messages: newMessages, conversations };
      });

      // Send to server
      try {
        console.log('[messaging-store] Sending reply via API...', {
            conversationId,
            participantId: conversation.participantId,
            content
        });
        const result = await sendReply(
          conversationId,
          conversation.participantId,
          content,
          clientId // Pass client_id to server
        );

        console.log('[messaging-store] API result:', result);

        if (!result.success) {
          // Mark as error
          update(s => {
            const messages = s.messages.get(conversationId) || [];
            const newMessages = messages.map(m =>
              m.id === tempId
                ? { ...m, properties: { ...m.properties, status: 'error' } }
                : m
            );
            const newMessagesMap = new Map(s.messages);
            newMessagesMap.set(conversationId, newMessages);
            return { ...s, messages: newMessagesMap };
          });
          return false;
        }

        // Server will send Created event which will reconcile the optimistic message using client_id
        return true;
      } catch (err) {
        console.error('[messaging-store] Send failed:', err);
        // Mark as error
        update(s => {
          const messages = s.messages.get(conversationId) || [];
          const newMessages = messages.map(m =>
            m.id === tempId
              ? { ...m, properties: { ...m.properties, status: 'error' } }
              : m
          );
          const newMessagesMap = new Map(s.messages);
          newMessagesMap.set(conversationId, newMessages);
          return { ...s, messages: newMessagesMap };
        });
        return false;
      }
    },

    /**
     * Mark a conversation as read (updates local state + sends to server).
     * Debounced to prevent excessive calls from $effect re-runs.
     */
    markAsRead(conversationId: string) {
      // Skip if already pending (API call in flight)
      if (pendingMarkAsRead.has(conversationId)) {
        return;
      }

      // Clear existing debounce timer for this conversation
      const existingTimer = markAsReadDebounceTimers.get(conversationId);
      if (existingTimer) {
        clearTimeout(existingTimer);
      }

      // Debounce: wait before actually marking as read
      markAsReadDebounceTimers.set(conversationId, setTimeout(async () => {
        markAsReadDebounceTimers.delete(conversationId);

        // Re-check unreadCount before proceeding (may have changed during debounce)
        const state = get({ subscribe });
        const conv = state.conversations.get(conversationId);
        if (!conv || conv.unreadCount === 0) {
          return;
        }

        // Mark as pending to prevent duplicate calls
        pendingMarkAsRead.add(conversationId);

        // Update local state immediately
        update(s => {
          const c = s.conversations.get(conversationId);
          if (!c || c.unreadCount === 0) return s;

          const conversations = new Map(s.conversations);
          conversations.set(conversationId, { ...c, unreadCount: 0 });

          const newUnread = s.unreadCount - c.unreadCount;

          return {
            ...s,
            conversations,
            unreadCount: Math.max(0, newUnread),
          };
        });

        // Send to server
        try {
          await markConvAsReadApi(conversationId);
        } catch (err) {
          console.error('[messaging-store] markAsRead API failed:', err);
        } finally {
          pendingMarkAsRead.delete(conversationId);
        }
      }, MARK_AS_READ_DEBOUNCE_MS));
    },

    // ... (getConversation, getConversationsArray, refresh, reset) ...
    getConversation(conversationId: string): Conversation | undefined {
      const state = get({ subscribe });
      return state.conversations.get(conversationId);
    },

    getConversationsArray(): Conversation[] {
      const state = get({ subscribe });
      return [...state.conversations.values()].sort(
        (a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime()
      );
    },

    async refresh() {
        // Reuse init logic partly
        const currentUser = get(user);
        if (!currentUser?.home) {
          return; // User not loaded yet
        }
        const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
        const inboxPath = `${homePath}/inbox`;

      try {
        const [conversations, unreadCount, inboxItems] = await Promise.all([
          getConversations(),
          getUnreadConversationCount(),
          query<Message>(`
            SELECT id, path, name, node_type, properties
            FROM '${ACCESS_CONTROL}'
            WHERE DESCENDANT_OF('${inboxPath}')
              AND node_type = 'raisin:Message'
              AND properties->>'message_type'::STRING NOT IN ('read_receipt')
            ORDER BY properties->>'created_at' DESC
          `)
        ]);

        const conversationsMap = new Map<string, Conversation>();
        for (const conv of conversations) {
          conversationsMap.set(conv.id, conv);
        }

        update(s => ({
          ...s,
          conversations: conversationsMap,
          unreadCount,
          inboxMessages: inboxItems,
        }));
      } catch (err) {
        console.error('[messaging-store] Refresh failed:', err);
      }
    },

    reset() {
      if (unsubscribeEvents) {
        unsubscribeEvents();
        unsubscribeEvents = null;
      }
      if (unsubscribeReconnected) {
        unsubscribeReconnected();
        unsubscribeReconnected = null;
      }

      set({ ...initialState });
    },
  };
}

export const messagingStore = createMessagingStore();

// ============================================================================
// Derived Stores for Component Consumption
// ============================================================================

/** All conversations as a Map */
export const conversations = derived(messagingStore, $s => $s.conversations);

/** All conversations as sorted array */
export const conversationsArray = derived(messagingStore, $s =>
  [...$s.conversations.values()].sort(
    (a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime()
  )
);

/** Messages map (conversation ID -> Message[]) */
export const messages = derived(messagingStore, $s => $s.messages);

/** Inbox messages flat list (for "All Messages" tab) */
export const inboxMessages = derived(messagingStore, $s => $s.inboxMessages);

/** Total unread count */
export const unreadCount = derived(messagingStore, $s => $s.unreadCount);

/** Loading state */
export const messagingLoading = derived(messagingStore, $s => $s.loading);

/** Error state */
export const messagingError = derived(messagingStore, $s => $s.error);

/** Whether store is initialized */
export const messagingInitialized = derived(messagingStore, $s => $s.initialized);

// Re-export types
export type { Conversation, Message } from './messaging';
