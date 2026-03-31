/**
 * Unified ConversationManager for RaisinDB.
 *
 * Merges all conversation operations (previously split across ChatClient,
 * ConversationClient, and FlowClient) into a single API accessed via
 * `db.conversations`.
 *
 * @example
 * ```typescript
 * const db = client.database('my-repo');
 *
 * // List conversations
 * const convos = await db.conversations.list({ type: 'ai_chat' });
 *
 * // Create a new conversation with an agent
 * const convo = await db.conversations.create({ participant: '/agents/support' });
 *
 * // Send a message and stream the response
 * for await (const event of db.conversations.sendMessage(convo.conversationPath, 'Hello!', { stream: true })) {
 *   if (event.type === 'text_chunk') process.stdout.write(event.text);
 * }
 *
 * // Get full message history
 * const messages = await db.conversations.getMessages(convo.conversationPath);
 * ```
 */

import type { AuthManager } from './auth';
import type {
  ChatEvent,
  ChatMessage,
  Conversation,
  ConversationType,
  ConversationListItem,
  ToolCallRecord,
  MessageChild,
  PlanTask,
} from './types/chat';
import { logger } from './logger';
import { SSEClient } from './streaming/sse-client';
import { RaisinAbortError, RaisinTimeoutError, classifyHttpError } from './errors';
import type { SqlResult } from './protocol';
import { isRecoveredDoneEvent } from './utils/chat-events';

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/** Options for listing conversations */
export interface ListConversationsOptions {
  /** Filter by conversation type */
  type?: ConversationType;
  /** Maximum number of conversations to return */
  limit?: number;
  /** Abort signal */
  signal?: AbortSignal;
}

/** Options for creating a conversation */
export interface CreateConversationOptions {
  /**
   * Participant identifier — auto-detects agents vs users.
   *
   * Agent paths (e.g. `/agents/support` or `agent:support`) create an
   * `ai_chat` conversation. Anything else creates a `direct_message`.
   */
  participant: string;
  /** Optional subject line */
  subject?: string;
  /** Additional input data passed to the conversation */
  input?: Record<string, unknown>;
  /** Abort signal */
  signal?: AbortSignal;
}

/** Options for sending a message */
export interface SendMessageOptions {
  /** Whether to stream events via SSE (default: true) */
  stream?: boolean;
  /** Abort signal */
  signal?: AbortSignal;
}

/** Handle for a persistent conversation SSE subscription */
export interface ConversationSubscription {
  /** Close the SSE connection */
  unsubscribe(): void;
  /** Wait until the SSE connection is established */
  waitUntilConnected(): Promise<void>;
}

/** Immediate receipt for queued plan actions (non-blocking). */
export interface PlanActionReceipt {
  accepted: boolean;
  action: 'approve' | 'reject';
  actionId: string;
  planPath: string;
  executionId: string;
  jobId: string;
  status?: string;
}

/** Options for plan actions */
export interface PlanActionOptions {
  actionId?: string;
  requestTimeoutMs?: number;
}

/** Options for constructing the ConversationManager */
export interface ConversationManagerOptions {
  /** Custom fetch implementation */
  fetch?: typeof fetch;
  /** Request timeout in milliseconds (default: 60000) */
  requestTimeout?: number;
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/** Internal type for SQL query response */
interface SqlQueryResponse {
  columns?: string[];
  rows?: Record<string, unknown>[];
}

interface SqlInvokeReceipt {
  execution_id?: string;
  job_id?: string;
}

// ---------------------------------------------------------------------------
// ConversationManager
// ---------------------------------------------------------------------------

/**
 * Unified conversation manager for RaisinDB.
 *
 * Handles the full lifecycle: listing, creating, opening, deleting, messaging,
 * streaming, and plan approval/rejection for all conversation types.
 */
export class ConversationManager {
  private baseUrl: string;
  private repository: string;
  private authManager: AuthManager;
  private fetchImpl: typeof fetch;
  private requestTimeout: number;

  private executeSql?: (query: string, params?: unknown[]) => Promise<SqlResult>;
  private cachedUserId?: string;
  private cachedUserHome?: string;

  constructor(
    baseUrl: string,
    repository: string,
    authManager: AuthManager,
    options: ConversationManagerOptions = {},
    executeSql?: (query: string, params?: unknown[]) => Promise<SqlResult>,
  ) {
    this.baseUrl = baseUrl.replace(/\/$/, '');
    this.repository = repository;
    this.authManager = authManager;
    this.fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
    this.requestTimeout = options.requestTimeout ?? 60000;
    this.executeSql = executeSql;
  }

  // ==========================================================================
  // User context
  // ==========================================================================

