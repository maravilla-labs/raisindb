/**
 * RaisinDB HTTP client for server-side rendering (SSR)
 *
 * This client uses HTTP REST API instead of WebSocket for initial data fetching
 * during server-side rendering. After client hydration, applications can upgrade
 * to WebSocket mode for real-time updates.
 */

import { EventEmitter } from 'events';
import { AuthManager, Credentials, AdminCredentials, isJwtCredentials, TokenStorage } from './auth';
import {
  RequestContext,
  SqlResult,
  Node,
  Workspace,
  NodeCreatePayload,
  WorkspaceCreatePayload,
  WorkspaceUpdatePayload,
  AuthenticatePayload,
  AuthenticateResponse,
} from './protocol';
import {
  HttpFunctionsApi,
  type FunctionInvokeOptions,
  type FunctionInvokeResponse,
  type FunctionInvokeSyncResponse
} from './functions-api';
import type { Upload, UploadOptions, BatchUpload, BatchUploadOptions } from './upload/types';
import { createFileSource } from './upload/file-source';
import { UploadManager } from './upload/uploader';

/**
 * Current user info
 */
export interface CurrentUser {
  /** User ID (identity UUID) */
  userId: string;
  /** Roles assigned to the user */
  roles?: string[];
  /** Whether this is an anonymous user */
  anonymous: boolean;
  /** The full user node from the repository */
  node?: UserNode;
}

/**
 * User node from the repository
 */
export interface UserNode {
  /** Node ID */
  id: string;
  /** Node path (e.g., '/users/internal/john-at-example-com') */
  path: string;
  /** Node name */
  name: string;
  /** Node type (e.g., 'raisin:User') */
  node_type: string;
  /** Node properties */
  properties: Record<string, unknown>;
}

/**
 * Response from /auth/{repo}/me endpoint
 */
interface AuthMeResponse {
  user_id: string;
  user_path: string;
  user_node?: UserNode;
  roles: string[];
  anonymous: boolean;
}

/**
 * HTTP client options
 */
export interface HttpClientOptions {
  /** Token storage (default: in-memory) */
  tokenStorage?: TokenStorage;
  /** Request timeout in milliseconds (default: 30000) */
  requestTimeout?: number;
  /** Tenant ID */
  tenantId?: string;
  /** Default branch (default: "main") */
  defaultBranch?: string;
  /** Custom fetch implementation (for testing or custom behavior) */
  fetch?: typeof fetch;
}

/**
 * HTTP response wrapper
 */
interface HttpResponse<T = unknown> {
  data: T;
  status: number;
  headers: Headers;
}

/**
 * Options for signing an asset URL
 */
export interface SignAssetOptions {
  /** Repository name */
  repository: string;
  /** Workspace name */
  workspace: string;
  /** Path to the asset node */
  path: string;
  /** Command type: 'download' forces download, 'display' shows inline */
  command: 'download' | 'display';
  /** Branch (default: context branch or "main") */
  branch?: string;
  /** Expiry time in seconds (default: 300) */
  expiresIn?: number;
  /** Property path to access (default: 'file'). Use 'thumbnail' for thumbnail images. */
  propertyPath?: string;
}

/**
 * Signed URL response
 */
export interface SignedAssetUrl {
  /** The signed URL for accessing the asset */
  url: string;
  /** When the URL expires (ISO 8601) */
  expiresAt: string;
  /** @deprecated Use expiresAt instead */
  expires_at?: string;
}

/**
 * RaisinDB HTTP client for SSR
 */
export class RaisinHttpClient extends EventEmitter {
  private baseUrl: string;
  private authManager: AuthManager;
  private _context: RequestContext;
  private options: Required<Omit<HttpClientOptions, 'tokenStorage' | 'fetch'>>;
  private fetchImpl: typeof fetch;
  private _currentUser: CurrentUser | null = null;
  private _uploadManager: UploadManager | null = null;

