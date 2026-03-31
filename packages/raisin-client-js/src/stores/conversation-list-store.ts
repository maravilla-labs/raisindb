/**
 * Multi-conversation list store for RaisinDB.
 *
 * Manages the list of conversations: loading, creating, deleting, marking
 * as read, and optional real-time updates via WebSocket events.
 *
 * Provides a factory (`getConversationStore`) that returns cached
 * per-conversation `ConversationStore` instances.
 *
 * @example Svelte 5:
 * ```typescript
 * const listStore = new ConversationListStore({ database: db, realtime: true });
 * let snapshot = $state(listStore.getSnapshot());
 * listStore.subscribe(s => { snapshot = s; });
 * await listStore.load();
 * ```
 */

import type { Database } from '../database';
import type { ConversationType, ConversationListItem } from '../types/chat';
import { ConversationStore, type ConversationStoreOptions } from './conversation-store';
import { logger } from '../logger';

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Snapshot of the conversation list state */
export interface ConversationListSnapshot {
  conversations: ConversationListItem[];
  totalUnreadCount: number;
  isLoading: boolean;
  error: string | null;
}

/** Options for creating a ConversationListStore */
export interface ConversationListStoreOptions {
  database: Database;
  /** Filter by conversation type */
  type?: ConversationType;
  /** Subscribe to WebSocket events for new conversations */
  realtime?: boolean;
}

type Subscriber = (snapshot: ConversationListSnapshot) => void;

// ---------------------------------------------------------------------------
// ConversationListStore
// ---------------------------------------------------------------------------

export class ConversationListStore {
  private database: Database;
  private type?: ConversationType;
  private realtime: boolean;
  private subscribers = new Set<Subscriber>();

  // Internal state
  private _conversations: ConversationListItem[] = [];
  private _isLoading = false;
  private _error: string | null = null;

  // Cached ConversationStore instances
  private _stores = new Map<string, ConversationStore>();

  // Real-time subscription
  private _realtimeSubscription: { unsubscribe: () => void } | null = null;

  constructor(options: ConversationListStoreOptions) {
    this.database = options.database;
    this.type = options.type;
    this.realtime = options.realtime ?? false;
  }

  // ==========================================================================
  // Subscription
  // ==========================================================================

  subscribe(callback: Subscriber): () => void {
    this.subscribers.add(callback);
    callback(this.getSnapshot());
    return () => { this.subscribers.delete(callback); };
  }

  getSnapshot(): ConversationListSnapshot {
    return {
      conversations: [...this._conversations],
      totalUnreadCount: this._conversations.reduce((sum, c) => sum + (c.unreadCount ?? 0), 0),
      isLoading: this._isLoading,
      error: this._error,
    };
  }

  // ==========================================================================
  // Actions
  // ==========================================================================

  /**
   * Load conversations from the server.
   */
  async load(): Promise<void> {
    this._isLoading = true;
    this._error = null;
    this.notify();

    try {
      const conversations = await this.database.conversations.list({
        type: this.type,
      });
      logger.debug(`[ConversationListStore] Loaded ${conversations.length} conversations`);
      this._conversations = conversations;

      if (this.realtime) {
        this.startRealtimeSubscription();
      }
    } catch (err) {
      logger.error('[ConversationListStore] Failed to load conversations', err);
      this._error = err instanceof Error ? err.message : String(err);
    } finally {
      this._isLoading = false;
      this.notify();
    }
  }

  /**
   * Create a new conversation.
   */
  async createConversation(options: {
    participant: string;
    subject?: string;
    input?: Record<string, unknown>;
  }): Promise<ConversationListItem> {
    const convo = await this.database.conversations.create(options);

    const listItem: ConversationListItem = {
      id: convo.id,
      type: convo.type,
      conversationPath: convo.conversationPath,
      conversationWorkspace: convo.conversationWorkspace,
      agentRef: convo.agentRef,
      participants: convo.participants,
      unreadCount: 0,
      updatedAt: new Date().toISOString(),
    };

    this._conversations = [listItem, ...this._conversations];
    this.notify();
    return listItem;
  }

