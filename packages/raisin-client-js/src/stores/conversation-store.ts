/**
 * Framework-agnostic conversation store for RaisinDB.
 *
 * Manages the full conversation lifecycle: creating conversations, sending
 * messages, streaming responses, persistent SSE subscriptions, message
 * history, tool call tracking, and plan projection. Notifies subscribers
 * on every state change so UI frameworks (React, Svelte, Vue) can bind.
 *
 * @example Svelte 5:
 * ```typescript
 * const store = new ConversationStore({ database: db, conversationPath: '/...' });
 * let snapshot = $state(store.getSnapshot());
 * store.subscribe(s => { snapshot = s; });
 * ```
 *
 * @example React (via useConversation hook):
 * ```tsx
 * const chat = useConversation(React, { database: db, conversationPath: '/...' });
 * ```
 */

import type { ConversationManager, ConversationSubscription } from '../conversations';
import type { Database } from '../database';
import type { ChatMessage, ChatEvent, ChatLogEvent } from '../types/chat';
import { logger, getLogLevel, LogLevel } from '../logger';
import { projectPlansFromMessages, type PlanProjection } from '../utils/plan-projection';
import { isRecoveredDoneEvent } from '../utils/chat-events';

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Info about an in-flight tool call */
export interface ToolCallInfo {
  id: string;
  functionName: string;
  arguments: unknown;
  status: 'running' | 'completed' | 'failed';
  result?: unknown;
  durationMs?: number;
}

/** Snapshot of the conversation store state */
export interface ConversationStoreSnapshot {
  /** The conversation object, if loaded */
  conversation: { conversationPath: string; type: string } | null;
  /** All messages in the conversation */
  messages: ChatMessage[];
  /** Whether the AI is currently generating a response */
  isStreaming: boolean;
  /** Whether the chat is waiting for user input */
  isWaiting: boolean;
  /** Accumulated text from the current streaming response */
  streamingText: string;
  /** Current error, if any */
  error: string | null;
  /** Tracks in-flight tool executions */
  activeToolCalls: ToolCallInfo[];
  /** Deterministic plan/task projection from persisted messages */
  plans: PlanProjection[];
  /** Whether messages are being loaded */
  isLoading: boolean;
  /** Path to the conversation node */
  conversationPath: string | null;
}

/** Options for creating a ConversationStore */
export interface ConversationStoreOptions {
  /** Pre-configured Database instance */
  database: Database;
  /** Resume an existing conversation */
  conversationPath?: string;
  /** Create a new conversation on first message */
  createOptions?: {
    participant: string;
    input?: Record<string, unknown>;
  };
  /** Callback for individual chat events */
  onEvent?: (event: ChatEvent) => void;
  /** Streaming inactivity timeout in ms (default: 120000). Auto-recovers when no SSE event arrives within this window. */
  streamingTimeoutMs?: number;
  /** Activity watchdog polling interval in ms (default: 30000). Periodically checks backend turn health while streaming. */
  watchdogIntervalMs?: number;
}

type Subscriber = (snapshot: ConversationStoreSnapshot) => void;

// ---------------------------------------------------------------------------
// ConversationStore
// ---------------------------------------------------------------------------

/**
 * Framework-agnostic store managing a single conversation's state.
 *
 * Handles the full lifecycle: creating conversations, sending messages,
 * streaming responses, tool call tracking, plan projection, and deferred
 * message reload on turn completion.
 */
export class ConversationStore {
  private manager: ConversationManager;
  private subscribers = new Set<Subscriber>();
  private onEvent?: (event: ChatEvent) => void;
  private createOptions?: { participant: string; input?: Record<string, unknown> };

  // Persistent SSE subscription
  private subscription: ConversationSubscription | null = null;

  // Internal mutable state
  private _conversationPath: string | null;
  private _messages: ChatMessage[] = [];
  private _isStreaming = false;
  private _isWaiting = false;
  private _streamingText = '';
  private _error: string | null = null;
  private _activeToolCalls: ToolCallInfo[] = [];
  private _isLoading = false;
  private _conversationType: string | null = null;

  // Streaming timeout (Fix A)
  private _streamingTimeoutMs: number;
  private _streamingTimer: ReturnType<typeof setTimeout> | null = null;

  // Activity watchdog (Fix B)
  private _watchdogIntervalMs: number;
  private _watchdogTimer: ReturnType<typeof setInterval> | null = null;

