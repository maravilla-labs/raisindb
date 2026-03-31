/**
 * Chat types for the RaisinDB JS SDK.
 *
 * Types for conversations, messages, tool calls, and plans.
 *
 * Design principle: the SDK does NOT differentiate between human and AI
 * conversations at the API level. The same `sendMessage()` call works for
 * both — the trigger system handles routing on the server.
 *
 * TODO (future phases):
 * - Reactions: `addReaction(messagePath, emoji)` / `removeReaction()`
 * - Threading: `replyTo` field on ChatMessage, `getThread(messagePath)`
 */

/** Record of a tool call made by the assistant */
export interface ToolCallRecord {
  /** Tool call ID */
  id: string;
  /** Function name */
  name: string;
  /** Function arguments */
  arguments: unknown;
}

/** Task record within a plan */
export interface PlanTask {
  /** Task node ID */
  id: string;
  /** Task title */
  title: string;
  /** Task status */
  status: string;
}

/** Child record attached to a message (thought, tool call detail, plan, cost) */
export interface MessageChild {
  /** Child node ID */
  id: string;
  /** Child node path */
  path?: string;
  /** Child type */
  type: 'thought' | 'tool_call' | 'tool_result' | 'plan' | 'cost';
  /** Content or description */
  content: string;
  /** Tool name (for tool_call / plan type) */
  toolName?: string;
  /** Tool input (for tool_call type) */
  toolInput?: unknown;
  /** Status (for tool_call / plan type) */
  status?: string;
  /** Plan title (for plan type) */
  planTitle?: string;
  /** Plan tasks (for plan type) */
  tasks?: PlanTask[];
  /** Cost record model (for cost type) */
  model?: string;
  /** Cost record provider (for cost type) */
  provider?: string;
  /** Prompt/input token count (for cost type) */
  inputTokens?: number;
  /** Completion/output token count (for cost type) */
  outputTokens?: number;
  /** Total token count (for cost type) */
  totalTokens?: number;
  /** Optional estimated cost in USD (for cost type) */
  costUsd?: number;
  /** Completion duration in milliseconds (for cost type) */
  durationMs?: number;
}

/** Structured message body (canonical content representation) */
export interface MessageBody {
  /** Primary text content */
  content: string;
  /** Raw message text (alias for content, used in outbox routing) */
  message_text?: string;
  /** Thread/conversation ID for routing */
  thread_id?: string;
}

export interface ChatMessage {
  /** Message role */
  role: 'user' | 'assistant' | 'system' | 'tool';
  /** Message text content (shorthand for body.content) */
  content: string;
  /** Structured message body */
  body?: MessageBody;
  /** Agent that produced this message (for handoff tracking) */
  agent?: string;
  /** ISO 8601 timestamp */
  timestamp: string;
  /** Node ID in the database */
  id?: string;
  /** Node path in the database */
  path?: string;
  /** Finish reason from AI (e.g., 'stop', 'tool_calls') */
  finishReason?: string;
  /** Internal orchestration dispatch phase */
  dispatchPhase?: 'pending' | 'queued' | 'awaiting_results' | 'ready_for_model' | 'terminal';
  /** Effective orchestration mode used by backend */
  orchestrationMode?: 'automatic' | 'approve_then_auto' | 'step_by_step' | 'manual';
  /** Continuation round counter */
  orchestrationRound?: number;
  /** Internal terminal reason set by backend orchestration */
  terminalReasonInternal?: string | null;
  /** Tool calls made by the assistant */
  toolCalls?: ToolCallRecord[];
  /** Tool call ID for tool role messages */
  toolCallId?: string;
  /** Child records: thoughts, tool calls, cost info */
  children?: MessageChild[];
  /** Sender identity ID */
  senderId?: string;
  /** Sender display name */
  senderDisplayName?: string;
  /** Message delivery status */
  status?: string;
  /** Message type discriminator */
  messageType?: string;
  /** Structured message metadata (mirrors node properties.data when present) */
  data?: Record<string, unknown>;
  /** Whether this message has been read by the current user */
  readAt?: string;
}

/** Conversation type discriminator */
export type ConversationType = 'ai_chat' | 'direct_message';

/**
 * Chat conversation handle.
 */
export interface Conversation {
  /** Conversation node ID */
  id: string;
  /** Conversation type */
  type: ConversationType;
  /** Agent reference path */
  agentRef?: string;
  /** Participant IDs */
  participants?: string[];
  /** Participant details */
  participantDetails?: Record<string, { display_name: string }>;
  /** Number of unread messages */
  unreadCount?: number;
  /** Last message preview */
  lastMessage?: { content: string; sender_id: string; created_at: string };
  /** Last updated timestamp */
  updatedAt?: string;
  /** Node path */
  conversationPath: string;
  /** Workspace */
  conversationWorkspace: string;
  /** Initial events from creation */
  initialEvents?: ChatEvent[];
}

