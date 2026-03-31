/**
 * Svelte 5 helpers for RaisinDB conversation stores.
 *
 * Unlike the React integration which provides hooks, the Svelte 5 integration
 * provides factory functions that return objects designed to work seamlessly
 * with Svelte 5 runes. Use these in `.svelte.ts` files with `$state`.
 *
 * The SDK stores (ConversationStore, ConversationListStore) already have a
 * `.subscribe(callback)` API compatible with Svelte's reactivity. These
 * helpers simplify the most common patterns.
 *
 * @example Svelte 5 runes (in .svelte.ts file):
 * ```typescript
 * // stores/chat.svelte.ts
 * import { ConversationStore, type ConversationStoreSnapshot, type Database } from '@raisindb/client';
 *
 * class ChatState {
 *   snapshot = $state<ConversationStoreSnapshot | null>(null);
 *   #store: ConversationStore | null = null;
 *   #unsub: (() => void) | null = null;
 *
 *   get messages() { return this.snapshot?.messages ?? []; }
 *   get isStreaming() { return this.snapshot?.isStreaming ?? false; }
 *   get streamingText() { return this.snapshot?.streamingText ?? ''; }
 *   get plans() { return this.snapshot?.plans ?? []; }
 *   get error() { return this.snapshot?.error ?? null; }
 *
 *   async open(db: Database, path: string) {
 *     this.close();
 *     this.#store = new ConversationStore({ database: db, conversationPath: path });
 *     this.#unsub = this.#store.subscribe(s => { this.snapshot = s; });
 *     await this.#store.loadMessages();
 *   }
 *
 *   sendMessage = async (content: string) => {
 *     await this.#store?.sendMessage(content);
 *   };
 *
 *   close() {
 *     this.#unsub?.();
 *     this.#store?.destroy();
 *     this.#store = null;
 *     this.snapshot = null;
 *   }
 * }
 *
 * export const chat = new ChatState();
 * ```
 *
 * @example In a .svelte component:
 * ```svelte
 * <script>
 *   import { chat } from '$lib/stores/chat.svelte';
 *
 *   // Properties are reactive — no $ prefix needed
 *   const messages = $derived(chat.messages);
 * </script>
 *
 * {#each messages as msg}
 *   <div>{msg.content}</div>
 * {/each}
 * {#if chat.isStreaming}
 *   <p class="streaming">{chat.streamingText}</p>
 * {/if}
 * ```
 *
 * @module
 */

import {
  ConversationStore,
  type ConversationStoreOptions,
  type ConversationStoreSnapshot,
} from '../stores/conversation-store';
import {
  ConversationListStore,
  type ConversationListStoreOptions,
  type ConversationListSnapshot,
} from '../stores/conversation-list-store';

/**
 * Create a conversation store adapter for Svelte 5.
 *
 * Returns an object with the store, a snapshot getter, actions, and a
 * destroy function. In a `.svelte.ts` file, bind `snapshot` to `$state`
 * to get fine-grained reactivity.
 *
 * @example
 * ```typescript
 * const adapter = createConversationAdapter({ database: db, conversationPath: path });
 * let snapshot = $state(adapter.getSnapshot());
 * adapter.subscribe(s => { snapshot = s; });
 * // ...later
 * adapter.destroy();
 * ```
 */
export function createConversationAdapter(options: ConversationStoreOptions) {
  const store = new ConversationStore(options);

  return {
    /** The underlying ConversationStore instance */
    store,

    /** Get the current snapshot */
    getSnapshot: (): ConversationStoreSnapshot => store.getSnapshot(),

    /** Subscribe to snapshot changes. Returns unsubscribe function. */
    subscribe: (cb: (s: ConversationStoreSnapshot) => void) => store.subscribe(cb),

    /** Send a user message */
    sendMessage: (content: string) => store.sendMessage(content),

    /** Load persisted messages */
    loadMessages: () => store.loadMessages(),

    /** Approve a pending plan */
    approvePlan: (planPath: string) => store.approvePlan(planPath),

    /** Reject a pending plan */
    rejectPlan: (planPath: string, feedback?: string) => store.rejectPlan(planPath, feedback),

    /** Mark a message as read */
    markMessageAsRead: (messagePath: string) => store.markMessageAsRead(messagePath),

    /** Stop streaming */
    stop: () => store.stop(),

    /** Get the conversation path */
    getConversationPath: () => store.getConversationPath(),

    /** Destroy the store and release resources */
    destroy: () => store.destroy(),
  };
}

/**
 * Create a conversation list store adapter for Svelte 5.
 *
 * @example
 * ```typescript
 * const adapter = createConversationListAdapter({ database: db, realtime: true });
 * let snapshot = $state(adapter.getSnapshot());
 * adapter.subscribe(s => { snapshot = s; });
 * await adapter.load();
 * ```
 */
export function createConversationListAdapter(options: ConversationListStoreOptions) {
  const store = new ConversationListStore(options);

  return {
    /** The underlying ConversationListStore instance */
    store,

    /** Get the current snapshot */
    getSnapshot: (): ConversationListSnapshot => store.getSnapshot(),

    /** Subscribe to snapshot changes. Returns unsubscribe function. */
    subscribe: (cb: (s: ConversationListSnapshot) => void) => store.subscribe(cb),

    /** Load conversations from the server */
    load: () => store.load(),

    /** Create a new conversation */
    createConversation: (opts: { participant: string; subject?: string; input?: Record<string, unknown> }) =>
      store.createConversation(opts),

    /** Delete a conversation */
    deleteConversation: (path: string) => store.deleteConversation(path),

    /** Mark a conversation as read */
    markAsRead: (path: string) => store.markAsRead(path),

    /** Get or create a ConversationStore for a specific path */
    getConversationStore: (path: string) => store.getConversationStore(path),

    /** Destroy the store and release resources */
    destroy: () => store.destroy(),
  };
}
