/**
 * Anonymous visitor chat — an agent chat on a public website, straight from
 * the browser over the WebSocket. No login, no site server endpoint, no
 * service identity.
 *
 * The server keeps a visitor session per browser tab: an ephemeral home
 * (`/visitors/<key>` in `raisin:access_control`) that only this session can
 * read, and that nobody but the server writes. The browser holds the
 * session's secret (in sessionStorage by default) so a reload or a reconnect
 * continues the same conversation.
 *
 * Use it through `db.conversations`:
 *
 * ```typescript
 * const client = new RaisinClient('wss://db.example.com/ws/website');
 * await client.connect();                      // anonymous
 * const db = client.database('website');
 *
 * const convo = await db.conversations.startAnonymous('/agents/website-assistant');
 * for await (const ev of db.conversations.sendMessage(convo.conversationPath, 'Hi!')) {
 *   if (ev.type === 'text_chunk') render(ev.text);
 * }
 * const history = await db.conversations.getMessages(convo.conversationPath);
 * ```
 *
 * The agent must allow anonymous visitors (`anonymous.enabled: true` on its
 * raisin:AIAgent node); its limits (message size and count, token budget,
 * rate, one turn at a time) are enforced by the server and surface here as
 * {@link VisitorChatError} codes.
 */

import type { EventHandler } from './events';
import type { EventMessage } from './protocol';
import type { ChatEvent, Conversation } from './types/chat';
import { logger } from './logger';

/** Where the session secret is kept between page loads. */
export interface VisitorSessionStore {
  get(key: string): string | null;
  set(key: string, value: string): void;
  remove(key: string): void;
}

/** sessionStorage when the page has one, memory otherwise (one tab = one session). */
export function defaultVisitorSessionStore(): VisitorSessionStore {
  const memory = new Map<string, string>();
  const storage = (): Storage | null => {
    try {
      return typeof sessionStorage !== 'undefined' ? sessionStorage : null;
    } catch {
      return null;
    }
  };
  return {
    get(key) {
      try {
        return storage()?.getItem(key) ?? memory.get(key) ?? null;
      } catch {
        return memory.get(key) ?? null;
      }
    },
    set(key, value) {
      memory.set(key, value);
      try {
        storage()?.setItem(key, value);
      } catch {
        /* storage blocked: memory only */
      }
    },
    remove(key) {
      memory.delete(key);
      try {
        storage()?.removeItem(key);
      } catch {
        /* storage blocked */
      }
    },
  };
}

/** Options for {@link VisitorChat.start}. */
export interface StartAnonymousOptions {
  /** Continue this conversation of the session (from `conversationId` of an earlier start). */
  conversationId?: string;
  /** Force a fresh conversation even when the store remembers one. */
  newConversation?: boolean;
  /** A session secret to resume; defaults to the one in `store`. */
  sessionToken?: string;
  /** Where to keep the secret and the last conversation; `false` keeps nothing. */
  store?: VisitorSessionStore | false;
  /** Store key prefix (default `raisindb:visitor:<repo>:<agent>`). */
  storageKey?: string;
}

/** The limits the agent applies, as the server reported them. */
export interface VisitorChatLimits {
  max_messages: number;
  max_message_chars: number;
}

/** A conversation started with {@link VisitorChat.start}. */
export interface AnonymousConversation extends Conversation {
  anonymous: true;
  /** The conversation id (also its event channel `chat:<id>`). */
  conversationId: string;
  /** The session secret. Keep it private: it is the visitor's credential. */
  sessionToken: string;
  limits: VisitorChatLimits;
}

/**
 * A refusal from the server. `code` is one of:
 * `ANONYMOUS_NOT_ALLOWED`, `ORIGIN_NOT_ALLOWED`, `RATE_LIMITED`, `BUSY`,
 * `TOO_LONG`, `EMPTY_MESSAGE`, `TOO_MANY_MESSAGES`, `LIMIT_REACHED`,
 * `DAILY_LIMIT_REACHED` (the agent's daily token budget, or the per-IP daily
 * message cap),
 * `CONVERSATION_LIMIT`, `SESSION_EXPIRED`, `SESSION_MISMATCH`,
 * `UNKNOWN_CONVERSATION`, `NO_SESSION`, `NOT_ANONYMOUS`, `UNAVAILABLE`.
 */
export class VisitorChatError extends Error {
  readonly code: string;
  constructor(message: string, code: string) {
    super(message);
    this.name = 'VisitorChatError';
    this.code = code;
  }
}

type Send = (payload: unknown, requestType: string) => Promise<unknown>;

interface StartResponse {
  session_token: string | null;
  session_home: string;
  conversation_id: string;
  conversation_path: string;
  conversation_workspace: string;
  channel: string;
  subscription_id: string;
  agent: string;
  limits: VisitorChatLimits;
}