  constructor(options: ConversationStoreOptions) {
    this.manager = options.database.conversations;
    this._conversationPath = options.conversationPath ?? null;
    this.createOptions = options.createOptions;
    this.onEvent = options.onEvent;
    this._streamingTimeoutMs = options.streamingTimeoutMs ?? 120_000;
    this._watchdogIntervalMs = options.watchdogIntervalMs ?? 30_000;

    // Auto-subscribe to existing conversation
    if (this._conversationPath) {
      this.ensureSubscription();
    }
  }

  // ==========================================================================
  // Subscription
  // ==========================================================================

  /**
   * Subscribe to state changes.
   * Returns an unsubscribe function.
   */
  subscribe(callback: Subscriber): () => void {
    this.subscribers.add(callback);
    callback(this.getSnapshot());
    return () => { this.subscribers.delete(callback); };
  }

  /**
   * Get the current state snapshot.
   */
  getSnapshot(): ConversationStoreSnapshot {
    return {
      conversation: this._conversationPath
        ? { conversationPath: this._conversationPath, type: this._conversationType || 'ai_chat' }
        : null,
      messages: [...this._messages],
      isStreaming: this._isStreaming,
      isWaiting: this._isWaiting,
      streamingText: this._streamingText,
      error: this._error,
      activeToolCalls: [...this._activeToolCalls],
      plans: projectPlansFromMessages(this._messages),
      isLoading: this._isLoading,
      conversationPath: this._conversationPath,
    };
  }

  // ==========================================================================
  // Actions
  // ==========================================================================

  /**
   * Send a user message and stream the AI response.
   *
   * If no conversation exists yet and `createOptions` was provided,
   * the conversation is created automatically on the first call.
   */
  async sendMessage(content: string): Promise<void> {
    if (this._isStreaming) {
      logger.warn('[ConversationStore] Cannot send message while streaming');
      return;
    }

    logger.info('[ConversationStore] Sending message', { path: this._conversationPath });
    this._error = null;

    // Create conversation if needed
    if (!this._conversationPath) {
      if (!this.createOptions) {
        throw new Error('No conversationPath and no createOptions provided');
      }
      try {
        const convo = await this.manager.create({
          participant: this.createOptions.participant,
          input: this.createOptions.input,
        });
        this._conversationPath = convo.conversationPath;
        this._conversationType = convo.type;

        for (const event of convo.initialEvents ?? []) {
          this.onEvent?.(event);
          this.handleEvent(event);
        }

        this.notify();
      } catch (err) {
        this._error = err instanceof Error ? err.message : String(err);
        this.notify();
        return;
      }
    }

    // Add user message optimistically
    const userMessage: ChatMessage = {
      role: 'user',
      content,
      timestamp: new Date().toISOString(),
    };
    this._messages.push(userMessage);

    // Start streaming
    this._isStreaming = true;
    this._isWaiting = false;
    this._streamingText = '';
    this._activeToolCalls = [];
    this.resetStreamingTimer();
    this.startWatchdog();
    this.notify();

    // Use persistent SSE subscription + create message node
    try {
      this.ensureSubscription();
      await this.subscription!.waitUntilConnected();
      await this.manager.createUserMessage(this._conversationPath!, content);
      // Events arrive via the persistent subscription → handleEvent
    } catch (err) {
      logger.error('[ConversationStore] Send failed', err);
      this._error = err instanceof Error ? err.message : String(err);
      this._isStreaming = false;
      this.clearStreamingTimer();
      this.stopWatchdog();
      this.notify();
    }
  }

  /**
   * Load messages from the persisted node tree.
   * Useful for loading history on page reload.
   */
  async loadMessages(): Promise<ChatMessage[]> {
    if (!this._conversationPath) return [];
    this._isLoading = true;
    this.notify();
    try {
      const messages = await this.manager.getMessages(this._conversationPath);
      logger.debug(`[ConversationStore] Loaded ${messages.length} messages`);
      if (messages.length > 0) {
        this._messages = messages;
        // Rebuild tool call state from messages
        this._activeToolCalls = [];
      }
      return messages;
    } finally {
      this._isLoading = false;
      this.notify();
    }
  }

  /**
   * Approve a pending plan.
   */
  async approvePlan(planPath: string): Promise<void> {
    await this.manager.approvePlan(planPath);
  }

  /**
   * Reject a pending plan.
   */
  async rejectPlan(planPath: string, feedback?: string): Promise<void> {
    await this.manager.rejectPlan(planPath, feedback);
  }

  /**
   * Mark a single message as read.
   */
  async markMessageAsRead(messagePath: string): Promise<void> {
    await this.manager.markMessageAsRead(messagePath);
  }

  /**
   * Stop the current streaming response.
   */
  stop(): void {
    this._isStreaming = false;
    this.clearStreamingTimer();
    this.stopWatchdog();
    this.notify();
  }