  private async ensureCurrentUser(): Promise<{ userId: string; userHome: string }> {
    if (this.cachedUserId && this.cachedUserHome) {
      return { userId: this.cachedUserId, userHome: this.cachedUserHome };
    }
    const result = await this.sqlQuery(
      `SELECT RAISIN_CURRENT_USER()->>'path'::String as home, RAISIN_CURRENT_USER()->>'id'::String as user_id`,
    );
    if (!result?.rows?.[0]) throw new Error('User not authenticated');
    const row = result.rows[0] as Record<string, unknown>;
    this.cachedUserHome = row.home as string;
    this.cachedUserId = (row.user_id as string) || this.cachedUserHome;
    if (!this.cachedUserHome) throw new Error('User home path not found');
    return { userId: this.cachedUserId, userHome: this.cachedUserHome };
  }

  // ==========================================================================
  // Conversation listing
  // ==========================================================================

  /**
   * List conversations for the current user.
   */
  async list(options?: ListConversationsOptions): Promise<ConversationListItem[]> {
    const workspace = 'raisin:access_control';
    const { userHome } = await this.ensureCurrentUser();

    const sql = `
      SELECT id, path, properties, created_at, updated_at
      FROM '${workspace}'
      WHERE node_type = 'raisin:Conversation'
        AND DESCENDANT_OF($1)
        ${options?.type ? "AND properties->>'conversation_type'::String = $2" : ''}
      ORDER BY updated_at DESC
      ${options?.limit ? `LIMIT ${options.limit}` : ''}
    `;
    const params = options?.type
      ? [`${userHome}/inbox/chats`, options.type]
      : [`${userHome}/inbox/chats`];
    const response = await this.sqlQuery(sql, params);

    if (!response || !Array.isArray(response.rows)) return [];

    const rows = response.rows as unknown as Record<string, unknown>[];
    return rows.map((row: any) => {
      const props = row.properties ?? {};
      return {
        id: row.id,
        type: (props.conversation_type as ConversationType) ?? 'ai_chat',
        conversationPath: row.path,
        conversationWorkspace: workspace,
        agentRef: props.agent_ref,
        participants: props.participants,
        unreadCount: props.unread_count,
        lastMessage: props.last_message,
        updatedAt: row.updated_at ?? row.created_at,
      } satisfies ConversationListItem;
    });
  }

  // ==========================================================================
  // Conversation lifecycle
  // ==========================================================================

  /**
   * Create a new conversation.
   *
   * `participant` auto-detects whether the target is an agent or a user:
   * - Paths starting with `/agents/` or `agent:` create an `ai_chat`
   * - Everything else creates a `direct_message`
   */
  async create(options: CreateConversationOptions): Promise<Conversation> {
    logger.info('[ConversationManager] Creating conversation', { participant: options.participant });
    const conversationId = `chat-${crypto.randomUUID()}`;
    const workspace = 'raisin:access_control';

    const recipientId = this.normalizeRecipientId(options.participant);
    const isAgent = recipientId.startsWith('agent:');
    const conversationType: ConversationType = isAgent ? 'ai_chat' : 'direct_message';
    const agentPath = isAgent ? this.normalizeAgentPath(options.participant) : undefined;

    const { userId, userHome } = await this.ensureCurrentUser();
    const conversationPath = `${userHome}/inbox/chats/${conversationId}`;

    await this.ensureFolderNode(workspace, `${userHome}/inbox`, 'Inbox');
    await this.ensureFolderNode(workspace, `${userHome}/inbox/chats`, 'Chats');

    const participants = [userId, recipientId].filter(Boolean);

    const properties: Record<string, unknown> = {
      conversation_type: conversationType,
      conversation_id: conversationId,
      subject: options.subject || 'Chat',
      participants,
      stream_channel: `chat:${conversationId}`,
      unread_count: 0,
      ...(agentPath ? {
        agent_ref: { 'raisin:path': agentPath, 'raisin:workspace': 'functions' },
      } : {}),
      ...(options.input ?? {}),
    };

    const sql = `
      INSERT INTO '${workspace}' (path, node_type, properties)
      VALUES ($1, 'raisin:Conversation', $2::jsonb)
      RETURNING id, path
    `;
    const result = await this.sqlQuery(sql, [conversationPath, JSON.stringify(properties)]);

    if (!result?.rows?.[0]) throw new Error('Failed to create conversation');

    const resultRow = result.rows[0] as Record<string, unknown>;
    const now = new Date().toISOString();

    logger.info('[ConversationManager] Conversation created', { path: conversationPath });

    return {
      id: resultRow.id as string,
      type: conversationType,
      agentRef: agentPath,
      participants,
      conversationPath,
      conversationWorkspace: workspace,
      initialEvents: [
        { type: 'conversation_created', conversationPath, workspace, timestamp: now },
        { type: 'waiting', timestamp: now },
      ],
    };
  }

