/**
 * Inbox Store — Svelte 5 rune-based wrapper around SDK ConversationListStore.
 *
 * Uses $state runes for fine-grained reactivity. Components access properties
 * directly (e.g., `inbox.conversations`) instead of the Svelte 4 `$store` syntax.
 *
 * @example
 * ```svelte
 * <script>
 *   import { inbox } from '$lib/stores/inbox-store.svelte';
 *   const convos = $derived(inbox.conversations);
 * </script>
 *
 * {#each convos as conv}...{/each}
 * <p>{inbox.unreadCount} unread</p>
 * ```
 */
import {
  ConversationListStore,
  type ConversationListSnapshot,
  type ConversationListItem,
  type Database,
  type ConversationType,
} from '@raisindb/client';

class InboxState {
  #store: ConversationListStore | null = null;
  #unsubscribe: (() => void) | null = null;

  snapshot = $state<ConversationListSnapshot>({
    conversations: [],
    totalUnreadCount: 0,
    isLoading: false,
    error: null,
  });

  // ---------------------------------------------------------------------------
  // Derived getters — reactive because they read from $state snapshot
  // ---------------------------------------------------------------------------

  get conversations() { return this.snapshot.conversations; }
  get unreadCount() { return this.snapshot.totalUnreadCount; }
  get isLoading() { return this.snapshot.isLoading; }
  get error() { return this.snapshot.error; }

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  /** Initialize the inbox. Call once after authentication. */
  async init(db: Database, type?: ConversationType): Promise<void> {
    this.destroy();
    this.#store = new ConversationListStore({
      database: db,
      type,
      realtime: true,
    });
    this.#unsubscribe = this.#store.subscribe(s => { this.snapshot = s; });
    await this.#store.load();
  }

  /** Create a new conversation. */
  async createConversation(
    participant: string,
    subject?: string,
  ): Promise<ConversationListItem | undefined> {
    return this.#store?.createConversation({ participant, subject });
  }

  /** Delete a conversation. */
  async deleteConversation(path: string): Promise<void> {
    await this.#store?.deleteConversation(path);
  }

  /** Mark a conversation as read (optimistic). */
  async markAsRead(path: string): Promise<void> {
    await this.#store?.markAsRead(path);
  }

  /** Get a cached ConversationStore for a specific conversation path. */
  getConversationStore(path: string) {
    return this.#store?.getConversationStore(path) ?? null;
  }

  /** Tear down the store. Call on logout or component destroy. */
  destroy(): void {
    this.#unsubscribe?.();
    this.#unsubscribe = null;
    this.#store?.destroy();
    this.#store = null;
    this.snapshot = {
      conversations: [],
      totalUnreadCount: 0,
      isLoading: false,
      error: null,
    };
  }
}

/** Singleton inbox state. */
export const inbox = new InboxState();