  /**
   * Clean up the store and release resources.
   */
  destroy(): void {
    this.stop();
    this.clearStreamingTimer();
    this.stopWatchdog();
    this.subscription?.unsubscribe();
    this.subscription = null;
    this.subscribers.clear();
  }

  /**
   * Get the conversation path.
   */
  getConversationPath(): string | null {
    return this._conversationPath;
  }

  // ==========================================================================
  // Internal
  // ==========================================================================

  private ensureSubscription(): void {
    if (this.subscription) return;
    if (!this._conversationPath) return;

    this.subscription = this.manager.subscribe(
      this._conversationPath,
      (event) => {
        this.onEvent?.(event);
        this.handleEvent(event);
      },
    );
  }

  private handleEvent(event: ChatEvent): void {
    logger.debug('[ConversationStore] SSE:', event.type);
    // Every SSE event resets the streaming inactivity timer
    if (this._isStreaming) {
      this.resetStreamingTimer();
    }
    switch (event.type) {
      case 'text_chunk':
        this._streamingText += event.text;
        this.notify();
        break;

      case 'thought_chunk':
        this.notify();
        break;

      case 'assistant_message':
        this._messages.push(event.message);
        this._streamingText = '';
        this.notify();
        break;

      case 'tool_call_started': {
        logger.debug('[ConversationStore] Tool started:', event.functionName);
        const info: ToolCallInfo = {
          id: event.toolCallId,
          functionName: event.functionName,
          arguments: event.arguments,
          status: 'running',
        };
        this._activeToolCalls.push(info);
        this.notify();
        break;
      }

      case 'tool_call_completed': {
        logger.debug('[ConversationStore] Tool completed:', event.toolCallId);
        const idx = this._activeToolCalls.findIndex(tc => tc.id === event.toolCallId);
        if (idx >= 0) {
          this._activeToolCalls[idx] = {
            ...this._activeToolCalls[idx],
            status: event.error ? 'failed' : 'completed',
            result: event.result,
            durationMs: event.durationMs,
          };
        }
        this.notify();
        break;
      }

      case 'waiting': {
        logger.info('[ConversationStore] Waiting for input');
        this.flushStreamingText(event.timestamp);
        this._isStreaming = false;
        this._isWaiting = true;
        this._activeToolCalls = [];
        this.clearStreamingTimer();
        this.stopWatchdog();
        this.notify();
        // Deferred reload: pick up canonical messages from node tree
        this.deferredReload();
        break;
      }

      case 'done': {
        if (isRecoveredDoneEvent(event)) {
          logger.debug('[ConversationStore] Ignoring recovered done event');
          this.reconcileRecoveredDone();
          break;
        }
        const doneEvent = event as import('../types/chat').ChatDoneEvent;
        if (doneEvent.dispatchPhase && doneEvent.dispatchPhase !== 'terminal') {
          logger.debug('[ConversationStore] Ignoring non-terminal done event');
          break;
        }
        logger.info('[ConversationStore] Turn done');
        this.flushStreamingText(event.timestamp);
        // Safety net: if streaming missed chunks, use the complete response from the event
        if (doneEvent.content && !this.lastMessageIsAssistant()) {
          this._messages.push({
            role: (doneEvent.role as ChatMessage['role']) || 'assistant',
            content: doneEvent.content,
            timestamp: event.timestamp,
            senderDisplayName: doneEvent.senderDisplayName,
            finishReason: doneEvent.finishReason,
          });
        }
        this._isStreaming = false;
        this._isWaiting = true;
        this._activeToolCalls = [];
        this.clearStreamingTimer();
        this.stopWatchdog();
        this.notify();
        this.deferredReload();
        break;
      }

      case 'completed': {
        this.flushStreamingText(event.timestamp);
        if (event.messages && event.messages.length > 0) {
          this._messages = event.messages;
        }
        this._isStreaming = false;
        this._isWaiting = false;
        this._activeToolCalls = [];
        this.clearStreamingTimer();
        this.stopWatchdog();
        this.notify();
        this.deferredReload();
        break;
      }

      case 'failed':
        logger.error('[ConversationStore] Turn failed', event.error);
        this._error = event.error;
        this._isStreaming = false;
        this._streamingText = '';
        this._activeToolCalls = [];
        this.clearStreamingTimer();
        this.stopWatchdog();
        this.notify();
        break;

      case 'conversation_created':
        this._conversationPath = event.conversationPath;
        this.notify();
        break;

      case 'message_saved':
        // A message was persisted — reload to pick it up, then notify
        this.silentReload().catch(() => {});
        this.notify();
        break;

      case 'message_delivered':
        this._messages.push(event.message);
        this.notify();
        break;

      case 'log': {
        const logEvent = event as ChatLogEvent;
        const currentLevel = getLogLevel();
        const levelMap: Record<string, LogLevel> = {
          debug: LogLevel.Debug,
          info: LogLevel.Info,
          warn: LogLevel.Warn,
          error: LogLevel.Error,
        };
        const eventLevel = levelMap[logEvent.level] ?? LogLevel.Debug;
        if (currentLevel >= eventLevel) {
          const fn = logEvent.level === 'error' ? console.error
                   : logEvent.level === 'warn'  ? console.warn
                   : logEvent.level === 'debug' ? console.debug
                   : console.info;
          fn('[server]', logEvent.message);
        }
        break;
      }
    }
  }

