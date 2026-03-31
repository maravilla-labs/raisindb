/**
 * Database/repository interface
 */

import { WorkspaceClient, WorkspaceManager } from './workspace';
import { SqlQuery, createSqlHandler } from './sql';
import { EventHandler } from './events';
import { RequestContext, SqlQueryPayload, SqlResult } from './protocol';
import { NodeTypes } from './node-types';
import { Archetypes } from './archetypes';
import { ElementTypes } from './element-types';
import { Branches } from './branches';
import { Tags } from './tags';
import { FlowsApi } from './flows';
import { FunctionsApi } from './functions-api';
import { ConversationManager } from './conversations';
import { FlowClient } from './flow-client';
import type { AuthManager } from './auth';
import type { UploadManager } from './upload/uploader';
import type { SignAssetOptions, SignedAssetUrl } from './http-client';

/**
 * Options for HTTP-based features (chat, flow) on a Database instance.
 * Provided automatically when the Database is created via `RaisinClient.database()`.
 */
export interface DatabaseHttpOptions {
  /** HTTP base URL of the RaisinDB server */
  httpBaseUrl?: string;
  /** Auth manager for token access */
  authManager?: AuthManager;
}

/**
 * Database interface for repository operations
 */
export class Database {
  private repository: string;
  private _context: RequestContext;
  private sendRequest: (
    payload: unknown,
    requestType?: string,
    contextOverride?: RequestContext,
    requestOptions?: { timeoutMs?: number }
  ) => Promise<unknown>;
  private eventHandler: EventHandler;
  private workspaceManager?: WorkspaceManager;
  private sqlQuery?: SqlQuery;
  private sqlHandler?: (strings: TemplateStringsArray, ...values: unknown[]) => Promise<SqlResult>;
  private branch?: string;
  private revision?: string;
  private getUploadManager?: () => UploadManager;
  private getSignAssetUrl?: (options: SignAssetOptions) => Promise<SignedAssetUrl>;
  private httpOptions?: DatabaseHttpOptions;
  private _flowsApi?: FlowsApi;
  private _functionsApi?: FunctionsApi;
  private _conversationManager?: ConversationManager;
  private _flowClient?: FlowClient;

  constructor(
    repository: string,
    context: RequestContext,
    sendRequest: (
      payload: unknown,
      requestType: string,
      contextOverride?: RequestContext,
      requestOptions?: { timeoutMs?: number }
    ) => Promise<unknown>,
    eventHandler: EventHandler,
    branch?: string,
    revision?: string,
    getUploadManager?: () => UploadManager,
    getSignAssetUrl?: (options: SignAssetOptions) => Promise<SignedAssetUrl>,
    httpOptions?: DatabaseHttpOptions,
  ) {
    this.repository = repository;
    this.branch = branch;
    this.revision = revision;
    this._context = {
      ...context,
      repository,
      branch: branch || context.branch,
      revision: revision || context.revision
    };
    // Wrap sendRequest to use our context by default, but allow override
    this.sendRequest = (
      payload: unknown,
      requestType?: string,
      contextOverride?: RequestContext,
      requestOptions?: { timeoutMs?: number }
    ) => sendRequest(
      payload,
      requestType || 'node_create',
      contextOverride || this._context,
      requestOptions
    );
    this.eventHandler = eventHandler;
    this.getUploadManager = getUploadManager;
    this.getSignAssetUrl = getSignAssetUrl;
    this.httpOptions = httpOptions;
  }

  /**
   * Get the repository name
   */
  getRepository(): string {
    return this.repository;
  }

  /**
   * Create a new Database instance scoped to a specific branch.
   *
   * @param branch - Branch name
   * @returns New Database instance with branch context
   */
  onBranch(branch: string): Database {
    return new Database(
      this.repository,
      this._context,
      this.sendRequest,
      this.eventHandler,
      branch,
      this.revision,
      this.getUploadManager,
      this.getSignAssetUrl,
      this.httpOptions,
    );
  }

  /**
   * Create a new Database instance scoped to a specific revision/commit.
   *
   * @param revision - Revision/commit ID
   * @returns New Database instance with revision context
   */
  atRevision(revision: string): Database {
    return new Database(
      this.repository,
      this._context,
      this.sendRequest,
      this.eventHandler,
      this.branch,
      revision,
      this.getUploadManager,
      this.getSignAssetUrl,
      this.httpOptions,
    );
  }

  /**
   * Get a workspace client
   *
   * @param name - Workspace name
   * @returns Workspace client for operations
   */
  workspace(name: string): WorkspaceClient {
    if (!this.workspaceManager) {
      this.workspaceManager = new WorkspaceManager(
        this._context,
        this.sendRequest,
        this.eventHandler,
        this.getUploadManager,
        this.getSignAssetUrl
      );
    }
    return this.workspaceManager.workspace(name);
  }

  /**
   * Get workspace management operations
   */
  workspaces(): WorkspaceManager {
    if (!this.workspaceManager) {
      this.workspaceManager = new WorkspaceManager(
        this._context,
        this.sendRequest,
        this.eventHandler,
        this.getUploadManager,
        this.getSignAssetUrl
      );
    }
    return this.workspaceManager;
  }

