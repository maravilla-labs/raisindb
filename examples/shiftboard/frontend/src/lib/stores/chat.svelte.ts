/**
 * Planning chat state (Svelte 5 runes) — wraps the SDK's ConversationStore.
 *
 * SSR handoff: +page.server.ts resolves the latest agent conversation and its
 * message history; the page seeds this store before first render so the chat
 * history is part of the server HTML. After hydration, connect() wires up the
 * real ConversationStore (streaming, tool calls) without re-fetching.
 *
 * ConversationStore is framework-agnostic and exposes a snapshot/subscribe
 * pattern; we mirror its snapshot into a $state rune so components can read
 * `chat.snapshot.messages`, `chat.snapshot.isStreaming`, etc. reactively.
 *
 * SDK APIs used:
 *   new ConversationStore({ database, conversationPath?, createOptions })
 *     .subscribe(cb)        - state snapshots (messages, streamingText, tool calls)
 *     .sendMessage(text)    - send + stream the agent's reply (conversation is
 *                             created lazily on first send via createOptions)
 */
import {
  ConversationStore,
  type ChatMessage,
  type ConversationStoreSnapshot,
} from '@raisindb/client';
import { getDb } from '../raisin';
import { AGENT_PATH } from '../agent';

export interface ChatSSRData {
  conversationPath: string | null;
  messages: ChatMessage[];
}

const EMPTY_SNAPSHOT: ConversationStoreSnapshot = {
  conversation: null,
  messages: [],
  isStreaming: false,
  isWaiting: false,
  streamingText: '',
  error: null,
  activeToolCalls: [],
  plans: [],
  isLoading: false,
  conversationPath: null,
};

function seededSnapshot(ssr: ChatSSRData): ConversationStoreSnapshot {
  return {
    ...EMPTY_SNAPSHOT,
    conversation: ssr.conversationPath
      ? { conversationPath: ssr.conversationPath, type: 'ai_chat' }
      : null,
    conversationPath: ssr.conversationPath,
    messages: ssr.messages,
  };
}

class ChatState {
  snapshot = $state<ConversationStoreSnapshot>(EMPTY_SNAPSHOT);
  /** True once the live store is wired up (input enabled). */
  ready = $state(false);

  #store: ConversationStore | null = null;
  #connected = false;

  /** Seed from SSR data. Runs during server render and again on hydration. */
  seed(ssr: ChatSSRData): void {
    if (this.#connected) return; // live store took over already
    this.snapshot = seededSnapshot(ssr);
  }

  /** Wire up the live ConversationStore. Call once after hydration. */
  connect(ssr: ChatSSRData): void {
    if (this.#connected) return;
    this.#connected = true;

    // With a conversationPath the store resumes it (persistent SSE
    // subscription); otherwise createOptions makes sendMessage() create the
    // conversation on first send.
    this.#store = new ConversationStore({
      database: getDb(),
      conversationPath: ssr.conversationPath ?? undefined,
      createOptions: { participant: AGENT_PATH },
    });

    // Seed the store with the SSR-loaded history instead of calling
    // loadMessages() — avoids a duplicate fetch on every page load.
    // (SDK friction: ConversationStore has no public initialMessages /
    // setMessages hydration hook yet, so we reach into the TS-private field.)
    if (ssr.messages.length > 0) {
      (this.#store as unknown as { _messages: ChatMessage[] })._messages = [...ssr.messages];
    }

    this.#store.subscribe((s) => {
      this.snapshot = s;
    });
    this.ready = true;
  }

  async send(text: string): Promise<void> {
    await this.#store?.sendMessage(text);
  }
}

export const chat = new ChatState();