  /**
   * If accumulated streaming text hasn't been captured in an assistant_message,
   * synthesize one.
   */
  private flushStreamingText(timestamp: string): void {
    if (this._streamingText && !this.lastMessageIsAssistant()) {
      this._messages.push({
        role: 'assistant',
        content: this._streamingText,
        timestamp,
      });
    }
    this._streamingText = '';
  }

  private lastMessageIsAssistant(): boolean {
    const last = this._messages[this._messages.length - 1];
    return last?.role === 'assistant';
  }

  /**
   * After a turn completes, reload canonical messages from the node tree
   * to catch any events missed during SSE gaps.
   */
  private deferredReload(): void {
    if (!this._conversationPath) return;
    logger.debug('[ConversationStore] Scheduling deferred reload');
    const attemptReload = (delay: number, retriesLeft: number) => {
      setTimeout(async () => {
        try {
          const prevCount = this._messages.length;
          await this.silentReload();
          // If no new messages appeared and retries remain, try again with exponential backoff
          if (this._messages.length === prevCount && retriesLeft > 0) {
            attemptReload(delay * 2, retriesLeft - 1);
          }
        } catch {}
      }, delay);
    };
    attemptReload(500, 2); // 500ms, 1000ms, 2000ms
  }

  private async silentReload(): Promise<void> {
    if (!this._conversationPath) return;
    try {
      const fresh = await this.manager.getMessages(this._conversationPath);
      if (fresh.length === 0) return;

      // Try incremental merge: match existing messages by role+content
      // to preserve object references (prevents Svelte DOM recreation)
      let matchCount = 0;
      const limit = Math.min(this._messages.length, fresh.length);
      for (let i = 0; i < limit; i++) {
        if (this._messages[i].role !== fresh[i].role ||
            this._messages[i].content !== fresh[i].content) break;
        matchCount++;
      }

      if (matchCount === this._messages.length && fresh.length === matchCount) {
        // Same content, possibly different metadata — update silently
        for (let i = 0; i < matchCount; i++) {
          const f = fresh[i];
          const e = this._messages[i];
          if (f.id) e.id = f.id;
          if (f.path) e.path = f.path;
          if (f.messageType) e.messageType = f.messageType;
          if (f.data) e.data = f.data;
          if (f.children) e.children = f.children;
          if (f.finishReason) e.finishReason = f.finishReason;
        }
        await this.rebuildToolCallState();
        return; // No visible change — don't notify
      }

      if (matchCount === this._messages.length && fresh.length > matchCount) {
        // Existing messages match prefix — append new ones
        for (let i = matchCount; i < fresh.length; i++) {
          this._messages.push(fresh[i]);
        }
        // Also update metadata on matched messages
        for (let i = 0; i < matchCount; i++) {
          const f = fresh[i];
          const e = this._messages[i];
          if (f.id) e.id = f.id;
          if (f.path) e.path = f.path;
          if (f.messageType) e.messageType = f.messageType;
          if (f.data) e.data = f.data;
          if (f.children) e.children = f.children;
        }
        await this.rebuildToolCallState();
        this.notify(); // New messages to show
        return;
      }

      // Fresh is a prefix subset — outbox delivery pending, keep extras
      if (matchCount === fresh.length && fresh.length < this._messages.length) {
        for (let i = 0; i < matchCount; i++) {
          const f = fresh[i];
          const e = this._messages[i];
          if (f.id) e.id = f.id;
          if (f.path) e.path = f.path;
          if (f.messageType) e.messageType = f.messageType;
          if (f.data) e.data = f.data;
          if (f.children) e.children = f.children;
        }
        await this.rebuildToolCallState();
        return; // Keep streaming-flushed extras — message_saved will trigger next reload
      }

      // Content mismatch — full replace
      this._messages = fresh;
      await this.rebuildToolCallState();
      this.notify();
    } catch (err) {
      logger.debug('[ConversationStore] Silent reload failed:', err);
    }
  }