  /**
   * Create a new workspace (convenience method)
   *
   * @param name - Workspace name
   * @param description - Workspace description
   * @returns Created workspace
   */
  async createWorkspace(name: string, description?: string): Promise<unknown> {
    return this.workspaces().create({ name, description });
  }

  /**
   * List all workspaces (convenience method)
   *
   * @returns Array of workspaces
   */
  async listWorkspaces(): Promise<unknown[]> {
    return this.workspaces().list();
  }

  /**
   * Execute SQL query using template literals
   *
   * @param strings - Template literal strings
   * @param values - Template literal values
   * @returns SQL query result
   *
   * @example
   * ```typescript
   * const results = await db.sql`SELECT * FROM nodes WHERE node_type = ${'Page'}`;
   * ```
   */
  async sql(strings: TemplateStringsArray, ...values: unknown[]): Promise<SqlResult> {
    if (!this.sqlHandler) {
      this.sqlHandler = createSqlHandler((payload: SqlQueryPayload) =>
        this.sendRequest(payload, 'sql_query')
      );
    }
    return this.sqlHandler(strings, ...values);
  }

  /**
   * Get SQL query builder for more advanced queries
   */
  getSqlQuery(): SqlQuery {
    if (!this.sqlQuery) {
      this.sqlQuery = new SqlQuery((payload: SqlQueryPayload) =>
        this.sendRequest(payload, 'sql_query')
      );
    }
    return this.sqlQuery;
  }

  /**
   * Execute a raw SQL query
   *
   * @param query - SQL query string
   * @param params - Query parameters
   * @returns SQL query result
   */
  async executeSql(query: string, params?: unknown[]): Promise<SqlResult> {
    return this.getSqlQuery().execute(query, params);
  }

  /**
   * Get NodeTypes management operations
   */
  nodeTypes(): NodeTypes {
    return new NodeTypes(this._context, this.sendRequest);
  }

  /**
   * Get Archetypes management operations
   */
  archetypes(): Archetypes {
    return new Archetypes(this._context, this.sendRequest);
  }

  /**
   * Get ElementTypes management operations
   */
  elementTypes(): ElementTypes {
    return new ElementTypes(this._context, this.sendRequest);
  }

  /**
   * Get Branches management operations
   */
  branches(): Branches {
    return new Branches(this._context, this.sendRequest);
  }

  /**
   * Get Tags management operations
   */
  tags(): Tags {
    return new Tags(this._context, this.sendRequest);
  }

  /**
   * Get FlowsApi for running flows via WebSocket
   */
  flows(): FlowsApi {
    if (!this._flowsApi) {
      this._flowsApi = new FlowsApi(
        this.repository,
        this._context,
        this.sendRequest,
        this.eventHandler,
      );
    }
    return this._flowsApi;
  }

  /**
   * Get FunctionsApi for invoking server-side functions via WebSocket
   */
  functions(): FunctionsApi {
    if (!this._functionsApi) {
      this._functionsApi = new FunctionsApi(
        this.repository,
        this._context,
        this.sendRequest,
      );
    }
    return this._functionsApi;
  }

  /**
   * Get a pre-configured ConversationManager for managing conversations.
   *
   * Unified API for conversation lifecycle, messaging, streaming, and
   * plan approval/rejection. Replaces the previous ChatClient and
   * ConversationClient split.
   *
   * @example
   * ```typescript
   * const db = client.database('my-repo');
   * const convos = await db.conversations.list({ type: 'ai_chat' });
   * const convo = await db.conversations.create({ participant: '/agents/support' });
   * ```
   */
  get conversations(): ConversationManager {
    if (!this._conversationManager) {
      if (!this.httpOptions?.httpBaseUrl || !this.httpOptions?.authManager) {
        throw new Error(
          'db.conversations requires HTTP context. Use client.database() to get a Database with conversation support.',
        );
      }
      this._conversationManager = new ConversationManager(
        this.httpOptions.httpBaseUrl,
        this.repository,
        this.httpOptions.authManager,
        {},
        (query: string, params?: unknown[]) => this.executeSql(query, params),
      );
    }
    return this._conversationManager;
  }

  /**
   * Get a pre-configured FlowClient for executing flows via HTTP/SSE.
   *
   * Requires the Database to have been created via `RaisinClient.database()`
   * (which automatically provides HTTP context). Throws if HTTP context is unavailable.
   *
   * @example
   * ```typescript
   * const db = client.database('my-repo');
   * const result = await db.flow.runAndWait('/flows/my-flow', { key: 'value' });
   * ```
   */
  get flow(): FlowClient {
    if (!this._flowClient) {
      if (!this.httpOptions?.httpBaseUrl || !this.httpOptions?.authManager) {
        throw new Error(
          'db.flow requires HTTP context. Use client.database() to get a Database with flow support.',
        );
      }
      this._flowClient = new FlowClient(
        this.httpOptions.httpBaseUrl,
        this.repository,
        this.httpOptions.authManager,
        {},
        this.flows(),
      );
    }
    return this._flowClient;
  }

  /**
   * Get EventHandler for subscribing to real-time events
   */
  events(): EventHandler {
    return this.eventHandler;
  }
}