  /**
   * Open an existing conversation by path.
   * Returns null if not found.
   */
  async open(conversationPath: string): Promise<Conversation | null> {
    const workspace = 'raisin:access_control';
    try {
      const sql = `SELECT id, path, properties, created_at FROM '${workspace}' WHERE path = $1 AND node_type = 'raisin:Conversation' LIMIT 1`;
      const response = await this.sqlQuery(sql, [conversationPath]);

      if (!response || !Array.isArray(response.rows) || response.rows.length === 0) {
        return null;
      }

      const rows = response.rows as unknown as Record<string, unknown>[];
      const row = rows[0];
      const props = (row.properties ?? {}) as Record<string, unknown>;

      return {
        id: row.id as string,
        type: (props.conversation_type as ConversationType) ?? 'ai_chat',
        agentRef: props.agent_ref as string | undefined,
        participants: props.participants as string[] | undefined,
        participantDetails: props.participant_details as Record<string, { display_name: string }> | undefined,
        unreadCount: props.unread_count as number | undefined,
        lastMessage: props.last_message as { content: string; sender_id: string; created_at: string } | undefined,
        conversationPath,
        conversationWorkspace: workspace,
        initialEvents: [{ type: 'waiting', timestamp: new Date().toISOString() }],
      };
    } catch {
      return null;
    }
  }

  /**
   * Delete a conversation and all its children.
   */
  async delete(conversationPath: string): Promise<void> {
    const workspace = 'raisin:access_control';
    await this.sqlQuery(
      `DELETE FROM '${workspace}' WHERE path = $1 OR path LIKE $2`,
      [conversationPath, `${conversationPath}/%`],
    );
  }

  /**
   * Mark a conversation as read by setting unread_count to 0.
   */
  async markAsRead(conversationPath: string): Promise<void> {
    const workspace = 'raisin:access_control';
    await this.sqlQuery(
      `UPDATE '${workspace}' SET properties = jsonb_set(properties, '{unread_count}', '0'::jsonb) WHERE path = $1 AND node_type = 'raisin:Conversation'`,
      [conversationPath],
    );
  }

  /**
   * Mark a single message as read by the current user.
   */
  async markMessageAsRead(messagePath: string): Promise<void> {
    const workspace = 'raisin:access_control';
    const now = new Date().toISOString();
    await this.sqlQuery(
      `UPDATE '${workspace}' SET properties = jsonb_set(properties, '{read_at}', $2::jsonb) WHERE path = $1 AND node_type = 'raisin:Message'`,
      [messagePath, JSON.stringify(now)],
    );
  }

  // ==========================================================================
  // Messaging
  // ==========================================================================

  /**
   * Get the full message history for a conversation.
   */
  async getMessages(conversationPath: string): Promise<ChatMessage[]> {
    return this.getMessagesFromNodeTree(conversationPath);
  }

  /**
   * Send a message to a conversation.
   *
   * Returns an `AsyncIterable<ChatEvent>` for streaming. By default streams
   * via SSE. Set `options.stream = false` to fire-and-forget.
   */
  async *sendMessage(
    conversationPath: string,
    content: string,
    options?: SendMessageOptions,
  ): AsyncIterable<ChatEvent> {
    const shouldStream = options?.stream !== false;

    logger.debug('[ConversationManager] Starting SSE stream for message send');

    if (!shouldStream) {
      await this.createUserMessage(conversationPath, content);
      yield { type: 'waiting', timestamp: new Date().toISOString() };
      return;
    }

    const workspace = 'raisin:access_control';
    const streamTarget = await this.resolveConversationStreamTarget(conversationPath, workspace);

    // Subscribe to SSE BEFORE creating the message so we don't miss events.
    const sseUrl = `${this.baseUrl}/api/conversations/${this.repository}/events`;
    const headers: Record<string, string> = {};
    const token = this.authManager.getAccessToken();
    if (token) headers['Authorization'] = `Bearer ${token}`;

    const sse = new SSEClient<ChatEvent>(sseUrl, {
      headers,
      eventTypes: ['conversation-event', 'message'],
      reconnect: { enabled: false },
      fetch: this.fetchImpl,
      signal: options?.signal,
      method: 'POST',
      body: { channel: streamTarget.channel, path: streamTarget.path },
    });

    const iterator = sse[Symbol.asyncIterator]();
    let pendingFirst: Promise<IteratorResult<{ type: string; data: ChatEvent }>> | null = iterator.next();

    try {
      await sse.waitUntilConnected();
    } catch {
      pendingFirst = null;
      sse.close();
      logger.debug('SSE connection failed, falling back to create-only');
    }

    // Create the user message node
    try {
      await this.createUserMessage(conversationPath, content);
    } catch (error) {
      logger.error('[ConversationManager] Failed to create user message', error);
      sse.close();
      throw error;
    }

    // Stream events from SSE
    if (pendingFirst) {
      try {
        while (true) {
          let result;
          if (pendingFirst) {
            result = await pendingFirst;
            pendingFirst = null;
          } else {
            result = await iterator.next();
          }

          if (result.done) break;

          const event = result.value.data;
          if (isRecoveredDoneEvent(event)) {
            logger.debug('[ConversationManager] Ignoring recovered done event while turn may still be active');
            continue;
          }
          if (event.type === 'done' && event.dispatchPhase && event.dispatchPhase !== 'terminal') {
            logger.debug('[ConversationManager] Ignoring non-terminal done event');
            continue;
          }
          yield event;

          if (event.type === 'done' || event.type === 'completed' || event.type === 'failed' || event.type === 'waiting') {
            return;
          }
        }
      } finally {
        sse.close();
      }
    }

    yield { type: 'waiting', timestamp: new Date().toISOString() };
  }

