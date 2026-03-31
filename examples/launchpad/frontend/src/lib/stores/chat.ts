/**
 * Chat widget UI state management.
 *
 * This store ONLY manages UI state for the chat widget:
 * - Which conversations are open (popup windows)
 * - Which conversations are minimized
 * - Whether the conversation list is open
 *
 * All data (conversations, messages, unread counts) comes from messagingStore.
 */
import { writable, derived, get } from 'svelte/store';
import { messagingStore } from './messaging-store';

const MAX_OPEN_CONVERSATIONS = 3;

interface ChatUIState {
  isListOpen: boolean;
  openConversations: string[];       // conversation_ids (max 3)
  minimizedConversations: string[];  // conversation_ids that are minimized
}

const initialState: ChatUIState = {
  isListOpen: false,
  openConversations: [],
  minimizedConversations: [],
};

function createChatStore() {
  const { subscribe, set, update } = writable<ChatUIState>(initialState);

  return {
    subscribe,

    /**
     * Initialize the chat UI store.
     * Called when user logs in - delegates data loading to messagingStore.
     */
    async init() {
      // Data initialization is handled by messagingStore
      // This method exists for API compatibility
    },

    /**
     * Toggle the conversation list dropdown.
     */
    toggleList() {
      update(state => ({ ...state, isListOpen: !state.isListOpen }));
    },

    /**
     * Close the conversation list dropdown.
     */
    closeList() {
      update(state => ({ ...state, isListOpen: false }));
    },

    /**
     * Open a conversation popup.
     * If already open, bring it to focus (un-minimize).
     * If max conversations reached, close the oldest one.
     */
    async openConversation(conversationId: string) {
      const state = get({ subscribe });

      // If already open, just make sure it's not minimized
      if (state.openConversations.includes(conversationId)) {
        update(s => ({
          ...s,
          minimizedConversations: s.minimizedConversations.filter(id => id !== conversationId),
          isListOpen: false
        }));
        return;
      }

      // Load messages for this conversation (lazy loading)
      await messagingStore.loadConversationMessages(conversationId);

      // Update UI state
      update(s => {
        const newOpen = [...s.openConversations];

        // If at max, remove the oldest
        if (newOpen.length >= MAX_OPEN_CONVERSATIONS) {
          newOpen.shift();
        }

        newOpen.push(conversationId);

        return {
          ...s,
          openConversations: newOpen,
          minimizedConversations: s.minimizedConversations.filter(id => id !== conversationId),
          isListOpen: false
        };
      });

      // Mark as read
      await messagingStore.markAsRead(conversationId);
    },

    /**
     * Close a conversation popup.
     */
    closeConversation(conversationId: string) {
      update(state => ({
        ...state,
        openConversations: state.openConversations.filter(id => id !== conversationId),
        minimizedConversations: state.minimizedConversations.filter(id => id !== conversationId)
      }));
    },

    /**
     * Minimize a conversation popup.
     */
    minimizeConversation(conversationId: string) {
      update(state => {
        if (!state.minimizedConversations.includes(conversationId)) {
          return {
            ...state,
            minimizedConversations: [...state.minimizedConversations, conversationId]
          };
        }
        return state;
      });
    },

    /**
     * Restore a minimized conversation.
     */
    restoreConversation(conversationId: string) {
      update(state => ({
        ...state,
        minimizedConversations: state.minimizedConversations.filter(id => id !== conversationId)
      }));
    },

    /**
     * Reset the store (on logout).
     */
    reset() {
      set(initialState);
    }
  };
}

export const chatStore = createChatStore();

// Derived stores for convenience
export const openConversations = derived(chatStore, $chat => $chat.openConversations);
export const minimizedConversations = derived(chatStore, $chat => $chat.minimizedConversations);
export const isListOpen = derived(chatStore, $chat => $chat.isListOpen);
