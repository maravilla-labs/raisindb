/**
 * RaisinDB JavaScript/TypeScript Client SDK
 *
 * A universal WebSocket client for RaisinDB that works in browser, Node.js, and serverless environments.
 *
 * @example
 * ```typescript
 * import { RaisinClient } from '@raisindb/client';
 *
 * const client = new RaisinClient('raisin://localhost:8080/sys/default');
 * await client.connect();
 * await client.authenticate({ username: 'admin', password: 'password' });
 *
 * const db = client.database('my_repo');
 * const ws = db.workspace('content');
 *
 * const node = await ws.nodes().create({
 *   type: 'Page',
 *   path: '/home',
 *   properties: { title: 'Home Page' }
 * });
 * ```
 */

// Main client
export { RaisinClient } from './client';
export type {
  ClientOptions,
  ClientMode,
  CurrentUser,
  UserNode,
  // Auth state change types (Firebase/Supabase-compatible)
  AuthEvent,
  AuthStateChange,
  AuthStateChangeCallback,
  UserChangeEvent,
  UserChangeCallback,
} from './client';

// HTTP client for SSR
export { RaisinHttpClient, HttpDatabase, HttpWorkspaceClient } from './http-client';
export type {
  HttpClientOptions,
  CurrentUser as HttpCurrentUser,
  UserNode as HttpUserNode,
  SignAssetOptions,
  SignedAssetUrl,
} from './http-client';

// Database and workspace
export { Database } from './database';
export type { DatabaseHttpOptions } from './database';
export { WorkspaceClient, WorkspaceManager } from './workspace';
export type { WorkspaceCreateOptions, WorkspaceUpdateOptions } from './workspace';

// Node operations
export { NodeOperations } from './nodes';
export type { NodeCreateOptions, NodeUpdateOptions, NodeQueryOptions } from './nodes';

// Transactions
export { Transaction } from './transactions';
export type { TransactionOptions } from './transactions';

// Node builder and helpers
export { NodeBuilder, NodeHelpers } from './node-builder';

// Management operations
export { NodeTypes } from './node-types';
export { Archetypes } from './archetypes';
export { ElementTypes } from './element-types';
export { Branches } from './branches';
export { Tags } from './tags';

// SQL queries
export { SqlQuery, createSqlHandler } from './sql';

// Events
export { EventHandler, EventSubscriptions } from './events';
export type { EventCallback, Subscription } from './events';

// Event type constants
export { NodeEventType, AllNodeEventTypes } from './constants';
export type { NodeEventTypeValue } from './constants';

// Connection
export { Connection, ConnectionState } from './connection';
export type { ConnectionOptions } from './connection';

// Authentication
export { AuthManager, MemoryTokenStorage, LocalStorageTokenStorage } from './auth';
export type {
  Credentials,
  TokenStorage,
  IdentityUser,
  IdentityAuthResponse,
  IdentityAuthError,
} from './auth';

// Protocol types
export type {
  RequestEnvelope,
  ResponseEnvelope,
  EventMessage,
  RequestContext,
  ErrorInfo,
  ResponseMetadata,
  Node,
  RelationRef,
  IncomingRelation,
  NodeRelationships,
  PropertyValue,
  NodeCreatePayload,
  NodeUpdatePayload,
  NodeDeletePayload,
  NodeGetPayload,
  NodeQueryPayload,
  RelationAddPayload,
  RelationRemovePayload,
  RelationsGetPayload,
  SqlQueryPayload,
  SqlResult,
  Workspace,
  WorkspaceCreatePayload,
  WorkspaceGetPayload,
  WorkspaceDeletePayload,
  WorkspaceUpdatePayload,
  SubscribePayload,
  SubscriptionFilters,
  UnsubscribePayload,
  SubscriptionResponse,
  AuthenticatePayload,
  AuthenticateResponse,
  RefreshTokenPayload,
  TransactionBeginPayload,
  TransactionBeginResponse,
  TransactionCommitPayload,
  TransactionCommitResponse,
  TransactionRollbackPayload,
} from './protocol';

export {
  RequestType,
  ResponseStatus,
  encodeMessage,
  decodeMessage,
  isEventMessage,
  isResponseEnvelope,
} from './protocol';

// Utilities
export { RequestTracker } from './utils/request-tracker';
export type { RequestTrackerOptions } from './utils/request-tracker';
export { ReconnectManager } from './utils/reconnect';
export type { ReconnectOptions } from './utils/reconnect';

// Logger
export { LogLevel, logger, configureLogger, setLogLevel, getLogLevel, getLoggerConfig } from './logger';
export type { LoggerConfig } from './logger';

// React Router 7 integration
export {
  createSSRClient,
  createLoader,
  createRequestScopedClient,
  rowsToObjects,
  getErrorMessage,
  isHttpClient,
  isWebSocketClient,
} from './integrations/react-router';
export type {
  SSRClientConfig,
  HybridClient,
} from './integrations/react-router';

// Upload module
export type {
  UploadStatus,
  UploadOptions,
  UploadProgress,
  UploadResult,
  Upload,
  CreateUploadRequest,
  CreateUploadResponse,
  ChunkUploadResponse,
  UploadStatusResponse,
  CompleteUploadRequest,
  CompleteUploadResponse,
  RetryOptions,
  FileSource,
  RetryContext,
  RetryCallback,
  UploaderConfig,
  // Batch upload types
  BatchFileProgress,
  BatchProgress,
  BatchUploadResult,
  BatchUploadOptions,
  BatchUpload,
} from './upload';
export {
  UploadError,
  UploadErrorCode,
  DEFAULT_CHUNK_SIZE,
  DEFAULT_MAX_RETRIES,
  DEFAULT_RETRY_BASE_DELAY,
  DEFAULT_RETRY_MAX_DELAY,
  DEFAULT_NODE_TYPE,
  DEFAULT_BRANCH,
  DEFAULT_BATCH_CONCURRENCY,
  BrowserFileSource,
  NodeFileSource,
  createFileSource,
  getContentType,
  calculateChunkCount,
  getChunkRange,
  DEFAULT_RETRY_OPTIONS,
  calculateDelay,
  sleep,
  isRetryableError,
  withRetry,
  RetryBuilder,
  classifyError,
  Uploader,
  UploadManager,
  BatchUploader,
} from './upload';