  /**
   * Create a user message in a conversation.
   *
   * The message node triggers server-side handlers (via triggers);
   * events arrive via the persistent subscription.
   */
  async createUserMessage(conversationPath: string, content: string): Promise<void> {
    logger.debug('[ConversationManager] Creating user message', { path: conversationPath });
    const workspace = 'raisin:access_control';
    const { userId, userHome } = await this.ensureCurrentUser();
    const messageId = crypto.randomUUID();

    const properties = await this.loadConversationProperties(conversationPath, workspace);
    const participants = Array.isArray(properties?.participants)
      ? (properties.participants as unknown[])
      : [];
    let recipientId = participants.find((p) =>
      typeof p === 'string' && p.length > 0 && p !== userId && p !== userHome
    ) as string | undefined;
    if (recipientId) recipientId = this.normalizeRecipientId(recipientId);

    const conversationId = (typeof properties?.conversation_id === 'string' && properties.conversation_id.length > 0)
      ? (properties.conversation_id as string)
      : conversationPath.split('/').pop() || '';

    if (!recipientId) {
      throw new Error(`Conversation recipient not found for ${conversationPath}`);
    }

    const messagePath = `${userHome}/outbox/msg-${messageId}`;
    const outboxProperties = {
      role: 'user',
      message_type: 'chat',
      status: 'pending',
      sender_id: userId,
      sender_path: userHome,
      recipient_id: recipientId,
      body: { content, message_text: content, thread_id: conversationId },
      conversation_id: conversationId,
      client_id: messageId,
      created_at: new Date().toISOString(),
    };

    await this.sqlQuery(
      `INSERT INTO '${workspace}' (path, node_type, properties) VALUES ($1, 'raisin:Message', $2::jsonb) RETURNING id`,
      [messagePath, JSON.stringify(outboxProperties)],
    );
  }

  // ==========================================================================
  // Persistent subscription
  // ==========================================================================

  /**
   * Subscribe to a conversation's events via a persistent SSE connection.
   *
   * Keeps a long-lived connection that survives across turns. Useful for
   * receiving async events between turns (background tool results, agent-
   * initiated messages, etc.). Auto-reconnects on disconnect.
   */
  subscribe(
    conversationPath: string,
    onEvent: (event: ChatEvent) => void,
    options?: { signal?: AbortSignal },
  ): ConversationSubscription {
    logger.info('[ConversationManager] Subscribing to SSE', { path: conversationPath });
    const sseUrl = `${this.baseUrl}/api/conversations/${this.repository}/events`;
    const headers: Record<string, string> = {};
    const token = this.authManager.getAccessToken();
    if (token) headers['Authorization'] = `Bearer ${token}`;

    let sse: SSEClient<ChatEvent> | null = null;
    let unsubscribed = false;

    const setupPromise = (async () => {
      const streamTarget = await this.resolveConversationStreamTarget(conversationPath);
      if (unsubscribed) return null;

      sse = new SSEClient<ChatEvent>(sseUrl, {
        headers,
        eventTypes: ['conversation-event', 'message'],
        reconnect: { enabled: true },
        fetch: this.fetchImpl,
        signal: options?.signal,
        method: 'POST',
        body: { channel: streamTarget.channel, path: streamTarget.path },
      });

      sse.connect(
        (sseEvent) => onEvent(sseEvent.data),
        (error) => logger.error('Conversation SSE error:', error.message),
      );
      return sse;
    })();

    return {
      unsubscribe: () => {
        unsubscribed = true;
        sse?.close();
      },
      waitUntilConnected: async () => {
        const activeSse = await setupPromise;
        if (!activeSse) return;
        await activeSse.waitUntilConnected();
      },
    };
  }