  constructor(baseUrl: string, options: HttpClientOptions = {}) {
    super();

    // Remove trailing slash from baseUrl
    this.baseUrl = baseUrl.replace(/\/$/, '');

    this.options = {
      requestTimeout: options.requestTimeout ?? 30000,
      tenantId: options.tenantId ?? 'default',
      defaultBranch: options.defaultBranch ?? 'main',
    };

    // Initialize context
    this._context = {
      tenant_id: this.options.tenantId,
      branch: this.options.defaultBranch,
    };

    // Initialize auth manager
    this.authManager = new AuthManager(options.tokenStorage);

    // Use custom fetch or global fetch
    this.fetchImpl = options.fetch ?? fetch;
  }

  /**
   * Authenticate with credentials
   *
   * Supports two authentication modes:
   * - Admin auth: { username: string, password: string }
   * - JWT auth: { type: 'jwt', token: string }
   *
   * @param credentials - User credentials
   */
  async authenticate(credentials: Credentials, repository?: string): Promise<void> {
    if (isJwtCredentials(credentials)) {
      // For HTTP client, JWT auth is typically done via Authorization header
      // Just store the token for future requests
      this.authManager.storage.setAccessToken(credentials.token);

      // Fetch user info from /auth/{repo}/me endpoint if repository is provided
      if (repository) {
        try {
          const userInfo = await this.fetchCurrentUser(repository);
          if (userInfo) {
            this._currentUser = userInfo;
          }
        } catch {
          // User info fetch failed, continue without it
        }
      }

      // Emit authenticated event
      this.emit('authenticated', { type: 'jwt', token: credentials.token });
    } else {
      // Admin authentication with username/password
      const payload: AuthenticatePayload = AuthManager.createAuthPayload(credentials as AdminCredentials);

      const response = await this.request<AuthenticateResponse>({
        method: 'POST',
        path: `/api/raisindb/sys/${this._context.tenant_id}/auth`,
        body: payload,
        skipAuth: true,
      });

      this.authManager.setTokens(response.data);

      // Set admin user info
      this._currentUser = {
        userId: 'admin',
        anonymous: false,
      };

      // Emit authenticated event
      this.emit('authenticated', response.data);
    }
  }

  /**
   * Execute SQL query
   *
   * @param repository - Repository name
   * @param query - SQL query string
   * @param params - Query parameters
   * @param branch - Optional branch (uses new /api/sql/{repo}/{branch} endpoint)
   * @returns SQL query result
   */
  async executeSql(
    repository: string,
    query: string,
    params?: unknown[],
    branch?: string
  ): Promise<SqlResult> {
    // Note: The HTTP API expects 'sql' field, not 'query'
    const payload = {
      sql: query,
      params: params || [],
    };

    // Use branch-specific endpoint if branch is provided
    const path = branch
      ? `/api/sql/${repository}/${branch}`
      : `/api/sql/${repository}`;

    const response = await this.request<SqlResult>({
      method: 'POST',
      path,
      body: payload,
    });

    return response.data;
  }

  /**
   * Get a node by ID
   *
   * @param repository - Repository name
   * @param workspace - Workspace name
   * @param nodeId - Node ID
   * @returns Node data
   */
  async getNode(
    repository: string,
    workspace: string,
    nodeId: string
  ): Promise<Node> {
    const branch = this._context.branch ?? this.options.defaultBranch;

    const response = await this.request<Node>({
      method: 'GET',
      path: `/api/repository/${repository}/${branch}/head/${workspace}/$ref/${nodeId}`,
    });

    return response.data;
  }

  /**
   * Get a node by path
   *
   * @param repository - Repository name
   * @param workspace - Workspace name
   * @param nodePath - Node path (e.g., "/content/my-page")
   * @returns Node data
   */
  async getNodeByPath(
    repository: string,
    workspace: string,
    nodePath: string
  ): Promise<Node> {
    const branch = this._context.branch ?? this.options.defaultBranch;

    // Ensure path starts with /
    const path = nodePath.startsWith('/') ? nodePath : `/${nodePath}`;

    const response = await this.request<Node>({
      method: 'GET',
      path: `/api/repository/${repository}/${branch}/head/${workspace}${path}`,
    });

    return response.data;
  }

  /**
   * Create a new node
   *
   * @param repository - Repository name
   * @param workspace - Workspace name
   * @param payload - Node creation payload
   * @returns Created node
   */
  async createNode(
    repository: string,
    workspace: string,
    payload: NodeCreatePayload
  ): Promise<Node> {
    const branch = this._context.branch ?? this.options.defaultBranch;

    const response = await this.request<Node>({
      method: 'POST',
      path: `/api/repository/${repository}/${branch}/head/${workspace}/`,
      body: payload,
    });

    return response.data;
  }