  /**
   * Delete a conversation.
   */
  async deleteConversation(conversationPath: string): Promise<void> {
    // Destroy cached store
    const store = this._stores.get(conversationPath);
    if (store) {
      store.destroy();
      this._stores.delete(conversationPath);
    }

    await this.database.conversations.delete(conversationPath);
    this._conversations = this._conversations.filter(c => c.conversationPath !== conversationPath);
    this.notify();
  }

  /**
   * Mark a conversation as read.
   */
  async markAsRead(conversationPath: string): Promise<void> {
    await this.database.conversations.markAsRead(conversationPath);
    this._conversations = this._conversations.map(c =>
      c.conversationPath === conversationPath ? { ...c, unreadCount: 0 } : c
    );
    this.notify();
  }

  /**
   * Get or create a ConversationStore for a specific conversation.
   * Caches instances so repeated calls return the same store.
   */
  getConversationStore(
    conversationPath: string,
    overrides?: Partial<ConversationStoreOptions>,
  ): ConversationStore {
    let store = this._stores.get(conversationPath);
    if (!store) {
      store = new ConversationStore({
        database: this.database,
        conversationPath,
        ...overrides,
      });
      this._stores.set(conversationPath, store);
    }
    return store;
  }

  /**
   * Clean up all resources.
   */
  destroy(): void {
    this._realtimeSubscription?.unsubscribe();
    this._realtimeSubscription = null;

    for (const store of this._stores.values()) {
      store.destroy();
    }
    this._stores.clear();
    this.subscribers.clear();
  }

  // ==========================================================================
  // Internal
  // ==========================================================================

  private async startRealtimeSubscription(): Promise<void> {
    if (this._realtimeSubscription) return;
    logger.debug('[ConversationListStore] Starting realtime subscription');

    try {
      const workspace = this.database.workspace('raisin:access_control');
      const events = workspace.events();

      // We need the user home path to subscribe to the right folder
      const result = await this.database.executeSql(
        `SELECT RAISIN_CURRENT_USER()->>'path'::String as home`,
      );
      const rows = result.rows as unknown as Record<string, unknown>[];
      const userHome = rows?.[0]?.home as string;
      if (!userHome) return;

      const chatsPath = `${userHome}/inbox/chats`;

      this._realtimeSubscription = await events.subscribe(
        {
          workspace: 'raisin:access_control',
          path: chatsPath,
          event_types: ['node:created', 'node:updated'],
        },
        async (event) => {
          const eventData = event as any;
          const nodeId = eventData.payload?.node_id;
          if (!nodeId) return;

          try {
            const nodeResult = await this.database.executeSql(
              `SELECT id, path, node_type, properties, created_at, updated_at FROM 'raisin:access_control' WHERE id = $1 LIMIT 1`,
              [nodeId],
            );
            const row = (nodeResult.rows as unknown as Record<string, unknown>[])?.[0];
            if (!row || row.node_type !== 'raisin:Conversation') return;

            const convPath = row.path as string;
            const props = (row.properties ?? {}) as Record<string, unknown>;
            const item: ConversationListItem = {
              id: row.id as string,
              type: (props.conversation_type as ConversationType) ?? 'ai_chat',
              conversationPath: convPath,
              conversationWorkspace: 'raisin:access_control',
              agentRef: props.agent_ref as string | undefined,
              participants: props.participants as string[] | undefined,
              unreadCount: (props.unread_count as number) ?? 0,
              lastMessage: props.last_message as { content: string; sender_id: string; created_at: string } | undefined,
              updatedAt: (row.updated_at ?? row.created_at) as string,
            };

            const existingIdx = this._conversations.findIndex(c => c.conversationPath === convPath);
            if (existingIdx >= 0) {
              // Update existing conversation and move to top
              this._conversations.splice(existingIdx, 1);
            }
            this._conversations = [item, ...this._conversations];
            this.notify();
          } catch (err) {
            logger.debug('Failed to process conversation event:', err);
          }
        },
      );
    } catch (err) {
      logger.debug('Failed to start realtime subscription:', err);
    }
  }

  private notify(): void {
    const snapshot = this.getSnapshot();
    for (const sub of this.subscribers) {
      try { sub(snapshot); }
      catch (err) { logger.error('Error in conversation list store subscriber:', err); }
    }
  }
}