  // ==========================================================================
  // Plan actions
  // ==========================================================================

  /**
   * Approve a pending plan without waiting for completion.
   * Returns an enqueue receipt; final state is observed via conversation events.
   */
  async approvePlan(planPath: string, options?: PlanActionOptions): Promise<PlanActionReceipt> {
    logger.info('[ConversationManager] Plan action: approve', { path: planPath });
    return this.invokePlanAction('approve', planPath, undefined, options);
  }

  /**
   * Reject a pending plan without waiting for completion.
   */
  async rejectPlan(planPath: string, feedback?: string, options?: PlanActionOptions): Promise<PlanActionReceipt> {
    logger.info('[ConversationManager] Plan action: reject', { path: planPath });
    return this.invokePlanAction('reject', planPath, feedback, options);
  }

  // ==========================================================================
  // One-shot convenience
  // ==========================================================================

  /**
   * One-shot chat: create a conversation, send a message, return response.
   */
  async chat(
    participant: string,
    message: string,
    options?: { signal?: AbortSignal; input?: Record<string, unknown> },
  ): Promise<{ response: string; conversationPath: string }> {
    const convo = await this.create({
      participant,
      input: options?.input,
      signal: options?.signal,
    });

    let response = '';
    for await (const event of this.sendMessage(convo.conversationPath, message, {
      signal: options?.signal,
      stream: true,
    })) {
      if (event.type === 'text_chunk') response += event.text;
    }

    return { response, conversationPath: convo.conversationPath };
  }

  // ==========================================================================
  // Health checks (used by ConversationStore watchdog)
  // ==========================================================================

  /**
   * Query pending (in-flight) tool calls from the node tree.
   * Used by ConversationStore to rebuild `_activeToolCalls` on reload.
   */
  async getActiveToolCalls(
    conversationPath: string,
  ): Promise<{ id: string; name: string; status: string }[]> {
    const workspace = 'raisin:access_control';
    const sql = `
      SELECT id, properties->>'function_name'::String as name, properties->>'status'::String as status
      FROM '${workspace}'
      WHERE DESCENDANT_OF($1)
        AND node_type = 'raisin:AIToolCall'
        AND properties->>'status'::String IN ('pending', 'running')
      ORDER BY created_at ASC
    `;
    try {
      const response = await this.sqlQuery(sql, [conversationPath]);
      if (!response?.rows || !Array.isArray(response.rows)) return [];
      return (response.rows as any[]).map((row: any) => ({
        id: row.id ?? '',
        name: row.name ?? '',
        status: row.status ?? 'pending',
      }));
    } catch {
      return [];
    }
  }

  /**
   * Check the health of the latest assistant turn.
   * Returns 'streaming' if the turn appears active, 'done' if it's finished,
   * or 'unknown' if no assistant message is found.
   */
  async checkTurnHealth(
    conversationPath: string,
  ): Promise<'streaming' | 'done' | 'unknown'> {
    const workspace = 'raisin:access_control';
    const sql = `
      SELECT properties->>'status'::String as status,
             properties->>'finish_reason'::String as finish_reason,
             properties->>'dispatch_phase'::String as dispatch_phase
      FROM '${workspace}'
      WHERE CHILD_OF($1)
        AND node_type = 'raisin:Message'
        AND properties->>'role'::String = 'assistant'
      ORDER BY created_at DESC LIMIT 1
    `;
    try {
      const response = await this.sqlQuery(sql, [conversationPath]);
      if (!response?.rows || !Array.isArray(response.rows) || response.rows.length === 0) {
        return 'unknown';
      }
      const row = response.rows[0] as Record<string, unknown>;
      const finishReason = row.finish_reason as string | undefined;
      const dispatchPhase = row.dispatch_phase as string | undefined;
      if (dispatchPhase) {
        return dispatchPhase === 'terminal' ? 'done' : 'streaming';
      }
      // If the message has a terminal finish_reason, the turn is done
      if (finishReason && finishReason !== 'tool_calls') {
        return 'done';
      }
      return 'streaming';
    } catch {
      return 'unknown';
    }
  }

  // ==========================================================================
  // Internal: message loading from node tree
  // ==========================================================================