  /**
   * Update a node
   *
   * @param repository - Repository name
   * @param workspace - Workspace name
   * @param nodeId - Node ID
   * @param properties - Properties to update
   * @returns Updated node
   */
  async updateNode(
    repository: string,
    workspace: string,
    nodeId: string,
    properties: Record<string, unknown>
  ): Promise<Node> {
    const branch = this._context.branch ?? this.options.defaultBranch;

    // First get the node to get its path
    const node = await this.getNode(repository, workspace, nodeId);

    const response = await this.request<Node>({
      method: 'PUT',
      path: `/api/repository/${repository}/${branch}/head/${workspace}${node.path}`,
      body: { properties },
    });

    return response.data;
  }

  /**
   * Delete a node
   *
   * @param repository - Repository name
   * @param workspace - Workspace name
   * @param nodeId - Node ID
   */
  async deleteNode(
    repository: string,
    workspace: string,
    nodeId: string
  ): Promise<void> {
    const branch = this._context.branch ?? this.options.defaultBranch;

    // First get the node to get its path
    const node = await this.getNode(repository, workspace, nodeId);

    await this.request<void>({
      method: 'DELETE',
      path: `/api/repository/${repository}/${branch}/head/${workspace}${node.path}`,
    });
  }

  /**
   * List workspaces
   *
   * @param repository - Repository name
   * @returns Array of workspaces
   */
  async listWorkspaces(repository: string): Promise<Workspace[]> {
    const response = await this.request<Workspace[]>({
      method: 'GET',
      path: `/api/workspaces/${repository}`,
    });

    return response.data;
  }

  /**
   * Get a workspace
   *
   * @param repository - Repository name
   * @param name - Workspace name
   * @returns Workspace data
   */
  async getWorkspace(repository: string, name: string): Promise<Workspace> {
    const response = await this.request<Workspace>({
      method: 'GET',
      path: `/api/workspaces/${repository}/${name}`,
    });

    return response.data;
  }

  /**
   * Create a workspace
   *
   * @param repository - Repository name
   * @param payload - Workspace creation payload
   * @returns Created workspace
   */
  async createWorkspace(
    repository: string,
    payload: WorkspaceCreatePayload
  ): Promise<Workspace> {
    const response = await this.request<Workspace>({
      method: 'POST',
      path: `/api/workspaces/${repository}`,
      body: payload,
    });

    return response.data;
  }

  /**
   * Update a workspace
   *
   * @param repository - Repository name
   * @param name - Workspace name
   * @param payload - Workspace update payload
   * @returns Updated workspace
   */
  async updateWorkspace(
    repository: string,
    name: string,
    payload: WorkspaceUpdatePayload
  ): Promise<Workspace> {
    const response = await this.request<Workspace>({
      method: 'PUT',
      path: `/api/workspaces/${repository}/${name}`,
      body: payload,
    });

    return response.data;
  }

  /**
   * Delete a workspace
   *
   * @param repository - Repository name
   * @param name - Workspace name
   */
  async deleteWorkspace(repository: string, name: string): Promise<void> {
    await this.request<void>({
      method: 'DELETE',
      path: `/api/workspaces/${repository}/${name}`,
    });
  }

  /**
   * Create a new repository
   *
   * @param repositoryId - Repository identifier
   * @param description - Repository description
   * @param config - Repository configuration
   * @returns Repository creation result
   */
  async createRepository(
    repositoryId: string,
    description?: string,
    config?: Record<string, unknown>
  ): Promise<unknown> {
    const response = await this.request<unknown>({
      method: 'POST',
      path: '/api/repositories',
      body: {
        repository_id: repositoryId,
        description,
        config,
      },
    });

    return response.data;
  }

  /**
   * Get a repository by ID
   *
   * @param repositoryId - Repository identifier
   * @returns Repository information
   */
  async getRepository(repositoryId: string): Promise<unknown> {
    const response = await this.request<unknown>({
      method: 'GET',
      path: `/api/repositories/${repositoryId}`,
    });

    return response.data;
  }

