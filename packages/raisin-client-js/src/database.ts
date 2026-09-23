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
import { SchedulerApi } from './scheduler';
import { Tags } from './tags';
import { FlowsApi } from './flows';
import { AgentRunsWsApi } from './agent-runs-ws';
import { FunctionsApi } from './functions-api';
import { SearchApi } from './search-api';
import type {
  Answer,
  AskOptions,
  SearchOptions,
  SearchPassage,
} from './search-api';
import { ConversationManager } from './conversations';
import { FlowClient } from './flow-client';
import { InboxApi } from './inbox';
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
  private _searchApi?: SearchApi;
  private branch?: string;
  private revision?: string;
  private getUploadManager?: () => UploadManager;
  private getSignAssetUrl?: (options: SignAssetOptions) => Promise<SignedAssetUrl>;
  private httpOptions?: DatabaseHttpOptions;
  private _flowsApi?: FlowsApi;
  private _runsApi?: AgentRunsWsApi;
  private _functionsApi?: FunctionsApi;
  private _conversationManager?: ConversationManager;
  private _flowClient?: FlowClient;
  private _inboxApi?: InboxApi;

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
   * Get SchedulerApi for one-shot scheduled invocations (time-based
   * function/flow runs).
   */
  scheduler(): SchedulerApi {
    return new SchedulerApi(this._context, this.sendRequest);
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
   * Durable agent runs over this WebSocket connection: find a subject's run,
   * follow it live (gap-free, resumable), control it, list its children.
   */
  runs(): AgentRunsWsApi {
    if (!this._runsApi) {
      this._runsApi = new AgentRunsWsApi(this._context, this.sendRequest, this.eventHandler);
    }
    return this._runsApi;
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
   * The retrieval implementation behind {@link search} and {@link ask}.
   *
   * Private: callers get the two verbs, not the object, so how retrieval is
   * served stays ours to change.
   */
  private searchApi(): SearchApi {
    if (!this._searchApi) {
      this._searchApi = new SearchApi(
        (sql, params) => this.executeSql(sql, params),
        async (name, input) => {
          const run = await this.functions().invoke(name, input, {
            waitForResult: true,
          });
          // `invoke` reports the job; the function's own return value is the
          // `result` it carries once it has completed.
          return (run as { result?: unknown })?.result ?? run;
        },
      );
    }
    return this._searchApi;
  }

  /**
   * Find the passages matching a query, by meaning and by keyword at once.
   *
   * No model is involved — this is the call for a search box or for assembling
   * your own context. Returns one entry per PASSAGE, so several may come from
   * one document.
   *
   * @example
   * ```typescript
   * const passages = await db.search('how much notice to terminate', {
   *   workspaces: 'stories',
   * });
   * passages[0].text;        // the passage itself
   * passages[0].chunkIndex;  // where in the document it came from
   * ```
   */
  async search(query: string, options: SearchOptions): Promise<SearchPassage[]> {
    return this.searchApi().search(query, options);
  }

  /**
   * Answer a question from the stored content, with citations.
   *
   * Retrieves, judges whether what came back answers the question, rewrites the
   * query once if it does not, and answers from those passages only. Check
   * `grounded` before showing the answer: when it is false nothing relevant was
   * found and the model was never asked.
   *
   * `workspaces` is required, the same as on {@link search}: an answer is
   * quoted back to whoever asked, so the corpus it may be drawn from is stated
   * rather than assumed. One name, a comma-separated list, a glob, or
   * `'ALL READABLE'`.
   *
   * A method rather than `functions().invoke('ask', ...)` on purpose — see the
   * note at the top of `search-api.ts`. `invoke` is the escape hatch for
   * functions YOU wrote.
   *
   * @example
   * ```typescript
   * const { answer, citations, grounded } = await db.ask(
   *   'How much notice do we have to give?',
   *   { workspaces: 'stories, handbook' },   // one name, a list, a glob, or 'ALL READABLE'
   * );
   * ```
   */
  async ask(question: string, options: AskOptions): Promise<Answer> {
    return this.searchApi().ask(question, options);
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
   * Get a pre-configured InboxApi for human-in-the-loop tasks.
   *
   * Requires the Database to have been created via `RaisinClient.database()`
   * (which automatically provides HTTP context).
   *
   * @example
   * ```typescript
   * const db = client.database('my-repo');
   * const { tasks } = await db.inbox.listTasks({ status: 'pending' });
   * await db.inbox.completeTask(tasks[0].id, { action: 'approve' });
   * ```
   */
  get inbox(): InboxApi {
    if (!this._inboxApi) {
      if (!this.httpOptions?.httpBaseUrl || !this.httpOptions?.authManager) {
        throw new Error(
          'db.inbox requires HTTP context. Use client.database() to get a Database with inbox support.',
        );
      }
      this._inboxApi = new InboxApi(
        this.httpOptions.httpBaseUrl,
        this.repository,
        this.httpOptions.authManager,
      );
    }
    return this._inboxApi;
  }

  /**
   * Get EventHandler for subscribing to real-time events
   *
   * Path filter semantics for subscriptions (matched server-side):
   * - A plain path matches **only that exact node** — there is no implicit
   *   prefix matching.
   * - `*` matches exactly one path segment (`/inbox/*` matches `/inbox/a`,
   *   not `/inbox/a/b`).
   * - `**` matches recursively (`/inbox/**` matches all descendants of
   *   `/inbox`, but not `/inbox` itself).
   */
  events(): EventHandler {
    return this.eventHandler;
  }
}