  private async getMessagesFromNodeTree(conversationPath: string, workspace?: string): Promise<ChatMessage[]> {
    const ws = workspace ?? 'raisin:access_control';

    const [msgResponse, childResponse] = await Promise.all([
      this.sqlQuery(
        `SELECT id, path, name, properties, created_at FROM '${ws}' WHERE CHILD_OF('${conversationPath}') AND node_type = 'raisin:Message' AND (properties->>'message_type'::String IS NULL OR properties->>'message_type'::String NOT IN ('ai_tool_call', 'ai_tool_result', 'ai_thought')) ORDER BY created_at ASC`,
      ),
      this.sqlQuery(
        `SELECT id, path, name, node_type, properties, created_at FROM '${ws}' WHERE DESCENDANT_OF('${conversationPath}') AND node_type IN ('raisin:AIPlan', 'raisin:AITask', 'raisin:AIThought', 'raisin:AICostRecord') ORDER BY created_at ASC`,
      ),
    ]);

    if (!msgResponse || !Array.isArray(msgResponse.rows)) return [];

    // Group child nodes by parent message path; collect tasks separately
    const childrenByMsg = new Map<string, MessageChild[]>();
    const tasksByPlan = new Map<string, PlanTask[]>();
    const planByPath = new Map<string, MessageChild>();

    for (const row of ((childResponse?.rows ?? []) as any[])) {
      const parts = (row.path as string).split('/');

      if (row.node_type === 'raisin:AITask') {
        const planPath = parts.slice(0, -1).join('/');
        const tasks = tasksByPlan.get(planPath) ?? [];
        tasks.push({
          id: row.id,
          title: row.properties?.title ?? row.name,
          status: row.properties?.status ?? 'pending',
        });
        tasksByPlan.set(planPath, tasks);
        continue;
      }

      const msgPath = parts.slice(0, -1).join('/');
      const children = childrenByMsg.get(msgPath) ?? [];
      const child: MessageChild = {
        id: row.id,
        path: row.path,
        type:
          row.node_type === 'raisin:AIPlan'
            ? 'plan'
            : row.node_type === 'raisin:AICostRecord'
              ? 'cost'
              : 'thought',
        content: row.properties?.content ?? '',
        status: row.properties?.status ?? 'completed',
      };
      if (child.type === 'plan') {
        child.planTitle = row.properties?.title;
        child.toolName = 'create_plan';
        planByPath.set(row.path, child);
      } else if (child.type === 'cost') {
        const inputTokens = Number(row.properties?.input_tokens ?? 0);
        const outputTokens = Number(row.properties?.output_tokens ?? 0);
        const totalTokens = Number(row.properties?.total_tokens ?? (inputTokens + outputTokens));
        const costUsdRaw = Number(row.properties?.cost_usd);
        const durationMsRaw = Number(row.properties?.duration_ms);
        child.model = row.properties?.model ?? undefined;
        child.provider = row.properties?.provider ?? undefined;
        child.inputTokens = Number.isFinite(inputTokens) ? inputTokens : 0;
        child.outputTokens = Number.isFinite(outputTokens) ? outputTokens : 0;
        child.totalTokens = Number.isFinite(totalTokens) ? totalTokens : 0;
        child.costUsd = Number.isFinite(costUsdRaw) ? costUsdRaw : undefined;
        child.durationMs = Number.isFinite(durationMsRaw) ? durationMsRaw : undefined;
        child.content = `tokens: ${child.totalTokens ?? 0}`;
      }
      children.push(child);
      childrenByMsg.set(msgPath, children);
    }

    for (const [planPath, tasks] of tasksByPlan) {
      const plan = planByPath.get(planPath);
      if (plan) plan.tasks = tasks;
    }

    // Filter out empty intermediate continuation messages and tool-echo messages
    const filteredRows = (msgResponse.rows as any[]).filter((row: any) => {
      const props = row.properties ?? {};
      const msgContent = props.content
        ?? (typeof props.body === 'string' ? props.body : props.body?.content)
        ?? '';
      const finishReason = props.finish_reason ?? '';
      const msgType = props.message_type ?? '';

      // Skip empty messages with finish_reason=tool_calls (intermediate continuations)
      if (!msgContent.trim() && finishReason === 'tool_calls') return false;

      // Skip tool-echo messages like "Calling update-task"
      if (msgType === 'chat' && /^Calling\s+[\w-]+\s*$/.test(msgContent.trim())) return false;

      return true;
    });

    return filteredRows.map((row: any) => {
      const props = row.properties ?? {};
      const message: ChatMessage = {
        role: props.role ?? 'assistant',
        content: props.content
          ?? (typeof props.body === 'string' ? props.body : props.body?.content)
          ?? props.data?.content
          ?? '',
        timestamp: row.created_at ?? new Date().toISOString(),
        id: row.id,
        path: row.path,
        children: childrenByMsg.get(row.path),
      };

      if (props.agent) message.agent = props.agent;
      if (props.finish_reason) message.finishReason = props.finish_reason;
      if (typeof props.dispatch_phase === 'string') {
        message.dispatchPhase = props.dispatch_phase as ChatMessage['dispatchPhase'];
      }
      if (typeof props.orchestration_mode === 'string') {
        message.orchestrationMode = props.orchestration_mode as ChatMessage['orchestrationMode'];
      }
      if (typeof props.orchestration_round === 'number') {
        message.orchestrationRound = props.orchestration_round;
      }
      if (props.terminal_reason_internal !== undefined) {
        message.terminalReasonInternal = (props.terminal_reason_internal ?? null) as string | null;
      }
      if (props.sender_id) message.senderId = props.sender_id;
      if (props.sender_display_name) message.senderDisplayName = props.sender_display_name;
      if (props.status) message.status = props.status;
      if (props.message_type) message.messageType = props.message_type;
      if (props.data && typeof props.data === 'object' && !Array.isArray(props.data)) {
        message.data = props.data as Record<string, unknown>;
      }

      if (Array.isArray(props.tool_calls) && props.tool_calls.length > 0) {
        message.toolCalls = props.tool_calls.map((tc: any) => ({
          id: tc.id ?? '',
          name: tc.name ?? tc.function?.name ?? '',
          arguments: tc.arguments ?? tc.function?.arguments,
        } satisfies ToolCallRecord));
      }

      if (props.tool_call_id) message.toolCallId = props.tool_call_id;

      if (Array.isArray(props.children) && props.children.length > 0) {
        const inlineChildren: MessageChild[] = props.children.map((c: any) => ({
          id: c.id ?? '',
          path: c.path,
          type: c.type ?? 'thought',
          content: c.content ?? '',
          toolName: c.tool_name,
          toolInput: c.tool_input,
          status: c.status,
          model: c.model,
          provider: c.provider,
          inputTokens: c.input_tokens,
          outputTokens: c.output_tokens,
          totalTokens: c.total_tokens,
          costUsd: c.cost_usd,
          durationMs: c.duration_ms,
        } satisfies MessageChild));
        message.children = [...(message.children ?? []), ...inlineChildren];
      }

      return message;
    });
  }