  /**
   * List all repositories
   *
   * @returns Array of repository information
   */
  async listRepositories(): Promise<unknown[]> {
    const response = await this.request<unknown[]>({
      method: 'GET',
      path: '/api/repositories',
    });

    return response.data;
  }

  /**
   * Update a repository
   *
   * @param repositoryId - Repository identifier
   * @param description - New repository description
   * @param config - New repository configuration
   * @returns Repository update result
   */
  async updateRepository(
    repositoryId: string,
    description?: string,
    config?: Record<string, unknown>
  ): Promise<unknown> {
    const response = await this.request<unknown>({
      method: 'PUT',
      path: `/api/repositories/${repositoryId}`,
      body: {
        description,
        config,
      },
    });

    return response.data;
  }

  /**
   * Delete a repository
   *
   * @param repositoryId - Repository identifier
   * @returns Repository deletion result
   */
  async deleteRepository(repositoryId: string): Promise<unknown> {
    const response = await this.request<unknown>({
      method: 'DELETE',
      path: `/api/repositories/${repositoryId}`,
    });

    return response.data;
  }

  /**
   * Set branch for subsequent requests
   *
   * @param branch - Branch name
   */
  setBranch(branch: string): void {
    this._context.branch = branch;
  }

  /**
   * Get current branch
   */
  getBranch(): string {
    return this._context.branch ?? this.options.defaultBranch;
  }

  /**
   * Get current tenant ID
   */
  getTenantId(): string {
    return this._context.tenant_id;
  }

  /**
   * Check if authenticated
   */
  isAuthenticated(): boolean {
    return this.authManager.isAuthenticated();
  }

  /**
   * Get current user info
   *
   * Returns the current authenticated user's information including
   * their ID, node in the repository, and roles.
   *
   * @returns Current user info or null if not authenticated
   */
  getCurrentUser(): CurrentUser | null {
    return this._currentUser;
  }

  /**
   * Get current user ID
   *
   * @returns User ID or null if not authenticated
   */
  getCurrentUserId(): string | null {
    return this._currentUser?.userId ?? null;
  }

  /**
   * Get current user's node path in the repository
   *
   * @returns User's node path (e.g., '/users/internal/john-at-example-com') or null
   */
  getCurrentUserPath(): string | null {
    return this._currentUser?.node?.path ?? null;
  }

  /**
   * Fetch current user info from /auth/{repo}/me endpoint
   *
   * @param repository - Repository name
   * @returns Current user info or null if fetch fails
   */
  async fetchCurrentUser(repository: string): Promise<CurrentUser | null> {
    try {
      const response = await this.request<AuthMeResponse>({
        method: 'GET',
        path: `/auth/${repository}/me`,
      });

      return {
        userId: response.data.user_id,
        roles: response.data.roles,
        anonymous: response.data.anonymous,
        node: response.data.user_node,
      };
    } catch {
      return null;
    }
  }

  /**
   * Get auth manager (for token access)
   */
  getAuthManager(): AuthManager {
    return this.authManager;
  }

  // ============================================================================
  // Upload Methods
  // ============================================================================

  /**
   * Get or create the upload manager
   */
  private getUploadManager(): UploadManager {
    if (!this._uploadManager) {
      this._uploadManager = new UploadManager({
        baseUrl: this.baseUrl,
        authManager: this.authManager,
        fetchImpl: this.fetchImpl,
        requestTimeout: this.options.requestTimeout,
      });
    }
    return this._uploadManager;
  }

  /**
   * Create a resumable upload from a File or Blob (Browser)
   *
   * Creates an upload session for large files with progress tracking,
   * pause/resume, and automatic retry support.
   *
   * @param file - File or Blob to upload
   * @param options - Upload options
   * @returns Upload controller
   *
   * @example
   * ```typescript
   * const file = document.querySelector('input').files[0];
   * const upload = await client.upload(file, {
   *   repository: 'media',
   *   workspace: 'assets',
   *   path: '/videos/my-video.mp4',
   *   onProgress: (p) => console.log(`${Math.round(p.progress * 100)}%`)
   * });
   * const result = await upload.start();
   * ```
   */
  async upload(file: File | Blob, options: UploadOptions): Promise<Upload> {
    const fileSource = await createFileSource(file);
    const manager = this.getUploadManager();
    return manager.createUpload(fileSource, {
      ...options,
      branch: options.branch ?? this._context.branch ?? this.options.defaultBranch,
    });
  }