/** Summary item for listing conversations */
export interface ConversationListItem {
  /** Conversation node ID */
  id: string;
  /** Conversation type */
  type: ConversationType;
  /** Node path */
  conversationPath: string;
  /** Workspace */
  conversationWorkspace: string;
  /** Agent reference path */
  agentRef?: string;
  /** Participant IDs */
  participants?: string[];
  /** Number of unread messages */
  unreadCount?: number;
  /** Last message preview */
  lastMessage?: { content: string; sender_id: string; created_at: string };
  /** Last updated timestamp */
  updatedAt?: string;
}

/**
 * Conversation status, derived from flow instance status.
 */
export type ConversationStatus =
  | 'active'
  | 'completed'
  | 'failed';

/**
 * Events emitted during chat message processing.
 *
 * Works uniformly for both AI and human conversations. AI conversations
 * emit streaming events (text_chunk, tool_call_*); human conversations
 * primarily emit message_delivered and waiting.
 */
export type ChatEvent =
  | ChatTextChunkEvent
  | ChatAssistantMessageEvent
  | ChatWaitingEvent
  | ChatCompletedEvent
  | ChatFailedEvent
  | ChatDoneEvent
  | ChatToolCallStartedEvent
  | ChatToolCallCompletedEvent
  | ChatThoughtChunkEvent
  | ChatConversationCreatedEvent
  | ChatMessageSavedEvent
  | ChatMessageDeliveredEvent
  | ChatLogEvent;

/** Partial text content streamed from the AI agent */
export interface ChatTextChunkEvent {
  type: 'text_chunk';
  /** The text fragment */
  text: string;
  timestamp: string;
}

/** Full assistant message assembled from a step completion */
export interface ChatAssistantMessageEvent {
  type: 'assistant_message';
  /** The complete assistant message */
  message: ChatMessage;
  timestamp: string;
}

/** Chat is waiting for the next user message */
export interface ChatWaitingEvent {
  type: 'waiting';
  /** Session metadata from the backend */
  sessionId?: string;
  /** Current turn count */
  turnCount?: number;
  timestamp: string;
}

/** Chat session has ended normally */
export interface ChatCompletedEvent {
  type: 'completed';
  /** Reason for completion */
  reason?: string;
  /** Final messages from the session */
  messages?: ChatMessage[];
  timestamp: string;
}

/** Chat session has failed */
export interface ChatFailedEvent {
  type: 'failed';
  /** Error description */
  error: string;
  timestamp: string;
}

/** Conversation turn is complete — server closes the SSE stream */
export interface ChatDoneEvent {
  type: 'done';
  /** Conversation path */
  conversationPath?: string;
  /** Final assistant response content (safety net for missed streaming chunks) */
  content?: string;
  /** Message role (typically 'assistant') */
  role?: string;
  /** Sender display name */
  senderDisplayName?: string;
  /** Reason the turn finished (e.g. 'stop', 'tool_calls') */
  finishReason?: string;
  /** Safety-net terminal marker from backend recovery paths */
  recovered?: boolean;
  /** Internal orchestration dispatch phase (when available) */
  dispatchPhase?: string;
  /** Internal terminal reason from orchestration state */
  terminalReasonInternal?: string | null;
  timestamp: string;
}

/** AI tool call has started (for visibility into agent actions) */
export interface ChatToolCallStartedEvent {
  type: 'tool_call_started';
  toolCallId: string;
  functionName: string;
  arguments: unknown;
  timestamp: string;
}

/** AI tool call has completed */
export interface ChatToolCallCompletedEvent {
  type: 'tool_call_completed';
  toolCallId: string;
  result: unknown;
  /** Error message if the tool call failed */
  error?: string;
  /** Execution duration in milliseconds */
  durationMs?: number;
  timestamp: string;
}

/** AI thinking/reasoning chunk */
export interface ChatThoughtChunkEvent {
  type: 'thought_chunk';
  text: string;
  timestamp: string;
}

/** AI conversation node was created */
export interface ChatConversationCreatedEvent {
  type: 'conversation_created';
  conversationPath: string;
  workspace: string;
  timestamp: string;
}

/** AI message was persisted to the node tree */
export interface ChatMessageSavedEvent {
  type: 'message_saved';
  messagePath: string;
  role: string;
  conversationPath: string;
  timestamp: string;
}

/** A message was delivered to the conversation (human or AI) */
export interface ChatMessageDeliveredEvent {
  type: 'message_delivered';
  /** The delivered message */
  message: ChatMessage;
  /** Conversation path */
  conversationPath: string;
  timestamp: string;
}

/** Log message from flow execution or backend handler */
export interface ChatLogEvent {
  type: 'log';
  level: string;
  message: string;
  module?: string;
  nodeId?: string;
  timestamp: string;
}