  // ==========================================================================
  // Internal: plan action
  // ==========================================================================

  private async invokePlanAction(
    action: 'approve' | 'reject',
    planPath: string,
    feedback: string | undefined,
    options?: PlanActionOptions,
  ): Promise<PlanActionReceipt> {
    const actionId = options?.actionId?.trim() || crypto.randomUUID();
    const payload: Record<string, unknown> = {
      action,
      plan_path: planPath,
      plan_action_id: actionId,
    };
    if (action === 'reject' && feedback?.trim()) {
      payload.feedback = feedback.trim();
    }

    const query = await this.sqlQuery(
      `SELECT INVOKE($1, $2::jsonb, $3) AS invoke`,
      ['/lib/raisin/ai/plan-approval-handler', JSON.stringify(payload), 'functions'],
    );
    const row = query?.rows?.[0] as Record<string, unknown> | undefined;
    const invoke = (row?.invoke ?? row?.INVOKE ?? null) as SqlInvokeReceipt | string | null;

    let receipt: SqlInvokeReceipt | null = null;
    if (invoke && typeof invoke === 'object') {
      receipt = invoke;
    } else if (typeof invoke === 'string' && invoke.trim()) {
      try { receipt = JSON.parse(invoke) as SqlInvokeReceipt; }
      catch { throw new Error(`Invalid SQL INVOKE receipt payload: ${invoke}`); }
    }

    if (!receipt?.job_id || !receipt?.execution_id) {
      throw new Error('Plan action was not accepted by the backend queue');
    }

    return {
      accepted: true,
      action,
      actionId,
      planPath,
      executionId: receipt.execution_id,
      jobId: receipt.job_id,
      status: 'scheduled',
    };
  }

  // ==========================================================================
  // Internal: HTTP + SQL helpers
  // ==========================================================================