  /**
   * Create a resumable upload from a file path (Node.js)
   *
   * Creates an upload session for large files with progress tracking,
   * pause/resume, and automatic retry support.
   *
   * @param filePath - Path to the file to upload
   * @param options - Upload options
   * @returns Upload controller
   *
   * @example
   * ```typescript
   * const upload = await client.uploadFile('/path/to/file.zip', {
   *   repository: 'data',
   *   workspace: 'backups',
   *   path: '/backup.zip'
   * });
   * await upload.start();
   * ```
   */
  async uploadFile(filePath: string, options: UploadOptions): Promise<Upload> {
    const fileSource = await createFileSource(filePath);
    const manager = this.getUploadManager();
    return manager.createUpload(fileSource, {
      ...options,
      branch: options.branch ?? this._context.branch ?? this.options.defaultBranch,
    });
  }

  /**
   * Resume an existing upload by ID
   *
   * Note: This requires the upload session to still be valid on the server.
   * Sessions typically expire after 24 hours.
   *
   * @param uploadId - ID of the upload to resume
   * @returns Upload controller
   */
  getUpload(uploadId: string): Upload | undefined {
    const manager = this.getUploadManager();
    return manager.getUpload(uploadId);
  }

  /**
   * Get all active uploads
   *
   * @returns Array of active upload controllers
   */
  getActiveUploads(): Upload[] {
    const manager = this.getUploadManager();
    return manager.getActiveUploads();
  }

  /**
   * Cancel all active uploads
   */
  async cancelAllUploads(): Promise<void> {
    const manager = this.getUploadManager();
    await manager.cancelAll();
  }

  // ============================================================================
  // Asset Signed URL Methods
  // ============================================================================

  /**
   * Get a signed URL for accessing an asset's binary content.
   *
   * Server validates user access before generating URL. The returned URL
   * can be used without authentication (signature provides access).
   *
   * @param options - Sign URL options
   * @returns Signed URL and expiry time
   *
   * @example
   * ```typescript
   * const { url, expiresAt } = await client.signAssetUrl({
   *   repository: 'media',
   *   workspace: 'assets',
   *   path: '/files/image.jpeg',
   *   command: 'display',
   * });
   *
   * // Use in <img> tag
   * document.querySelector('img').src = url;
   *
   * // Or force download
   * const { url: downloadUrl } = await client.signAssetUrl({
   *   repository: 'media',
   *   workspace: 'assets',
   *   path: '/files/document.pdf',
   *   command: 'download',
   * });
   * ```
   */
  async signAssetUrl(options: SignAssetOptions): Promise<SignedAssetUrl> {
    const branch = options.branch ?? this._context.branch ?? 'main';
    // Add @propertyPath suffix if specified (for thumbnails, etc.)
    // Only add suffix when propertyPath is provided - maintains backward compatibility
    const pathWithProperty = options.propertyPath
      ? `${options.path}@${options.propertyPath}`
      : options.path;
    const endpoint = `/api/repository/${options.repository}/${branch}/head/${options.workspace}${pathWithProperty}/raisin:sign`;

    const response = await this.request<SignedAssetUrl>({
      method: 'POST',
      path: endpoint,
      body: {
        command: options.command,
        expires_in: options.expiresIn ?? 300,
      },
    });

    return {
      url: `${this.baseUrl}${response.data.url}`,
      expiresAt: response.data.expiresAt ?? response.data.expires_at,
    };
  }