interface Entry {
  agent: string;
  conversationId: string;
  subscriptionId: string;
  listeners: Set<(event: ChatEvent) => void>;
  options: StartAnonymousOptions;
}

/** Codes after which a re-bind (a reconnect lost the server side) is worth one retry. */
const REBIND_CODES = new Set(['NO_SESSION', 'UNKNOWN_CONVERSATION']);

function asError(err: unknown): VisitorChatError {
  const e = err as { message?: string; code?: string };
  return new VisitorChatError(e?.message ?? String(err), e?.code ?? 'UNAVAILABLE');
}

/**
 * The anonymous half of `db.conversations`. Not used directly: call
 * `db.conversations.startAnonymous(...)`.
 */
export class VisitorChat {
  private byPath = new Map<string, Entry>();
  private token: string | null = null;

  constructor(
    private readonly repository: string,
    private readonly send: Send,
    private readonly events: EventHandler,
    onReconnected?: (callback: () => void) => () => void,
  ) {
    // A reconnect is a new server-side connection: it must prove the session
    // again before it can read its home or receive its conversations.
    onReconnected?.(() => {
      void this.rebindAll();
    });
  }

  /** Whether `conversationPath` is a visitor conversation started here. */
  isAnonymous(conversationPath: string): boolean {
    return this.byPath.has(conversationPath);
  }

  private storeOf(options: StartAnonymousOptions): VisitorSessionStore | null {
    if (options.store === false) return null;
    return options.store ?? defaultVisitorSessionStore();
  }

  private keyOf(agent: string, options: StartAnonymousOptions): string {
    return options.storageKey ?? `raisindb:visitor:${this.repository}:${agent}`;
  }

  /** Start (or resume) a visitor conversation with `agent` (`/agents/<name>`). */
  async start(agent: string, options: StartAnonymousOptions = {}): Promise<AnonymousConversation> {
    const store = this.storeOf(options);
    const key = this.keyOf(agent, options);
    const sessionToken = options.sessionToken ?? this.token ?? store?.get(`${key}:session`) ?? undefined;
    const conversationId = options.newConversation
      ? undefined
      : options.conversationId ?? store?.get(`${key}:conversation`) ?? undefined;

    let res: StartResponse;
    try {
      res = (await this.send(
        { agent, session_token: sessionToken, conversation_id: conversationId },
        'visitor_chat_start',
      )) as StartResponse;
    } catch (err) {
      const e = asError(err);
      // A remembered conversation or session that is gone: start afresh once.
      if (
        (conversationId || sessionToken) &&
        !options.conversationId &&
        !options.sessionToken &&
        ['UNKNOWN_CONVERSATION', 'INVALID_CONVERSATION', 'SESSION_EXPIRED', 'SESSION_MISMATCH'].includes(e.code)
      ) {
        store?.remove(`${key}:conversation`);
        if (e.code !== 'UNKNOWN_CONVERSATION' && e.code !== 'INVALID_CONVERSATION') {
          store?.remove(`${key}:session`);
          this.token = null;
        }
        return this.start(agent, { ...options, newConversation: true });
      }
      throw e;
    }

    const token = res.session_token ?? sessionToken ?? this.token;
    if (!token) throw new VisitorChatError('the server returned no session', 'UNAVAILABLE');
    this.token = token;
    store?.set(`${key}:session`, token);
    store?.set(`${key}:conversation`, res.conversation_id);

    const path = res.conversation_path;
    let entry = this.byPath.get(path);
    if (!entry) {
      entry = {
        agent: res.agent,
        conversationId: res.conversation_id,
        subscriptionId: res.subscription_id,
        listeners: new Set(),
        options,
      };
      this.byPath.set(path, entry);
    }
    this.route(entry);

    const now = new Date().toISOString();
    return {
      id: res.conversation_id,
      type: 'ai_chat',
      agentRef: res.agent,
      conversationPath: path,
      conversationWorkspace: res.conversation_workspace,
      anonymous: true,
      conversationId: res.conversation_id,
      sessionToken: token,
      limits: res.limits,
      initialEvents: [{ type: 'waiting', timestamp: now }],
    };
  }

  /** Route the server's forwarded events for this conversation to its listeners. */
  private route(entry: Entry): void {
    this.events.removeFlowEventListener(entry.subscriptionId);
    this.events.addFlowEventListener(entry.subscriptionId, (msg: EventMessage) => {
      const event = msg.payload as unknown as ChatEvent;
      if (!event || typeof (event as { type?: unknown }).type !== 'string') return;
      for (const listener of entry.listeners) {
        try {
          listener(event);
        } catch (err) {
          logger.error('[VisitorChat] listener failed', err);
        }
      }
    });
  }