  private async httpRequest<T>(options: {
    method: string;
    path: string;
    body?: unknown;
    signal?: AbortSignal;
    timeoutMs?: number;
  }): Promise<T> {
    const url = `${this.baseUrl}${options.path}`;
    const headers: Record<string, string> = { 'Content-Type': 'application/json' };

    const token = this.authManager.getAccessToken();
    if (token) headers['Authorization'] = `Bearer ${token}`;

    const controller = new AbortController();
    const requestTimeout = options.timeoutMs ?? this.requestTimeout;
    const timeoutId = setTimeout(() => controller.abort(), requestTimeout);

    if (options.signal) {
      if (options.signal.aborted) {
        clearTimeout(timeoutId);
        throw new RaisinAbortError();
      }
      options.signal.addEventListener('abort', () => controller.abort(), { once: true });
    }

    try {
      const response = await this.fetchImpl(url, {
        method: options.method,
        headers,
        body: options.body ? JSON.stringify(options.body) : undefined,
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      if (!response.ok) {
        const errorText = await response.text();
        let errorMessage: string;
        try {
          const errorJson = JSON.parse(errorText);
          errorMessage = errorJson.message || errorJson.error || errorText;
        } catch { errorMessage = errorText || `HTTP ${response.status}: ${response.statusText}`; }
        throw classifyHttpError(response.status, errorMessage);
      }

      const contentType = response.headers.get('content-type');
      if (contentType?.includes('application/json')) return (await response.json()) as T;
      return undefined as unknown as T;
    } catch (error) {
      clearTimeout(timeoutId);
      if (error instanceof RaisinAbortError || error instanceof RaisinTimeoutError) throw error;
      if (error instanceof Error && error.name === 'AbortError') {
        if (options.signal?.aborted) throw new RaisinAbortError();
        throw new RaisinTimeoutError(`Request timeout after ${requestTimeout}ms`, 'REQUEST_TIMEOUT', requestTimeout);
      }
      throw error;
    }
  }

  private async sqlQuery(sql: string, params?: unknown[]): Promise<SqlQueryResponse> {
    if (this.executeSql) {
      const result = await this.executeSql(sql, params);
      return { columns: result.columns, rows: result.rows as unknown as Record<string, unknown>[] };
    }
    return this.httpRequest<SqlQueryResponse>({
      method: 'POST',
      path: `/api/sql/${this.repository}/query`,
      body: { sql, params },
    });
  }

  private async resolveConversationStreamTarget(
    conversationPath: string,
    workspace = 'raisin:access_control',
  ): Promise<{ channel: string; path: string }> {
    try {
      const result = await this.sqlQuery(
        `SELECT properties FROM '${workspace}' WHERE path = $1 AND node_type = 'raisin:Conversation' LIMIT 1`,
        [conversationPath],
      );
      const row = result?.rows?.[0] as Record<string, unknown> | undefined;
      const properties = (row?.properties ?? {}) as Record<string, unknown>;
      const streamChannel = properties.stream_channel;
      if (typeof streamChannel === 'string' && streamChannel.length > 0) {
        return { channel: streamChannel, path: conversationPath };
      }
      const conversationId = typeof properties.conversation_id === 'string'
        ? properties.conversation_id
        : (conversationPath.split('/').pop() || '');
      if (conversationId) return { channel: `chat:${conversationId}`, path: conversationPath };
    } catch (error) {
      logger.debug('Failed to resolve conversation stream channel, deriving from path', error);
    }
    const conversationId = conversationPath.split('/').pop() || conversationPath;
    return { channel: `chat:${conversationId}`, path: conversationPath };
  }

  private async loadConversationProperties(
    conversationPath: string,
    workspace = 'raisin:access_control',
  ): Promise<Record<string, unknown> | null> {
    const result = await this.sqlQuery(
      `SELECT properties FROM '${workspace}' WHERE path = $1 AND node_type = 'raisin:Conversation' LIMIT 1`,
      [conversationPath],
    );
    const row = result?.rows?.[0] as Record<string, unknown> | undefined;
    if (!row) return null;
    return (row.properties ?? null) as Record<string, unknown> | null;
  }

  private normalizeRecipientId(participant: string): string {
    if (participant.startsWith('agent:')) return participant;
    if (participant.startsWith('/agents/')) {
      const parts = participant.split('/').filter(Boolean);
      if (parts.length >= 2) return `agent:${parts[1]}`;
    }
    return participant;
  }

  private normalizeAgentPath(participant: string): string {
    if (participant.startsWith('/agents/')) return participant;
    if (participant.startsWith('agent:')) return `/agents/${participant.slice('agent:'.length)}`;
    return participant;
  }

  private async ensureFolderNode(workspace: string, path: string, title: string): Promise<void> {
    await this.sqlQuery(
      `UPSERT INTO '${workspace}' (path, node_type, properties) VALUES ($1, 'raisin:Folder', $2::jsonb)`,
      [path, JSON.stringify({ title })],
    );
  }
}
