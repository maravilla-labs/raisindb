/**
 * Active Conversation Store — Svelte 5 rune-based wrapper around SDK ConversationStore.
 *
 * Uses $state runes for fine-grained reactivity. Components access properties
 * directly (e.g., `chat.messages`) instead of the Svelte 4 `$store` syntax.
 *
 * @example
 * ```svelte
 * <script>
 *   import { chat } from '$lib/stores/active-conversation.svelte';
 *   const messages = $derived(chat.messages);
 * </script>
 *
 * {#each messages as msg}...{/each}
 * {#if chat.isStreaming}<p>{chat.streamingText}</p>{/if}
 * ```
 */
import {
  ConversationStore,
  type ConversationStoreSnapshot,
  type Database,
} from '@raisindb/client';

class ActiveConversationState {
  snapshot = $state<ConversationStoreSnapshot | null>(null);
  #store: ConversationStore | null = null;
  #unsubscribe: (() => void) | null = null;

  // ---------------------------------------------------------------------------
  // Derived getters — reactive because they read from $state snapshot
  // ---------------------------------------------------------------------------

  get messages() { return this.snapshot?.messages ?? []; }
  get isStreaming() { return this.snapshot?.isStreaming ?? false; }
  get isWaiting() { return this.snapshot?.isWaiting ?? false; }
  get streamingText() { return this.snapshot?.streamingText ?? ''; }
  get activeToolCalls() { return this.snapshot?.activeToolCalls ?? []; }
  get plans() { return this.snapshot?.plans ?? []; }
  get error() { return this.snapshot?.error ?? null; }
  get conversationPath() { return this.snapshot?.conversationPath ?? null; }
  get isLoading() { return this.snapshot?.isLoading ?? false; }

  // ---------------------------------------------------------------------------
  // Actions
  // ---------------------------------------------------------------------------

  /** Open an existing conversation by path. Loads messages and subscribes to SSE. */
  async open(db: Database, path: string): Promise<void> {
    this.close();
    this.#store = new ConversationStore({
      database: db,
      conversationPath: path,
    });
    this.#unsubscribe = this.#store.subscribe(s => { this.snapshot = s; });
    await this.#store.loadMessages();
  }

  /** Start a new conversation. Created on first sendMessage(). */
  start(db: Database, participant: string, input?: Record<string, unknown>): void {
    this.close();
    this.#store = new ConversationStore({
      database: db,
      createOptions: { participant, input },
    });
    this.#unsubscribe = this.#store.subscribe(s => { this.snapshot = s; });
  }

  /** Send a message in the active conversation. */
  sendMessage = async (content: string): Promise<void> => {
    await this.#store?.sendMessage(content);
  };

  /** Approve a pending plan. */
  approvePlan = async (planPath: string): Promise<void> => {
    await this.#store?.approvePlan(planPath);
  };

  /** Reject a pending plan with optional feedback. */
  rejectPlan = async (planPath: string, feedback?: string): Promise<void> => {
    await this.#store?.rejectPlan(planPath, feedback);
  };

  /** Stop the current streaming response. */
  stop(): void {
    this.#store?.stop();
  }

  /** Reload messages from the node tree. */
  async reload(): Promise<void> {
    await this.#store?.loadMessages();
  }

  /** Get the underlying ConversationStore (for advanced use). */
  getStore(): ConversationStore | null {
    return this.#store;
  }

  /** Close the conversation and release resources. */
  close(): void {
    this.#unsubscribe?.();
    this.#unsubscribe = null;
    this.#store?.destroy();
    this.#store = null;
    this.snapshot = null;
  }
}

/** Singleton active conversation state. */
export const chat = new ActiveConversationState();