  // ==========================================================================
  // Streaming timeout (Fix A) — auto-recover on SSE inactivity
  // ==========================================================================

  private resetStreamingTimer(): void {
    this.clearStreamingTimer();
    if (!this._isStreaming) return;
    this._streamingTimer = setTimeout(() => {
      if (this._isStreaming) {
        logger.warn('[ConversationStore] Streaming timeout — no SSE activity for', this._streamingTimeoutMs, 'ms');
        this.recoverFromHang();
      }
    }, this._streamingTimeoutMs);
  }

  private clearStreamingTimer(): void {
    if (this._streamingTimer) {
      clearTimeout(this._streamingTimer);
      this._streamingTimer = null;
    }
  }

  // ==========================================================================
  // Activity watchdog (Fix B) — periodic backend health check
  // ==========================================================================

  private startWatchdog(): void {
    this.stopWatchdog();
    if (!this._conversationPath) return;
    this._watchdogTimer = setInterval(async () => {
      if (!this._isStreaming || !this._conversationPath) {
        this.stopWatchdog();
        return;
      }
      try {
        const health = await this.manager.checkTurnHealth(this._conversationPath);
        if (health === 'done' && this._isStreaming) {
          logger.warn('[ConversationStore] Watchdog detected completed turn while still streaming — recovering');
          this.recoverFromHang();
        }
      } catch {
        // Ignore watchdog errors — it's a best-effort check
      }
    }, this._watchdogIntervalMs);
  }

  private stopWatchdog(): void {
    if (this._watchdogTimer) {
      clearInterval(this._watchdogTimer);
      this._watchdogTimer = null;
    }
  }

  // ==========================================================================
  // Hang recovery (shared by timeout + watchdog)
  // ==========================================================================

  private recoverFromHang(): void {
    logger.info('[ConversationStore] Recovering from hang');
    this.clearStreamingTimer();
    this.stopWatchdog();
    this._isStreaming = false;
    this._isWaiting = true;
    this._streamingText = '';
    this.notify();
    // Reload to pick up any persisted state we missed
    this.silentReload().catch(() => {});
  }

  /**
   * Reconcile a recovered done event against persisted state.
   * If the backend turn is actually complete, finalize the local state.
   * Otherwise keep streaming active and let normal events continue.
   */
  private reconcileRecoveredDone(): void {
    if (!this._conversationPath) return;
    void this.manager.checkTurnHealth(this._conversationPath).then((health) => {
      if (health !== 'done') return;
      if (!this._isStreaming) return;
      this._isStreaming = false;
      this._isWaiting = true;
      this._activeToolCalls = [];
      this.clearStreamingTimer();
      this.stopWatchdog();
      this.notify();
      this.deferredReload();
    }).catch(() => {
      // Best effort only; streaming watchdog still protects against hangs.
    });
  }

  // ==========================================================================
  // Tool call state rebuild (Fix C) — reconstruct from node tree on reload
  // ==========================================================================

  private async rebuildToolCallState(): Promise<void> {
    if (!this._conversationPath) {
      this._activeToolCalls = [];
      return;
    }
    try {
      const pendingCalls = await this.manager.getActiveToolCalls(this._conversationPath);
      if (pendingCalls.length > 0) {
        this._activeToolCalls = pendingCalls.map(tc => ({
          id: tc.id,
          functionName: tc.name,
          arguments: undefined,
          status: 'running' as const,
        }));
        // Keep streaming active if there are pending tool calls
        if (!this._isStreaming) {
          this._isStreaming = true;
          this.resetStreamingTimer();
          this.startWatchdog();
        }
      } else {
        this._activeToolCalls = [];
        // No pending tools — check if we should clear streaming
        if (this._isStreaming) {
          const lastMsg = this._messages[this._messages.length - 1];
          const isLastAssistantDone = lastMsg?.role === 'assistant' &&
            (lastMsg.dispatchPhase
              ? lastMsg.dispatchPhase === 'terminal'
              : (!!lastMsg.finishReason && lastMsg.finishReason !== 'tool_calls'));
          if (isLastAssistantDone) {
            this._isStreaming = false;
            this._isWaiting = true;
            this.clearStreamingTimer();
            this.stopWatchdog();
          }
        }
      }
    } catch {
      this._activeToolCalls = [];
    }
  }

  private notify(): void {
    const snapshot = this.getSnapshot();
    for (const sub of this.subscribers) {
      try { sub(snapshot); }
      catch (err) { logger.error('Error in conversation store subscriber:', err); }
    }
  }
}