  /**
   * Upload multiple files as a batch
   *
   * Creates a batch upload with concurrency control and aggregate progress tracking.
   *
   * @param files - FileList, File[], or Blob[] to upload
   * @param options - Batch upload options
   * @returns BatchUpload controller
   *
   * @example
   * ```typescript
   * const files = document.querySelector('input[type="file"]').files;
   * const batch = await client.uploadFiles(files, {
   *   repository: 'media',
   *   workspace: 'assets',
   *   basePath: '/uploads',
   *   concurrency: 3,
   *   onProgress: (p) => {
   *     console.log(`${p.filesCompleted}/${p.filesTotal} files`);
   *     console.log(`${Math.round(p.progress * 100)}% complete`);
   *   },
   *   onFileComplete: (file, result) => console.log(`Uploaded: ${file}`),
   *   onFileError: (file, err) => console.error(`Failed: ${file}`, err),
   * });
   *
   * const result = await batch.start();
   * console.log(`Success: ${result.successful.length}, Failed: ${result.failed.length}`);
   * ```
   */
  async uploadFiles(
    files: FileList | File[] | Blob[],
    options: BatchUploadOptions
  ): Promise<BatchUpload> {
    const sources = await Promise.all(
      Array.from(files).map((f) => createFileSource(f))
    );
    const manager = this.getUploadManager();
    return manager.createBatchUpload(sources, options);
  }

  /**
   * Make an HTTP request
   *
   * @param options - Request options
   * @returns HTTP response
   */
  private async request<T = unknown>(options: {
    method: string;
    path: string;
    body?: unknown;
    headers?: Record<string, string>;
    skipAuth?: boolean;
    timeoutMs?: number;
  }): Promise<HttpResponse<T>> {
    const url = `${this.baseUrl}${options.path}`;

    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...options.headers,
    };

    // Add authentication header if available
    if (!options.skipAuth && this.authManager.isAuthenticated()) {
      const token = this.authManager.getAccessToken();
      if (token) {
        headers['Authorization'] = `Bearer ${token}`;
      }
    }

    // Create AbortController for timeout
    const controller = new AbortController();
    const requestTimeout = options.timeoutMs ?? this.options.requestTimeout;
    const timeoutId = setTimeout(() => controller.abort(), requestTimeout);

    try {
      const response = await this.fetchImpl(url, {
        method: options.method,
        headers,
        body: options.body ? JSON.stringify(options.body) : undefined,
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      // Handle error responses
      if (!response.ok) {
        const errorText = await response.text();
        let errorMessage: string;
        let errorCode: string = response.statusText;

        try {
          const errorJson = JSON.parse(errorText);
          errorMessage = errorJson.message || errorJson.error || errorText;
          errorCode = errorJson.code || errorCode;
        } catch {
          errorMessage = errorText || `HTTP ${response.status}: ${response.statusText}`;
        }

        const error = new Error(errorMessage);
        (error as any).code = errorCode;
        (error as any).status = response.status;
        throw error;
      }

      // Parse response
      const contentType = response.headers.get('content-type');
      let data: T;

      if (contentType?.includes('application/json')) {
        data = await response.json();
      } else {
        data = (await response.text()) as unknown as T;
      }

      return {
        data,
        status: response.status,
        headers: response.headers,
      };
    } catch (error) {
      clearTimeout(timeoutId);

      if (error instanceof Error) {
        if (error.name === 'AbortError') {
          throw new Error(`Request timeout after ${requestTimeout}ms`);
        }
      }

      throw error;
    }
  }

  /**
   * Invoke a server-side function by name.
   *
   * Calls `POST /api/functions/{repo}/{name}/invoke`.
   *
   * @param repository - Repository name
   * @param functionName - Name of the function to invoke
   * @param input - Input data passed to the function
   * @returns Execution ID and job ID for tracking
   */
  async invokeFunction(
    repository: string,
    functionName: string,
    input?: Record<string, unknown>,
    options?: FunctionInvokeOptions,
  ): Promise<FunctionInvokeResponse> {
    const requestTimeoutMs = options?.requestTimeoutMs ?? this.options.requestTimeout;
    let waitTimeoutMs = options?.waitTimeoutMs;
    if (options?.waitForResult) {
      waitTimeoutMs = Math.min(waitTimeoutMs ?? requestTimeoutMs, requestTimeoutMs);
    }

    const response = await this.request<FunctionInvokeResponse>({
      method: 'POST',
      path: `/api/functions/${repository}/${functionName}/invoke`,
      body: {
        input: input ?? {},
        wait_for_completion: options?.waitForResult ?? false,
        wait_timeout_ms: waitTimeoutMs
      },
      timeoutMs: requestTimeoutMs,
    });
    return response.data;
  }

  /**
   * Invoke a server-side function synchronously.
   *
   * Calls `POST /api/functions/{repo}/{name}/invoke` with `sync: true`.
   * The function executes inline and the result is returned directly.
   *
   * @param repository - Repository name
   * @param functionName - Name of the function to invoke
   * @param input - Input data passed to the function
   * @returns Execution result including output, logs, and duration
   */
  async invokeFunctionSync(
    repository: string,
    functionName: string,
    input?: Record<string, unknown>,
  ): Promise<FunctionInvokeSyncResponse> {
    const response = await this.request<FunctionInvokeSyncResponse>({
      method: 'POST',
      path: `/api/functions/${repository}/${functionName}/invoke`,
      body: { input: input ?? {}, sync: true },
    });
    return response.data;
  }

  /**
   * Get a simplified database interface for SSR
   * This returns an HTTP-based database client
   */
  database(name: string): HttpDatabase {
    this._context.repository = name;
    return new HttpDatabase(name, this);
  }
}

/**
 * HTTP-based database interface for SSR
 */
export class HttpDatabase {
  private branch?: string;