// Errors
export {
  RaisinError,
  RaisinConnectionError,
  RaisinAuthError,
  RaisinFlowError,
  RaisinTimeoutError,
  RaisinAbortError,
} from './errors';
export type {
  RaisinErrorCode,
  ConnectionErrorCode,
  AuthErrorCode,
  FlowErrorCode,
  TimeoutErrorCode,
  AbortErrorCode,
} from './errors';

// Flow execution
export { FlowClient } from './flow-client';
export type { FlowClientOptions, FlowRunResult, FlowCollectResult } from './flow-client';

// Function invocation
export { FunctionsApi, HttpFunctionsApi } from './functions-api';
export type {
  FunctionInvokeOptions,
  FunctionInvokeResponse,
  FunctionInvokeSyncResponse
} from './functions-api';

// Flow execution (WebSocket)
export { FlowsApi } from './flows';
export type {
  FlowRunResponse,
  FlowInstanceStatus,
  FlowInstanceStatusResponse,
  FlowExecutionEvent,
  StepStartedEvent,
  StepCompletedEvent,
  StepFailedEvent,
  FlowWaitingEvent,
  FlowResumedEvent,
  FlowCompletedEvent,
  FlowFailedEvent,
  TextChunkEvent,
  ToolCallStartedEvent,
  ToolCallCompletedEvent,
  ThoughtChunkEvent,
  ConversationCreatedEvent,
  MessageSavedEvent,
  LogEvent,
} from './types/flow';
export { isTerminalEvent } from './types/flow';

// Chat types
export type {
  ChatMessage,
  ChatEvent,
  Conversation,
  ConversationType,
  ConversationListItem,
  ConversationStatus,
  MessageBody,
  ChatTextChunkEvent,
  ChatAssistantMessageEvent,
  ChatWaitingEvent,
  ChatCompletedEvent,
  ChatFailedEvent,
  ChatDoneEvent,
  ChatToolCallStartedEvent,
  ChatToolCallCompletedEvent,
  ChatThoughtChunkEvent,
  ToolCallRecord,
  MessageChild,
  PlanTask,
  ChatConversationCreatedEvent,
  ChatMessageSavedEvent,
  ChatMessageDeliveredEvent,
  ChatLogEvent,
} from './types/chat';

// Conversation management (unified API)
export { ConversationManager } from './conversations';
export type {
  ListConversationsOptions,
  CreateConversationOptions as CreateConvoOptions,
  SendMessageOptions,
  ConversationSubscription,
  PlanActionReceipt,
  PlanActionOptions,
  ConversationManagerOptions,
} from './conversations';

// Conversation stores (framework-agnostic)
export { ConversationStore } from './stores/conversation-store';
export type {
  ConversationStoreSnapshot,
  ConversationStoreOptions,
  ToolCallInfo,
} from './stores/conversation-store';
export { ConversationListStore } from './stores/conversation-list-store';
export type {
  ConversationListSnapshot,
  ConversationListStoreOptions,
} from './stores/conversation-list-store';

// Plan projection utility
export { projectPlansFromMessages } from './utils/plan-projection';
export type { PlanProjection, PlanProjectionTask } from './utils/plan-projection';

// React conversation adapters
export { useConversation, useConversationList } from './integrations/react-conversation';
export type {
  ReactLike,
  UseConversationReturn,
  UseConversationListReturn,
} from './integrations/react-conversation';

// Svelte 5 conversation adapters (legacy path — prefer @raisindb/client/svelte)
export { createConversationAdapter, createConversationListAdapter } from './integrations/svelte-conversation';

// Svelte 5 integration (full)
export {
  createAuthAdapter,
  createConnectionAdapter,
  createSqlAdapter,
  createSubscriptionAdapter,
  createFlowAdapter,
  RAISIN_CONTEXT_KEY,
} from './integrations/svelte';
export type {
  AuthSnapshot,
  AuthAdapter,
  ConnectionSnapshot,
  ConnectionAdapter,
  SqlAdapterOptions,
  SqlSnapshot,
  SqlAdapter,
  SubscriptionAdapter,
  FlowSnapshot,
  FlowAdapterOptions,
  FlowAdapter,
  FlowStatus as SvelteFlowStatus,
  RaisinContext,
} from './integrations/svelte';

// React flow adapter
export { useFlow } from './integrations/react-flow';
export type {
  UseFlowOptions,
  UseFlowReturn,
  FlowStatus,
} from './integrations/react-flow';

// React integration (Provider + hooks factory)
export { createRaisinReact } from './integrations/react';
export type {
  RaisinReact,
  ReactLikeWithContext,
  RaisinProviderProps,
  RaisinContextValue,
  UseAuthReturn,
  UseConnectionReturn,
  UseSqlOptions,
  UseSqlReturn,
  UseSubscriptionOptions,
} from './integrations/react';

// Streaming (SSE)
export { SSEClient } from './streaming/sse-client';
export type {
  SSEEvent,
  SSEClientOptions,
  SSEConnectionState,
  SSEStateCallback,
} from './streaming/sse-client';