  /** Re-prove the session on a new connection, for every conversation started here. */
  private async rebindAll(): Promise<void> {
    for (const entry of this.byPath.values()) {
      try {
        await this.send(
          { agent: entry.agent, session_token: this.token, conversation_id: entry.conversationId },
          'visitor_chat_start',
        );
        this.route(entry);
      } catch (err) {
        logger.warn('[VisitorChat] could not resume a conversation after reconnect', err);
      }
    }
  }

  /** Listen to a visitor conversation's events. Returns an unsubscribe function. */
  listen(conversationPath: string, listener: (event: ChatEvent) => void): () => void {
    const entry = this.require(conversationPath);
    entry.listeners.add(listener);
    return () => {
      entry.listeners.delete(listener);
    };
  }

  private require(conversationPath: string): Entry {
    const entry = this.byPath.get(conversationPath);
    if (!entry) {
      throw new VisitorChatError(
        'not a visitor conversation of this client; call startAnonymous first',
        'UNKNOWN_CONVERSATION',
      );
    }
    return entry;
  }

  /** Make sure this connection is bound (after a reconnect nothing else may read the home). */
  async ensureBound(conversationPath: string): Promise<void> {
    const entry = this.require(conversationPath);
    await this.send(
      { agent: entry.agent, session_token: this.token, conversation_id: entry.conversationId },
      'visitor_chat_start',
    ).catch((err) => {
      throw asError(err);
    });
  }

  /** Send one visitor message. Events arrive through {@link listen}. */
  async post(conversationPath: string, text: string): Promise<{ messageId: string; messagesLeft?: number }> {
    const entry = this.require(conversationPath);
    const once = async () =>
      (await this.send(
        { conversation_id: entry.conversationId, text },
        'visitor_chat_send',
      )) as { message_id: string; messages_left?: number };
    try {
      const res = await once();
      return { messageId: res.message_id, messagesLeft: res.messages_left };
    } catch (err) {
      const e = asError(err);
      if (!REBIND_CODES.has(e.code)) throw e;
      // The connection was replaced since the start: prove the session again.
      await this.ensureBound(conversationPath);
      this.route(entry);
      const res = await once().catch((again) => {
        throw asError(again);
      });
      return { messageId: res.message_id, messagesLeft: res.messages_left };
    }
  }

  /**
   * Send one visitor message and stream the turn: `text_chunk`,
   * `tool_call_started`, `tool_call_completed`, … until `done`. A refusal
   * ends the stream with `failed` (its `code` says why). No event for
   * `inactivityTimeoutMs` ends it with a synthetic `waiting`.
   */
  async *stream(
    conversationPath: string,
    text: string,
    options: { signal?: AbortSignal; inactivityTimeoutMs?: number } = {},
  ): AsyncIterable<ChatEvent> {
    const queue: ChatEvent[] = [];
    let wake: (() => void) | null = null;
    const push = (event: ChatEvent) => {
      queue.push(event);
      wake?.();
    };
    // Listen BEFORE sending, or the first events are lost.
    const stop = this.listen(conversationPath, push);
    const onAbort = () => push({ type: 'failed', error: 'aborted', code: 'ABORTED', timestamp: new Date().toISOString() });
    options.signal?.addEventListener('abort', onAbort, { once: true });
    const timeout = options.inactivityTimeoutMs ?? 120_000;
    try {
      try {
        await this.post(conversationPath, text);
      } catch (err) {
        const e = asError(err);
        yield { type: 'failed', error: e.message, code: e.code, timestamp: new Date().toISOString() };
        return;
      }
      for (;;) {
        if (!queue.length) {
          const idle = await new Promise<boolean>((resolve) => {
            const timer = timeout > 0 ? setTimeout(() => resolve(true), timeout) : null;
            wake = () => {
              if (timer) clearTimeout(timer);
              resolve(false);
            };
          });
          wake = null;
          if (idle && !queue.length) {
            yield { type: 'waiting', timestamp: new Date().toISOString() };
            return;
          }
        }
        const event = queue.shift()!;
        yield event;
        if (event.type === 'done' || event.type === 'failed' || event.type === 'waiting') return;
      }
    } finally {
      stop();
      options.signal?.removeEventListener('abort', onAbort);
    }
  }

  /** Whether the store remembers a conversation with `agent`. */
  remembers(agent: string, options: StartAnonymousOptions = {}): boolean {
    const store = this.storeOf(options);
    return !!store?.get(`${this.keyOf(agent, options)}:conversation`);
  }

  /** Forget the stored session (e.g. a "new chat" button that should start a new session). */
  forget(agent: string, options: StartAnonymousOptions = {}): void {
    const store = this.storeOf(options);
    const key = this.keyOf(agent, options);
    store?.remove(`${key}:session`);
    store?.remove(`${key}:conversation`);
    this.token = null;
  }
}