  constructor(
    private repository: string,
    private client: RaisinHttpClient,
    branch?: string
  ) {
    this.branch = branch;
  }

  /**
   * Get the repository name
   */
  getRepository(): string {
    return this.repository;
  }

  /**
   * Get the current branch (if set)
   */
  getBranch(): string | undefined {
    return this.branch;
  }

  /**
   * Create a new HttpDatabase instance scoped to a specific branch.
   *
   * @param branch - Branch name
   * @returns New HttpDatabase instance with branch context
   *
   * @example
   * ```typescript
   * const db = client.database('my-repo');
   * const staging = db.onBranch('staging');
   * const result = await staging.executeSql('SELECT * FROM content');
   * ```
   */
  onBranch(branch: string): HttpDatabase {
    return new HttpDatabase(this.repository, this.client, branch);
  }

  /**
   * Execute SQL query
   *
   * @param query - SQL query string
   * @param params - Query parameters
   * @returns SQL query result
   */
  async executeSql(query: string, params?: unknown[]): Promise<SqlResult> {
    return this.client.executeSql(this.repository, query, params, this.branch);
  }

  /**
   * Get workspace client
   */
  workspace(name: string): HttpWorkspaceClient {
    return new HttpWorkspaceClient(this.repository, name, this.client);
  }

  /**
   * List workspaces
   */
  async listWorkspaces(): Promise<Workspace[]> {
    return this.client.listWorkspaces(this.repository);
  }

  /**
   * Get a workspace
   */
  async getWorkspace(name: string): Promise<Workspace> {
    return this.client.getWorkspace(this.repository, name);
  }

  /**
   * Create a workspace
   */
  async createWorkspace(name: string, description?: string): Promise<Workspace> {
    return this.client.createWorkspace(this.repository, { name, description });
  }

  /**
   * Get FunctionsApi for invoking server-side functions via HTTP
   */
  functions(): HttpFunctionsApi {
    return new HttpFunctionsApi(
      this.repository,
      (repo, name, input, options) => this.client.invokeFunction(repo, name, input, options),
      (repo, name, input) => this.client.invokeFunctionSync(repo, name, input),
    );
  }
}

/**
 * HTTP-based workspace client for SSR
 */
export class HttpWorkspaceClient {
  constructor(
    private repository: string,
    private workspace: string,
    private client: RaisinHttpClient
  ) {}

  /**
   * Get workspace name
   */
  getWorkspace(): string {
    return this.workspace;
  }

  /**
   * Get a node by ID
   */
  async getNode(nodeId: string): Promise<Node> {
    return this.client.getNode(this.repository, this.workspace, nodeId);
  }

  /**
   * Get a node by path
   */
  async getNodeByPath(nodePath: string): Promise<Node> {
    return this.client.getNodeByPath(this.repository, this.workspace, nodePath);
  }

  /**
   * Create a node
   */
  async createNode(payload: NodeCreatePayload): Promise<Node> {
    return this.client.createNode(this.repository, this.workspace, payload);
  }

  /**
   * Update a node
   */
  async updateNode(nodeId: string, properties: Record<string, unknown>): Promise<Node> {
    return this.client.updateNode(this.repository, this.workspace, nodeId, properties);
  }

  /**
   * Delete a node
   */
  async deleteNode(nodeId: string): Promise<void> {
    return this.client.deleteNode(this.repository, this.workspace, nodeId);
  }

  /**
   * Create a resumable upload for this workspace (Browser)
   *
   * Convenience method that pre-fills repository and workspace.
   *
   * @param file - File or Blob to upload
   * @param path - Path for the new node
   * @param options - Additional upload options
   * @returns Upload controller
   *
   * @example
   * ```typescript
   * const ws = client.database('media').workspace('assets');
   * const upload = await ws.upload(file, '/videos/my-video.mp4', {
   *   onProgress: (p) => console.log(`${Math.round(p.progress * 100)}%`)
   * });
   * await upload.start();
   * ```
   */
  async upload(
    file: File | Blob,
    path: string,
    options?: Partial<Omit<UploadOptions, 'repository' | 'workspace' | 'path'>>
  ): Promise<Upload> {
    return this.client.upload(file, {
      ...options,
      repository: this.repository,
      workspace: this.workspace,
      path,
    });
  }

  /**
   * Create a resumable upload for this workspace (Node.js)
   *
   * Convenience method that pre-fills repository and workspace.
   *
   * @param filePath - Path to the file to upload
   * @param nodePath - Path for the new node
   * @param options - Additional upload options
   * @returns Upload controller
   *
   * @example
   * ```typescript
   * const ws = client.database('data').workspace('backups');
   * const upload = await ws.uploadFile('/path/to/backup.zip', '/backup.zip');
   * await upload.start();
   * ```
   */
  async uploadFile(
    filePath: string,
    nodePath: string,
    options?: Partial<Omit<UploadOptions, 'repository' | 'workspace' | 'path'>>
  ): Promise<Upload> {
    return this.client.uploadFile(filePath, {
      ...options,
      repository: this.repository,
      workspace: this.workspace,
      path: nodePath,
    });
  }

  /**
   * Upload multiple files to this workspace
   *
   * Creates a batch upload with concurrency control and aggregate progress tracking.
   *
   * @param files - FileList, File[], or Blob[] to upload
   * @param basePath - Base path for all uploads (files go to basePath/filename)
   * @param options - Additional batch upload options
   * @returns BatchUpload controller
   *
   * @example
   * ```typescript
   * const ws = client.database('media').workspace('assets');
   * const files = document.querySelector('input[type="file"]').files;
   * const batch = await ws.uploadFiles(files, '/uploads', {
   *   concurrency: 3,
   *   onProgress: (p) => console.log(`${p.filesCompleted}/${p.filesTotal}`),
   * });
   * const result = await batch.start();
   * ```
   */
  async uploadFiles(
    files: FileList | File[] | Blob[],
    basePath: string,
    options?: Partial<Omit<BatchUploadOptions, 'repository' | 'workspace' | 'basePath'>>
  ): Promise<BatchUpload> {
    return this.client.uploadFiles(files, {
      ...options,
      repository: this.repository,
      workspace: this.workspace,
      basePath,
    });
  }

  /**
   * Get a signed URL for an asset in this workspace
   *
   * Convenience method that pre-fills repository and workspace.
   *
   * @param path - Path to the asset node
   * @param command - 'download' (forces download) or 'display' (shows inline)
   * @param options - Optional settings: expiresIn (seconds), propertyPath (e.g., 'thumbnail')
   * @returns Signed URL and expiry time
   *
   * @example
   * ```typescript
   * const ws = client.database('media').workspace('assets');
   * const { url } = await ws.signAssetUrl('/files/image.jpeg', 'display');
   * document.querySelector('img').src = url;
   *
   * // Get thumbnail instead of main file
   * const { url: thumbUrl } = await ws.signAssetUrl('/files/image.jpeg', 'display', { propertyPath: 'thumbnail' });
   * ```
   */
  async signAssetUrl(
    path: string,
    command: 'download' | 'display' = 'display',
    options?: { expiresIn?: number; propertyPath?: string }
  ): Promise<SignedAssetUrl> {
    return this.client.signAssetUrl({
      repository: this.repository,
      workspace: this.workspace,
      path,
      command,
      expiresIn: options?.expiresIn,
      propertyPath: options?.propertyPath,
    });
  }
}
