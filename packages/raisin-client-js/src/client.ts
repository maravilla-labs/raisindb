/**
 * RaisinDB WebSocket client
 */

import { EventEmitter } from 'events';
import { Connection, ConnectionState, ConnectionOptions } from './connection';
import { RequestTracker } from './utils/request-tracker';
import {
  AuthManager,
  Credentials,
  AdminCredentials,
  isJwtCredentials,
  TokenStorage,
  LocalStorageTokenStorage,
  IdentityUser,
  IdentityAuthResponse,
} from './auth';
import { EventHandler } from './events';
import { Database } from './database';
import {
  RequestEnvelope,
  ResponseEnvelope,
  RequestContext,
  RequestType,
  ResponseStatus,
  AuthenticatePayload,
  AuthenticateResponse,
  AuthenticateJwtPayload,
  AuthenticateJwtResponse,
  ConnectedMessage,
  encodeMessage,
  decodeMessage,
  isEventMessage,
  isResponseEnvelope,
  isConnectedMessage,
} from './protocol';
import { RaisinHttpClient, HttpClientOptions, SignAssetOptions, SignedAssetUrl } from './http-client';
import { RaisinAuthError, RaisinTimeoutError } from './errors';
import { logger, LogLevel, setLogLevel } from './logger';
import { normalizeHomePath } from './utils/home-path';
import type { Upload, UploadOptions, BatchUpload, BatchUploadOptions } from './upload/types';
import { createFileSource } from './upload/file-source';
import { UploadManager } from './upload/uploader';

/**
 * Client mode
 */
export type ClientMode = 'websocket' | 'http' | 'hybrid';

// ============================================================================
// Auth State Change Types (Firebase/Supabase-compatible pattern)
// ============================================================================

/**
 * Auth event types emitted by onAuthStateChange
 */
export type AuthEvent =
  | 'SIGNED_IN'
  | 'SIGNED_OUT'
  | 'TOKEN_REFRESHED'
  | 'SESSION_EXPIRED'
  | 'USER_UPDATED';

/**
 * Auth state change payload
 */
export interface AuthStateChange {
  /** The auth event that occurred */
  event: AuthEvent;
  /** Current session state */
  session: {
    /** Current identity user or null if signed out */
    user: IdentityUser | null;
    /** Current access token or null if signed out */
    accessToken: string | null;
    /**
     * Current EFFECTIVE roles (direct + group + inherited) for the connected
     * user, or null/undefined when signed out. Populated from the WS auth
     * handshake so role-gated consumers can read roles from the event without a
     * separate call. Updated on SIGNED_IN, USER_UPDATED (incl. after a
     * server-pushed permission change), and reconnect.
     */
    roles?: string[] | null;
  };
}

/**
 * Callback for auth state changes
 */
export type AuthStateChangeCallback = (change: AuthStateChange) => void;

/**
 * User change event payload (for user home node updates)
 */
export interface UserChangeEvent {
  /** The user's home node with updated data */
  node: UserNode;
  /** Type of change (e.g., 'node:updated') */
  changeType: string;
}

/**
 * Callback for user home node changes
 */
export type UserChangeCallback = (event: UserChangeEvent) => void;

/**
 * Client options
 */
export interface ClientOptions {
  /** Connection options */
  connection?: ConnectionOptions;
  /** Token storage (default: in-memory) */
  tokenStorage?: TokenStorage;
  /** Request timeout in milliseconds (default: 30000) */
  requestTimeout?: number;
  /** Tenant ID (extracted from URL if not provided; default: "default") */
  tenantId?: string;
  /**
   * Repository name. When the URL has no path (e.g. `ws://host:8081`),
   * the client connects to the tenant-less `/ws/{repository}` route.
   * When the URL already carries a path, this overrides the repository
   * extracted from the URL (used for repo-scoped auth endpoints).
   */
  repository?: string;
  /** Default branch (default: "main") */
  defaultBranch?: string;
  /** Client mode (default: "websocket") */
  mode?: ClientMode;
  /** Log level (default: Info - minimal logging) */
  logLevel?: LogLevel;
  /** HTTP base URL for identity auth (derived from WS URL if not provided) */
  httpBaseUrl?: string;
}

/**
 * RaisinDB client
 */
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
  /** The full user node from the repository (from CURRENT_USER()) */
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

export class RaisinClient extends EventEmitter {
  private connection: Connection;
  private requestTracker: RequestTracker;
  private authManager: AuthManager;
  private eventHandler: EventHandler;
  private _context: RequestContext;
  private options: Required<Omit<ClientOptions, 'connection' | 'tokenStorage' | 'mode' | 'logLevel' | 'httpBaseUrl' | 'repository'>>;
  private _currentUser: CurrentUser | null = null;
  /** In-flight role re-resolve, used to coalesce concurrent refreshCurrentUser() calls. */
  #refreshingUser: Promise<void> | null = null;
  private _httpBaseUrl: string;
  private _repository: string;

  // Auth state change listeners (Firebase/Supabase pattern)
  private _authListeners: Set<AuthStateChangeCallback> = new Set();
  private _userChangeListeners: Set<UserChangeCallback> = new Set();
  private _userHomeSubscription: { unsubscribe: () => Promise<void> } | null = null;

  // Ready state: connected AND (authenticated OR no stored token)
  private _ready: boolean = false;
  private _readyListeners: Set<(ready: boolean) => void> = new Set();

  // Reconnection tracking
  private _previousConnectionState: ConnectionState = ConnectionState.Disconnected;
  private _reconnectedListeners: Set<() => void> = new Set();
  private _everConnected = false;

  // Requests queued while the connection is re-establishing.
  // Flushed after reconnect + re-auth; bounded by MAX_QUEUED_REQUESTS.
  private _queuedRequests: Array<{ requestId: string; encoded: Uint8Array }> = [];
  /** Hard cap on requests held while reconnecting; overflow rejects immediately. */
  private static readonly MAX_QUEUED_REQUESTS = 100;

  /**
   * Backoff delays (ms) between subscription-restore retries after reconnect.
   * @internal - overridable in tests
   */
  private _restoreRetryDelaysMs: number[] = [1000, 3000, 9000];

  // Upload manager (uses HTTP under the hood)
  private _uploadManager: UploadManager | null = null;

  constructor(url: string, options: ClientOptions = {}) {
    super();

    // Configure log level if provided
    if (options.logLevel !== undefined) {
      setLogLevel(options.logLevel);
    }

    // When a repository option is given and the URL has no path, build the
    // tenant-less /ws/{repository} URL internally.
    const connectionUrl = this.buildConnectionUrl(url, options.repository);

    this.options = {
      requestTimeout: options.requestTimeout ?? 30000,
      tenantId: options.tenantId ?? this.extractTenantFromUrl(connectionUrl),
      defaultBranch: options.defaultBranch ?? 'main',
    };

    // Derive HTTP base URL from WebSocket URL if not provided
    this._httpBaseUrl = options.httpBaseUrl ?? this.deriveHttpUrl(connectionUrl);

    // Extract repository from URL for repo-scoped auth endpoints
    this._repository = options.repository ?? this.extractRepositoryFromUrl(connectionUrl);

    // Initialize context
    this._context = {
      tenant_id: this.options.tenantId,
      branch: this.options.defaultBranch,
    };

    // Send the tenant on the WebSocket upgrade request so tenant-less /ws
    // URLs resolve to the right tenant - but ONLY when the caller explicitly
    // provided one (or the URL carries one). Header ABSENCE is the protocol's
    // "use your default resolution" signal; always asserting 'default' would
    // fight proxies that inject the real tenant header. Only effective where
    // the WebSocket implementation supports custom headers (Node.js `ws`);
    // browsers ignore it and rely on the server-side resolution.
    const explicitTenant =
      options.tenantId ?? (this.extractTenantFromUrl(url) !== 'default' ? this.options.tenantId : undefined);
    const connectionOptions: ConnectionOptions = {
      ...options.connection,
      headers: {
        ...(explicitTenant ? { 'x-tenant-id': explicitTenant } : {}),
        ...options.connection?.headers,
      },
    };

    // Initialize components
    this.connection = this.createConnection(connectionUrl, connectionOptions);
    this.requestTracker = new RequestTracker({
      defaultTimeout: this.options.requestTimeout,
    });
    this.authManager = new AuthManager(options.tokenStorage);
    this.eventHandler = new EventHandler((payload, requestType) =>
      this.sendRequestInternal(payload, requestType)
    );

    // Set up event handlers
    this.setupConnectionHandlers();
  }

  /**
   * Create the underlying WebSocket connection.
   * Seam for tests to inject a fake connection.
   * @internal
   */
  protected createConnection(url: string, options?: ConnectionOptions): Connection {
    return new Connection(url, options);
  }

  /**
   * Derive HTTP URL from WebSocket URL
   * ws://host:port/path -> http://host:port
   * wss://host:port/path -> https://host:port
   */
  private deriveHttpUrl(wsUrl: string): string {
    try {
      const url = wsUrl
        .replace(/^raisin:\/\//, 'ws://')
        .replace(/^raisins:\/\//, 'wss://');
      const parsed = new URL(url);
      const protocol = parsed.protocol === 'wss:' ? 'https:' : 'http:';
      return `${protocol}//${parsed.host}`;
    } catch {
      return 'http://localhost:8081';
    }
  }

  /**
   * Build the WebSocket connection URL.
   *
   * When a repository option is provided and the URL carries no path
   * (e.g. `ws://host:8081`), build the tenant-less `/ws/{repository}`
   * route internally. URLs that already carry a path are used as-is.
   */
  private buildConnectionUrl(url: string, repository?: string): string {
    if (!repository) {
      return url;
    }
    try {
      const wsUrl = url.replace(/^raisin:\/\//, 'ws://').replace(/^raisins:\/\//, 'wss://');
      const parsed = new URL(wsUrl);
      const hasPath = parsed.pathname.split('/').some((p) => p.length > 0);
      if (hasPath) {
        return url;
      }
      return `${url.replace(/\/+$/, '')}/ws/${repository}`;
    } catch {
      return url;
    }
  }

  /**
   * Extract repository from URL
   * URL forms: /ws/{repo} (tenant-less, preferred), /sys/{tenant}/{repo}
   * (operator/multi-tenant), /{tenant}/{repo} (legacy).
   */
  private extractRepositoryFromUrl(url: string): string {
    try {
      const wsUrl = url.replace(/^raisin:\/\//, 'ws://').replace(/^raisins:\/\//, 'wss://');
      const parsed = new URL(wsUrl);
      const parts = parsed.pathname.split('/').filter((p) => p.length > 0);
      if (parts[0] === 'ws') {
        // /ws/{repo}
        return parts.length > 1 ? parts[1] : '';
      }
      if (parts[0] === 'sys') {
        // /sys/{tenant}/{repo}
        return parts.length > 2 ? parts[2] : '';
      }
      // Legacy /{tenant}/{repo}
      return parts.length > 1 ? parts[1] : '';
    } catch {
      return '';
    }
  }

  /**
   * Extract tenant ID from URL
   * URL forms: /ws/{repo} (tenant-less — no tenant in URL),
   * /sys/{tenant}/{repo} (canonical), /{tenant}/{repo} (legacy).
   * When the URL doesn't carry a tenant, fall back to 'default' — the
   * server's tenant middleware resolves it anyway (production derives the
   * tenant from the request itself), so an explicit tenantId is optional.
   */
  private extractTenantFromUrl(url: string): string {
    try {
      // Convert raisin:// to ws:// for URL parsing
      const wsUrl = url.replace(/^raisin:\/\//, 'ws://').replace(/^raisins:\/\//, 'wss://');
      const parsed = new URL(wsUrl);
      const parts = parsed.pathname.split('/').filter((p) => p.length > 0);
      if (parts[0] === 'ws') {
        // Tenant-less route: the server resolves the tenant from the
        // upgrade request (x-tenant-id header, falling back to 'default').
        return 'default';
      }
      if (parts[0] === 'sys') {
        return parts.length > 1 ? parts[1] : 'default';
      }
      return parts.length >= 2 ? parts[0] : 'default';
    } catch (error) {
      return 'default';
    }
  }

  /**
   * Set up connection event handlers
   */
  private setupConnectionHandlers(): void {
    this.connection.on('message', (data: ArrayBuffer) => {
      this.handleMessage(data);
    });

    this.connection.on('stateChange', (state: ConnectionState) => {
      const wasDisconnected =
        this._previousConnectionState === ConnectionState.Disconnected ||
        this._previousConnectionState === ConnectionState.Reconnecting;

      if (state === ConnectionState.Connected) {
        this._everConnected = true;
      }

      if (state === ConnectionState.Disconnected || state === ConnectionState.Closed) {
        // The connection is down for good (manual close, max reconnect
        // attempts reached, ...): cancel all pending requests, including
        // any held in the reconnect queue.
        this._queuedRequests = [];
        this.requestTracker.cancelAll();
      }

      // Handle reconnection: restore subscriptions after connection is re-established
      // Note: handleConnectedMessage() handles re-auth, then we restore subscriptions
      if (state === ConnectionState.Connected && wasDisconnected && this._previousConnectionState !== ConnectionState.Disconnected) {
        // This is a reconnection (not initial connection)
        // Subscription restoration happens after auth in handleConnectedMessage
        logger.info('[setupConnectionHandlers] Reconnection detected, will restore subscriptions after auth');
      }

      this._previousConnectionState = state;

      // Update ready state on all connection changes
      this._updateReadyState();
    });

    this.connection.on('error', (error: Error) => {
      logger.error('Connection error:', error);
    });
  }

  /**
   * Handle incoming message
   */
  private handleMessage(data: ArrayBuffer): void {
    logger.debug(`handleMessage() called - data size: ${data.byteLength} bytes`);
    try {
      logger.debug(`Attempting to decode MessagePack message...`);
      const message = decodeMessage(data);
      logger.debug(`Successfully decoded message`);
      logger.debug(`Message keys:`, Object.keys(message));
      logger.debug(`Message structure:`, JSON.stringify(message, null, 2));
      logger.debug(`Type check: isEvent=${isEventMessage(message)}, isResponse=${isResponseEnvelope(message)}, isConnected=${isConnectedMessage(message)}`);
      logger.debug(`Has 'status' property:`, 'status' in message);
      logger.debug(`Has 'subscription_id' property:`, 'subscription_id' in message);
      logger.debug(`Status value:`, (message as any).status);

      if (isConnectedMessage(message)) {
        // Handle connected message from server
        this.handleConnectedMessage(message);
      } else if (isEventMessage(message)) {
        // Out-of-band "your permissions may have changed" push (reserved
        // subscription id / event type, sent by the server after an
        // access-control write). Re-resolve roles instead of routing it to a
        // normal subscription callback. Keep in sync with the server
        // (raisin-transport-ws event_handler PERMISSIONS_EVENT_TYPE).
        if ((message as { event_type?: string }).event_type === 'permissions:changed') {
          logger.info('[permissions:changed] server signaled a role/permission change; re-resolving');
          void this.refreshCurrentUser();
        } else {
          // Handle event message
          logger.debug(`Routing to event handler`);
          this.eventHandler.handleEvent(message);
        }
      } else if (isResponseEnvelope(message)) {
        // Handle response message
        logger.debug(`Routing to response handler - request_id: ${(message as any).request_id}, status: ${(message as any).status}`);
        this.handleResponse(message);
      } else {
        logger.warn(`Unknown message type - not event or response`);
      }
    } catch (error) {
      logger.error('Error decoding message:', error);
      logger.error('Message data (first 100 bytes):', new Uint8Array(data.slice(0, 100)));
    }
  }

  /**
   * Handle connected message from server
   * Stores current user info for anonymous users
   *
   * NOTE: We do NOT store the anonymous token here because it would overwrite
   * any existing user JWT in localStorage. The anonymous token is only needed
   * for HTTP API calls when there's no existing session.
   */
  private handleConnectedMessage(message: ConnectedMessage): void {
    logger.info(`Connected to server - connection_id: ${message.connection_id}, anonymous: ${message.anonymous}`);

    // Check if this is a reconnection (we had subscriptions before)
    const hasActiveSubscriptions = this.eventHandler.getActiveSubscriptions().length > 0 ||
      this.eventHandler.hasStoredFilters();

    // Store current user info for anonymous users (but don't overwrite stored tokens!)
    if (message.anonymous && message.user_id) {
      // Only set anonymous user if we don't already have a stored token
      // (otherwise we're about to authenticate with the stored token)
      const hasStoredToken = this.authManager.storage.getAccessToken() !== null;
      if (!hasStoredToken) {
        this._currentUser = {
          userId: message.user_id,
          anonymous: true,
          // Anonymous users don't have a node in the repository
        };
        logger.debug(`Anonymous user info stored: ${message.user_id}`);
        // Anonymous user is ready immediately
        this._updateReadyState();
        this._flushQueuedRequests();

        // Restore subscriptions for anonymous users on reconnect
        if (hasActiveSubscriptions) {
          this._restoreSubscriptionsAndNotify();
        }
      } else {
        // Auto-authenticate with stored token on reconnect
        logger.info(`[handleConnectedMessage] Stored token found, auto-authenticating...`);
        this.autoReauthenticate()
          .then(async (authenticated) => {
            if (authenticated) {
              // After successful re-auth, deliver requests queued while down
              this._flushQueuedRequests();
            } else {
              this._rejectQueuedRequests(
                new RaisinAuthError(
                  'Re-authentication failed after reconnect',
                  'AUTH_TOKEN_EXPIRED',
                  401
                )
              );
            }
            // Restore subscriptions regardless (they may be readable anonymously)
            if (hasActiveSubscriptions) {
              await this._restoreSubscriptionsAndNotify();
            }
          })
          .catch((err) => {
            logger.error(`[handleConnectedMessage] Auto-authenticate failed:`, err);
            this._rejectQueuedRequests(
              err instanceof Error ? err : new Error('Re-authentication failed after reconnect')
            );
          });
      }
    } else {
      // Non-anonymous connection message: nothing to re-authenticate here
      this._flushQueuedRequests();
    }

    // Emit connected event
    this.emit('connected', {
      connectionId: message.connection_id,
      anonymous: message.anonymous,
      userId: message.user_id,
    });
  }

  /**
   * Restore subscriptions after reconnection and notify listeners.
   *
   * Retries with backoff (see `_restoreRetryDelaysMs`); on final failure a
   * `subscription_restore_failed` event is emitted so applications can
   * resync their state (e.g. reload lists and re-subscribe manually).
   * @internal
   */
  private async _restoreSubscriptionsAndNotify(): Promise<void> {
    const delays = this._restoreRetryDelaysMs;
    let lastError: unknown;

    for (let attempt = 0; attempt <= delays.length; attempt++) {
      try {
        logger.info(
          `[_restoreSubscriptionsAndNotify] Restoring subscriptions (attempt ${attempt + 1}/${delays.length + 1})...`
        );
        await this.eventHandler.restoreSubscriptions({ retry: attempt > 0 });
        logger.info('[_restoreSubscriptionsAndNotify] Subscriptions restored successfully');

        // Notify reconnected listeners
        this._emitReconnected();
        return;
      } catch (error) {
        lastError = error;
        logger.error(
          `[_restoreSubscriptionsAndNotify] Restore attempt ${attempt + 1} failed:`,
          error
        );
        if (attempt < delays.length) {
          await new Promise((resolve) => setTimeout(resolve, delays[attempt]));
        }
      }
    }

    logger.error('[_restoreSubscriptionsAndNotify] Giving up after retries; events may be stale');
    this.emit(
      'subscription_restore_failed',
      lastError instanceof Error ? lastError : new Error(String(lastError))
    );
  }

  /**
   * Emit reconnected event to all listeners
   * @internal
   */
  private _emitReconnected(): void {
    logger.info('[_emitReconnected] Emitting reconnected event');
    for (const callback of this._reconnectedListeners) {
      try {
        callback();
      } catch (err) {
        logger.error('[_emitReconnected] Listener error:', err);
      }
    }
    this.emit('reconnected');
  }

  /**
   * Auto-reauthenticate with stored JWT on reconnection
   *
   * @returns true if the session was re-authenticated, false otherwise
   * @internal
   */
  private async autoReauthenticate(): Promise<boolean> {
    const token = this.authManager.storage.getAccessToken();
    if (!token) {
      logger.warn('[autoReauthenticate] No stored token');
      return false;
    }

    // Check if token is expired
    const payload = AuthManager.parseToken(token);
    if (payload?.exp && typeof payload.exp === 'number') {
      if (payload.exp * 1000 < Date.now()) {
        logger.warn('[autoReauthenticate] Token expired, attempting refresh...');
        const refreshed = await this.refreshToken();
        if (!refreshed) {
          logger.error('[autoReauthenticate] Token refresh failed, clearing session');
          this.authManager.clear();
          this._currentUser = null;
          this._emitAuthEvent('SESSION_EXPIRED');
          return false;
        }
        // Use the new token
        const newToken = this.authManager.storage.getAccessToken();
        if (newToken) {
          await this.authenticate({ type: 'jwt', token: newToken });
          return true;
        }
        return false;
      }
    }

    // Authenticate with the stored token
    try {
      await this.authenticate({ type: 'jwt', token });
      logger.info('[autoReauthenticate] Successfully re-authenticated');
      return true;
    } catch (err) {
      logger.error('[autoReauthenticate] Failed to authenticate:', err);
      // Try to refresh token
      const refreshed = await this.refreshToken();
      if (refreshed) {
        const newToken = this.authManager.storage.getAccessToken();
        if (newToken) {
          await this.authenticate({ type: 'jwt', token: newToken });
          return true;
        }
        return false;
      } else {
        this.authManager.clear();
        this._currentUser = null;
        this._emitAuthEvent('SESSION_EXPIRED');
        return false;
      }
    }
  }

  /**
   * Handle response message
   */
  private handleResponse(response: ResponseEnvelope): void {
    const requestId = response.request_id;
    logger.debug(`handleResponse() called - request_id: ${requestId}, status: ${response.status}`);

    if (response.status === ResponseStatus.Success || response.status === ResponseStatus.Complete) {
      logger.debug(`Resolving request ${requestId} with success result`);
      this.requestTracker.resolveRequest(requestId, response.result);
      logger.debug(`Request ${requestId} resolved successfully`);
    } else if (response.status === ResponseStatus.Error) {
      logger.debug(`Rejecting request ${requestId} with error: ${response.error?.message}`);
      const error = new Error(
        response.error?.message ?? 'Unknown error'
      );
      (error as any).code = response.error?.code;
      (error as any).details = response.error?.details;
      this.requestTracker.rejectRequest(requestId, error);
      logger.debug(`Request ${requestId} rejected with error`);
    } else if (response.status === ResponseStatus.Streaming) {
      // Handle streaming responses (could be enhanced later)
      // For now, just resolve with the current chunk
      logger.debug(`Resolving streaming response for request ${requestId}`);
      this.requestTracker.resolveRequest(requestId, response.result);
    } else if (response.status === ResponseStatus.Acknowledged) {
      // Request acknowledged, continue waiting
      logger.debug(`Request ${requestId} acknowledged, waiting for completion`);
    } else {
      logger.warn(`Unknown response status: ${response.status} for request ${requestId}`);
    }
  }

  /**
   * Connect to the server
   */
  async connect(): Promise<void> {
    await this.connection.connect();
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
  async authenticate(credentials: Credentials): Promise<void> {
    if (isJwtCredentials(credentials)) {
      // JWT authentication for identity users
      logger.info('[authenticate] Starting JWT authentication...');
      // Learn this token's expiry. Every bare-token path funnels through here
      // — session restore included — and without it `expiresAt` stays null,
      // which `isTokenExpired()` reads as expired. See AuthManager.adoptToken.
      this.authManager.adoptToken(credentials.token);
      const payload: AuthenticateJwtPayload = { token: credentials.token };
      const response = await this.sendRequestInternal(
        payload,
        RequestType.AuthenticateJwt
      ) as AuthenticateJwtResponse;

      // Store current user info from auth response
      // Note: User node can be fetched separately via fetchUserNode() if needed
      this._currentUser = {
        userId: response.user_id,
        roles: response.roles,
        anonymous: false,
      };
      logger.info(`[authenticate] JWT auth successful: user_id=${response.user_id}, roles=${JSON.stringify(response.roles)}`);

      // Emit authenticated event with JWT response
      this.emit('authenticated', response);
    } else {
      // Admin authentication with username/password
      const payload: AuthenticatePayload = AuthManager.createAuthPayload(credentials as AdminCredentials);
      const response = await this.sendRequestInternal(
        payload,
        RequestType.Authenticate
      ) as AuthenticateResponse;

      this.authManager.setTokens(response);

      // For admin auth, we don't have user_id in response, use 'admin' as placeholder
      this._currentUser = {
        userId: 'admin',
        anonymous: false,
        // Admin users don't have a node in the repository
      };

      // Emit authenticated event
      this.emit('authenticated', response);
    }
  }

  /**
   * Disconnect from the server
   */
  disconnect(): void {
    this.connection.disconnect();
    this.requestTracker.clear();
    this.eventHandler.clearAll();
    this.eventHandler.destroy();
    this.authManager.clear();
    this._currentUser = null;
  }

  // ============================================================================
  // Identity Authentication (Email/Password via HTTP)
  // ============================================================================

  /**
   * Login with email and password (identity auth)
   *
   * This uses HTTP /auth/{repo}/login endpoint, stores tokens, connects WebSocket,
   * authenticates the WebSocket, and fetches the user node via SQL.
   *
   * @param email - User's email address
   * @param password - User's password
   * @param repository - Repository name (used for auth endpoint)
   * @returns The authenticated user info
   *
   * @example
   * ```typescript
   * const client = new RaisinClient('ws://localhost:8081/sys/default/myrepo');
   * const user = await client.loginWithEmail('user@example.com', 'password', 'myrepo');
   * console.log('Logged in as:', user.email, 'home:', user.home);
   * ```
   */
  async loginWithEmail(
    email: string,
    password: string,
    repository: string
  ): Promise<IdentityUser> {
    logger.info('[loginWithEmail] Starting login...');

    // Override _repository with the explicitly provided value (URL extraction
    // can return the wrong segment for 3-segment paths like /sys/default/repo)
    if (repository) {
      this._repository = repository;
    }

    // 1. Call HTTP login endpoint
    const url = `${this._httpBaseUrl}/auth/${repository}/login`;
    logger.debug('[loginWithEmail] POST', url);

    const response = await this._fetchWithTimeout(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email, password }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: response.statusText }));
      // Real Error instance; keeps `{ code, message }` fields for
      // backwards-compatible destructuring (IdentityAuthError shape).
      throw new RaisinAuthError(
        error.message || 'Login failed',
        error.code || 'LOGIN_FAILED',
        response.status
      );
    }

    const tokens: IdentityAuthResponse = await response.json();
    logger.info('[loginWithEmail] HTTP login successful, user_id:', tokens.identity.id);

    // 2. Store tokens
    this.authManager.storage.setAccessToken(tokens.access_token);
    this.authManager.storage.setRefreshToken(tokens.refresh_token);

    // 3. Connect WebSocket if not connected
    if (!this.connection.isConnected()) {
      logger.debug('[loginWithEmail] Connecting WebSocket...');
      await this.connect();
    }

    // 4. Authenticate WebSocket with JWT
    logger.debug('[loginWithEmail] Authenticating WebSocket with JWT...');
    await this.authenticate({ type: 'jwt', token: tokens.access_token });

    // 5. Fetch user node via SQL to get the correct path (fallback if HTTP response has no home)
    logger.debug('[loginWithEmail] Fetching user node via SQL RAISIN_CURRENT_USER()...');
    const userNode = await this.fetchUserNode(repository);

    // 6. Build user object (HTTP response takes precedence, SQL is fallback)
    logger.info('[loginWithEmail] HTTP response identity.home:', tokens.identity.home);
    logger.info('[loginWithEmail] SQL userNode?.path:', userNode?.path);
    const user: IdentityUser = {
      id: tokens.identity.id,
      email: tokens.identity.email,
      displayName: tokens.identity.display_name,
      avatarUrl: tokens.identity.avatar_url,
      emailVerified: tokens.identity.email_verified,
      home: tokens.identity.home ?? userNode?.path ?? null,
    };

    // 7. Store user in localStorage if using LocalStorageTokenStorage
    if (this.authManager.storage instanceof LocalStorageTokenStorage) {
      this.authManager.storage.setUser(user);
    }

    // 8. Update current user.
    // Preserve the roles that authenticate({type:'jwt'}) just resolved from the
    // server (client.ts authenticate() sets _currentUser.roles = response.roles).
    // This overwrite exists to attach the fetched user node; it must NOT discard
    // the effective roles, or getCurrentUser().roles would always be [].
    this._currentUser = {
      userId: user.id,
      roles: this._currentUser?.roles ?? [],
      anonymous: false,
      node: userNode ?? undefined,
    };

    // 9. Schedule auto-refresh (expires_at minus 5 minutes)
    const expiresInMs = RaisinClient.expiresAtToMs(tokens.expires_at) - Date.now() - (5 * 60 * 1000);
    if (expiresInMs > 0) {
      this.scheduleAutoRefresh(expiresInMs);
    }

    // 10. Subscribe to user home changes if available
    if (user.home) {
      await this._subscribeToUserHome(repository, user.home);
    }

    // 11. Emit auth events
    logger.info('[loginWithEmail] Login complete, user home:', user.home);
    this._emitAuthEvent('SIGNED_IN');
    this.emit('login', user);
    return user;
  }

  /**
   * Register a new user with email and password
   *
   * @param email - User's email address
   * @param password - User's password
   * @param repository - Repository name
   * @param displayName - Optional display name
   * @returns The registered user info
   */
  async registerWithEmail(
    email: string,
    password: string,
    repository: string,
    displayName?: string
  ): Promise<IdentityUser> {
    logger.info('[registerWithEmail] Starting registration...');

    // 1. Call HTTP register endpoint
    const url = `${this._httpBaseUrl}/auth/${repository}/register`;
    const response = await this._fetchWithTimeout(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email, password, display_name: displayName }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: response.statusText }));
      // Real Error instance; keeps `{ code, message }` fields for
      // backwards-compatible destructuring (IdentityAuthError shape).
      throw new RaisinAuthError(
        error.message || 'Registration failed',
        error.code || 'REGISTRATION_FAILED',
        response.status
      );
    }

    const tokens: IdentityAuthResponse = await response.json();
    logger.info('[registerWithEmail] Registration successful, user_id:', tokens.identity.id);

    // 2. Store tokens
    this.authManager.storage.setAccessToken(tokens.access_token);
    this.authManager.storage.setRefreshToken(tokens.refresh_token);

    // 3. Connect WebSocket if not connected
    if (!this.connection.isConnected()) {
      await this.connect();
    }

    // 4. Authenticate WebSocket with JWT
    await this.authenticate({ type: 'jwt', token: tokens.access_token });

    // 5. Fetch user node - note: might be null initially until job completes
    // Wait a bit for the user node creation job to complete
    await new Promise(resolve => setTimeout(resolve, 500));
    const userNode = await this.fetchUserNode(repository);

    // 6. Build user object (HTTP response takes precedence, SQL is fallback)
    logger.info('[registerWithEmail] HTTP response identity.home:', tokens.identity.home);
    logger.info('[registerWithEmail] SQL userNode?.path:', userNode?.path);
    const user: IdentityUser = {
      id: tokens.identity.id,
      email: tokens.identity.email,
      displayName: tokens.identity.display_name,
      avatarUrl: tokens.identity.avatar_url,
      emailVerified: tokens.identity.email_verified,
      home: tokens.identity.home ?? userNode?.path ?? null,
    };

    // 7. Store user in localStorage if using LocalStorageTokenStorage
    if (this.authManager.storage instanceof LocalStorageTokenStorage) {
      this.authManager.storage.setUser(user);
    }

    // 8. Update current user.
    // Preserve the roles that authenticate({type:'jwt'}) just resolved from the
    // server (client.ts authenticate() sets _currentUser.roles = response.roles).
    // This overwrite exists to attach the fetched user node; it must NOT discard
    // the effective roles, or getCurrentUser().roles would always be [].
    this._currentUser = {
      userId: user.id,
      roles: this._currentUser?.roles ?? [],
      anonymous: false,
      node: userNode ?? undefined,
    };

    // 9. Schedule auto-refresh (expires_at minus 5 minutes)
    const expiresInMs = RaisinClient.expiresAtToMs(tokens.expires_at) - Date.now() - (5 * 60 * 1000);
    if (expiresInMs > 0) {
      this.scheduleAutoRefresh(expiresInMs);
    }

    // 10. Subscribe to user home changes if available
    if (user.home) {
      await this._subscribeToUserHome(repository, user.home);
    }

    // 11. Emit auth events
    logger.info('[registerWithEmail] Registration complete, user home:', user.home);
    this._emitAuthEvent('SIGNED_IN');
    this.emit('register', user);
    return user;
  }

  /**
   * Logout - clear tokens and handle reconnection
   *
   * For SPAs, use `reconnect: true` (default) to stay connected as anonymous.
   * This allows the app to continue functioning without auth.
   *
   * @param options.reconnect - Reconnect as anonymous after logout (default: true for SPAs)
   * @param options.disconnect - Fully disconnect without reconnecting (default: false)
   *
   * @example
   * ```typescript
   * // SPA: logout and reconnect as anonymous (default)
   * await client.logout();
   *
   * // Fully disconnect (e.g., closing app)
   * await client.logout({ disconnect: true });
   * ```
   */
  async logout(options: { disconnect?: boolean; reconnect?: boolean } = {}): Promise<void> {
    logger.info('[logout] Logging out...');

    // Clear auto-refresh timer
    if (this._refreshTimer) {
      clearTimeout(this._refreshTimer);
      this._refreshTimer = undefined;
    }

    // Unsubscribe from user home
    await this._unsubscribeFromUserHome();

    // Clear tokens and user state
    this.authManager.clear();
    this._currentUser = null;

    // Determine behavior: reconnect by default unless disconnect is explicitly true
    const shouldDisconnect = options.disconnect === true;
    const shouldReconnect = !shouldDisconnect && (options.reconnect !== false);

    if (shouldReconnect) {
      // Disconnect and reconnect as anonymous (SPA flow)
      logger.info('[logout] Reconnecting as anonymous...');
      this.connection.disconnect();
      await this.connect();
      logger.info('[logout] Reconnected as anonymous');
    } else if (shouldDisconnect) {
      // Fully disconnect
      this.disconnect();
    }

    // Emit auth event
    this._emitAuthEvent('SIGNED_OUT');
    this.emit('logout');
  }

  /**
   * Initialize session from stored tokens
   *
   * This checks localStorage for existing tokens, connects WebSocket,
   * authenticates, and fetches the current user node. Call this on app startup.
   *
   * @param repository - Repository name for user node lookup
   * @returns The current user if session is valid, null otherwise
   *
   * @example
   * ```typescript
   * const client = new RaisinClient('ws://localhost:8081/sys/default/myrepo', {
   *   tokenStorage: new LocalStorageTokenStorage('myapp')
   * });
   * const user = await client.initSession('myrepo');
   * if (user) {
   *   console.log('Restored session for:', user.email);
   * } else {
   *   // Redirect to login
   * }
   * ```
   */
  async initSession(repository: string): Promise<IdentityUser | null> {
    logger.info('[initSession] Initializing session from storage...');

    // Override _repository with the explicitly provided value (URL extraction
    // can return the wrong segment for 3-segment paths like /sys/default/repo)
    if (repository) {
      this._repository = repository;
    }

    const token = this.authManager.storage.getAccessToken();

    // Always connect WebSocket first (even for anonymous)
    if (!this.connection.isConnected()) {
      logger.debug('[initSession] Connecting WebSocket...');
      await this.connect();
    }

    if (!token) {
      logger.info('[initSession] No stored token found, connected as anonymous');
      return null;
    }

    // Check if token is expired (basic check)
    const payload = AuthManager.parseToken(token);
    if (payload && payload.exp && typeof payload.exp === 'number') {
      if (payload.exp * 1000 < Date.now()) {
        logger.info('[initSession] Token expired, attempting refresh...');

        // Delegate to refreshToken() rather than issuing our own refresh POST.
        // Refresh tokens are single-use (the server rotates them and revokes the
        // session on reuse), and connect() above can already have started a
        // refresh via autoReauthenticate(). Two refreshes racing on the same
        // stored token trip the server's reuse detection and kill the session.
        // refreshToken() is single-flighted, so both callers share one request.
        if (!this.authManager.storage.getRefreshToken()) {
          logger.warn('[initSession] No refresh token, clearing session');
          this.authManager.clear();
          return null;
        }

        const refreshed = await this.refreshToken();
        if (!refreshed) {
          logger.warn('[initSession] Token refresh failed, clearing session');
          this.authManager.clear();
          return null;
        }
        logger.info('[initSession] Token refreshed successfully');
      }
    }

    try {

      // Authenticate with stored token
      const currentToken = this.authManager.storage.getAccessToken();
      if (!currentToken) {
        return null;
      }

      logger.debug('[initSession] Authenticating WebSocket...');
      await this.authenticate({ type: 'jwt', token: currentToken });

      // Fetch user node via SQL
      logger.debug('[initSession] Fetching user node...');
      const userNode = await this.fetchUserNode(repository);

      // Get stored user or build from token
      let user: IdentityUser | null = null;
      if (this.authManager.storage instanceof LocalStorageTokenStorage) {
        user = this.authManager.storage.getUser();
      }

      if (!user && payload) {
        // Build user from JWT payload (home is encoded in the token)
        user = {
          id: payload.sub as string,
          email: payload.email as string || '',
          displayName: payload.display_name as string || null,
          avatarUrl: null,
          emailVerified: false,
          home: (payload.home as string) ?? null,
        };
      }

      if (user) {
        // Update user home: SQL result > JWT payload > stored value
        const jwtHome = (payload?.home as string) ?? null;
        user.home = userNode?.path ?? jwtHome ?? user.home;

        // Update stored user
        if (this.authManager.storage instanceof LocalStorageTokenStorage) {
          this.authManager.storage.setUser(user);
        }

        // Preserve roles resolved by authenticate({type:'jwt'}) above — do not
        // clobber them with [] (that made getCurrentUser().roles empty after a
        // localStorage session restore).
        this._currentUser = {
          userId: user.id,
          roles: this._currentUser?.roles ?? [],
          anonymous: false,
          node: userNode ?? undefined,
        };

        // Schedule auto-refresh based on token expiration
        const currentToken = this.authManager.storage.getAccessToken();
        if (currentToken) {
          const tokenPayload = AuthManager.parseToken(currentToken);
          if (tokenPayload?.exp && typeof tokenPayload.exp === 'number') {
            const expiresInMs = (tokenPayload.exp * 1000) - Date.now() - (5 * 60 * 1000);
            if (expiresInMs > 0) {
              this.scheduleAutoRefresh(expiresInMs);
            }
          }
        }

        // Subscribe to user home changes if available
        if (user.home) {
          await this._subscribeToUserHome(repository, user.home);
        }

        logger.info('[initSession] Session restored for user:', user.email, 'home:', user.home);
        this._emitAuthEvent('SIGNED_IN');
        this.emit('session_restored', user);
        return user;
      }

      return null;
    } catch (err) {
      logger.warn('[initSession] Failed to restore session:', err);
      return null;
    }
  }

  /**
   * Get the stored identity user (from localStorage)
   */
  getStoredUser(): IdentityUser | null {
    if (this.authManager.storage instanceof LocalStorageTokenStorage) {
      return this.authManager.storage.getUser();
    }
    return null;
  }

  /**
   * Check if there's a stored token (may or may not be valid)
   */
  hasStoredToken(): boolean {
    return this.authManager.storage.getAccessToken() !== null;
  }

  /**
   * Refresh the access token using the stored refresh token.
   *
   * This method:
   * 1. Gets the stored refresh token
   * 2. Calls the /auth/refresh endpoint
   * 3. Stores the new access and refresh tokens
   * 4. Returns the updated user info
   *
   * @returns The identity user if refresh was successful, null otherwise
   * @throws Error if no refresh token is stored or refresh fails
   */
  async refreshToken(): Promise<IdentityUser | null> {
    // Single-flight: refresh tokens are single-use. The server rotates the
    // token on every refresh and revokes the whole session if it ever sees a
    // stale generation (TOKEN_REUSE_DETECTED), since that is indistinguishable
    // from a stolen token being replayed. Several call sites can want a refresh
    // at once on startup — initSession(), connect() -> autoReauthenticate(),
    // the auto-refresh timer, and any 401 retry — so without this guard they
    // each POST the same stored token and we revoke our own session.
    if (this._refreshInFlight) {
      logger.debug('[refreshToken] Refresh already in flight, awaiting it');
      return this._refreshInFlight;
    }
    this._refreshInFlight = this._refreshToken().finally(() => {
      this._refreshInFlight = null;
    });
    return this._refreshInFlight;
  }

  /** In-flight refresh shared by concurrent refreshToken() callers. */
  private _refreshInFlight: Promise<IdentityUser | null> | null = null;

  private async _refreshToken(): Promise<IdentityUser | null> {
    const refreshToken = this.authManager.storage.getRefreshToken();
    if (!refreshToken) {
      logger.warn('[refreshToken] No refresh token available');
      return null;
    }

    try {
      logger.info('[refreshToken] Refreshing access token...');
      const url = this._repository
        ? `${this._httpBaseUrl}/auth/${this._repository}/refresh`
        : `${this._httpBaseUrl}/auth/refresh`;
      const response = await this._fetchWithTimeout(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ refresh_token: refreshToken }),
      });

      if (!response.ok) {
        const errorText = await response.text();
        logger.error('[refreshToken] Refresh failed:', response.status, errorText);

        // A revoked-for-security response means the token was presented twice.
        // With single-flighting in place that should no longer be us, so treat
        // it as a real signal (either a genuine replay, or a second client
        // instance sharing this storage) rather than a routine expiry.
        if (/TOKEN_REUSE_DETECTED|TOKEN_FAMILY_MISMATCH/.test(errorText)) {
          logger.error(
            '[refreshToken] Session revoked by the server as a suspected token replay. ' +
              'If this happened on a normal page load, two refreshes raced on the same token.',
          );
        }

        // Clear tokens on auth failure (401/403)
        if (response.status === 401 || response.status === 403) {
          this.authManager.clear();
          this._currentUser = null;
          this.emit('auth_error', new Error('Token refresh failed - session expired'));
        }
        return null;
      }

      const tokens: IdentityAuthResponse = await response.json();

      // Store new tokens
      this.authManager.storage.setAccessToken(tokens.access_token);
      this.authManager.storage.setRefreshToken(tokens.refresh_token);

      // Update user info
      const user: IdentityUser = {
        id: tokens.identity.id,
        email: tokens.identity.email,
        displayName: tokens.identity.display_name,
        avatarUrl: tokens.identity.avatar_url,
        emailVerified: tokens.identity.email_verified,
        home: null, // Will be updated on next query
      };

      if (this.authManager.storage instanceof LocalStorageTokenStorage) {
        this.authManager.storage.setUser(user);
      }

      // Schedule next refresh (token expires_at minus 5 minutes)
      const expiresInMs = RaisinClient.expiresAtToMs(tokens.expires_at) - Date.now() - (5 * 60 * 1000);
      if (expiresInMs > 0) {
        this.scheduleAutoRefresh(expiresInMs);
      }

      logger.info('[refreshToken] Token refreshed successfully');
      this._emitAuthEvent('TOKEN_REFRESHED');
      this.emit('token_refreshed', user);
      return user;
    } catch (err) {
      logger.error('[refreshToken] Error during token refresh:', err);
      return null;
    }
  }

  /**
   * Fetch with a timeout derived from `requestTimeout`.
   *
   * Aborts the request after `requestTimeout` ms and rejects with a
   * `RaisinTimeoutError`. If the caller provides its own AbortSignal in
   * `init.signal`, it is respected: an external abort rejects with the
   * original AbortError instead of a timeout error.
   * @internal
   */
  private async _fetchWithTimeout(url: string, init: RequestInit = {}): Promise<Response> {
    const timeoutMs = this.options.requestTimeout;
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);

    const externalSignal = init.signal ?? undefined;
    if (externalSignal) {
      if (externalSignal.aborted) {
        controller.abort();
      } else {
        externalSignal.addEventListener('abort', () => controller.abort(), { once: true });
      }
    }

    try {
      return await fetch(url, { ...init, signal: controller.signal });
    } catch (error) {
      if (
        error instanceof Error &&
        error.name === 'AbortError' &&
        !externalSignal?.aborted
      ) {
        throw new RaisinTimeoutError(
          `Request to ${url} timed out after ${timeoutMs}ms`,
          'REQUEST_TIMEOUT',
          timeoutMs
        );
      }
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }

  /**
   * Normalize a token `expires_at` value to epoch milliseconds.
   *
   * Server endpoints are inconsistent: some return seconds, others
   * milliseconds. Anything above 1e11 can't be a seconds-epoch (year 5138),
   * so treat it as milliseconds.
   * @internal
   */
  private static expiresAtToMs(expiresAt: number): number {
    return expiresAt > 100_000_000_000 ? expiresAt : expiresAt * 1000;
  }

  /**
   * Schedule automatic token refresh
   * @internal
   */
  private scheduleAutoRefresh(delayMs: number): void {
    // Clear any existing timer
    if (this._refreshTimer) {
      clearTimeout(this._refreshTimer);
    }

    // Clamp to a sane window: at least 30s out (avoid refresh storms on
    // skewed clocks), at most ~24 days (setTimeout 32-bit overflow fires
    // immediately otherwise).
    const clamped = Math.min(Math.max(delayMs, 30_000), 0x7fffffff);

    logger.debug(`[scheduleAutoRefresh] Scheduling refresh in ${Math.round(clamped / 1000 / 60)} minutes`);

    this._refreshTimer = setTimeout(async () => {
      logger.info('[autoRefresh] Auto-refreshing token...');
      const user = await this.refreshToken();
      if (!user) {
        // The session is over and NOBODY IS LOOKING. This timer is the only
        // thing that runs when a tab sits idle past its refresh window, and
        // dropping the failure here meant the app was never told: an
        // unauthenticated read returns an empty result rather than a 401, so
        // the UI kept rendering, kept reading, and kept getting nothing —
        // which looks like the user's content disappeared.
        //
        // Same teardown as the reconnect path in autoReauthenticate(), which
        // was until now the only emitter of SESSION_EXPIRED.
        logger.error('[autoRefresh] Refresh failed, clearing session');
        this.authManager.clear();
        this._currentUser = null;
        this._emitAuthEvent('SESSION_EXPIRED');
      }
    }, clamped);
  }

  /** Timer for automatic token refresh */
  private _refreshTimer?: ReturnType<typeof setTimeout>;

  /**
   * Get a database/repository interface
   *
   * @param name - Repository name
   * @returns Database interface
   */
  database(name: string): Database {
    // Set repository in client context for this database
    this._context.repository = name;
    return new Database(
      name,
      this._context,
      (payload, requestType, contextOverride, requestOptions) =>
        this.sendRequestInternal(payload, requestType as RequestType, contextOverride, requestOptions),
      this.eventHandler,
      undefined,
      undefined,
      () => this.getUploadManager(),
      (options) => this.signAssetUrl(options),
      { httpBaseUrl: this._httpBaseUrl, authManager: this.authManager },
    );
  }

  /**
   * Send a request to the server
   *
   * If the connection is temporarily down (connecting/reconnecting, or
   * disconnected with auto-reconnect enabled), the request is queued and
   * flushed once the connection is re-established and re-authenticated.
   * Queued requests are still subject to their own request timeout, and the
   * queue is bounded (overflow rejects immediately).
   */
  private async sendRequestInternal(
    payload: unknown,
    requestType: RequestType | string,
    contextOverride?: RequestContext,
    requestOptions?: { timeoutMs?: number }
  ): Promise<unknown> {
    const connected = this.connection.isConnected();
    if (!connected) {
      const state = this.connection.getState();
      const willReconnect =
        state === ConnectionState.Connecting ||
        state === ConnectionState.Reconnecting ||
        (state === ConnectionState.Disconnected &&
          this._everConnected &&
          this.connection.isAutoReconnectEnabled());

      if (!willReconnect) {
        throw new Error('Not connected to server');
      }

      if (this._queuedRequests.length >= RaisinClient.MAX_QUEUED_REQUESTS) {
        throw new Error(
          `Request queue is full (${RaisinClient.MAX_QUEUED_REQUESTS} requests pending while reconnecting)`
        );
      }
    }

    const requestId = this.requestTracker.generateRequestId();

    // Use provided context or default to this._context
    const context = contextOverride || this._context;

    logger.debug(`sendRequestInternal - request_id: ${requestId}, type: ${requestType}`);
    logger.debug(`Context:`, JSON.stringify(context, null, 2));

    // Create request envelope
    const request: RequestEnvelope = {
      request_id: requestId,
      type: requestType as RequestType,
      context,
      payload,
    };

    // Create promise for response
    const responsePromise = this.requestTracker.createRequest(
      requestId,
      requestOptions?.timeoutMs,
    );

    // Send request (or queue it while the connection is re-establishing)
    try {
      const encoded = encodeMessage(request);
      logger.debug(`Encoded request - ID: ${requestId}, type: ${requestType}, size: ${encoded.length} bytes`);
      if (connected) {
        this.connection.send(encoded);
        logger.debug(`Request sent - ID: ${requestId}`);
      } else {
        this._queuedRequests.push({ requestId, encoded });
        logger.debug(
          `Request queued while reconnecting - ID: ${requestId}, queue size: ${this._queuedRequests.length}`
        );
      }
    } catch (error) {
      logger.error(`Failed to send request - ID: ${requestId}:`, error);
      this.requestTracker.rejectRequest(
        requestId,
        error instanceof Error ? error : new Error('Failed to send request')
      );
      throw error;
    }

    return responsePromise;
  }

  /**
   * Flush requests queued while the connection was re-establishing.
   * Called after the connection is connected and (re-)authenticated.
   * @internal
   */
  private _flushQueuedRequests(): void {
    if (this._queuedRequests.length === 0) return;

    const queued = this._queuedRequests;
    this._queuedRequests = [];
    logger.info(`[_flushQueuedRequests] Flushing ${queued.length} queued request(s)`);

    for (const { requestId, encoded } of queued) {
      // Skip requests that already timed out while queued
      if (!this.requestTracker.hasPendingRequest(requestId)) continue;
      try {
        this.connection.send(encoded);
      } catch (error) {
        this.requestTracker.rejectRequest(
          requestId,
          error instanceof Error ? error : new Error('Failed to send queued request')
        );
      }
    }
  }

  /**
   * Reject all requests queued while the connection was re-establishing
   * (e.g. re-authentication failed after reconnect).
   * @internal
   */
  private _rejectQueuedRequests(error: Error): void {
    if (this._queuedRequests.length === 0) return;

    const queued = this._queuedRequests;
    this._queuedRequests = [];
    logger.warn(`[_rejectQueuedRequests] Rejecting ${queued.length} queued request(s): ${error.message}`);

    for (const { requestId } of queued) {
      this.requestTracker.rejectRequest(requestId, error);
    }
  }

  /**
   * Get the HTTP base URL derived from the WebSocket URL
   */
  get httpBaseUrl(): string {
    return this._httpBaseUrl;
  }

  /**
   * Get current connection state
   */
  getConnectionState(): ConnectionState {
    return this.connection.getState();
  }

  /**
   * Check if connected
   */
  isConnected(): boolean {
    return this.connection.isConnected();
  }

  /**
   * Check if ready (connected AND authenticated)
   *
   * Returns true when the client is fully ready to make requests:
   * - WebSocket is connected AND
   * - Either authenticated (signed in) OR no stored token (anonymous is ok)
   *
   * Use this for UI indicators that should show "connected" only when
   * the client is actually ready to process requests.
   */
  isReady(): boolean {
    return this._ready;
  }

  /**
   * Subscribe to ready state changes
   *
   * The callback is invoked whenever the ready state changes.
   * Ready = connected AND (authenticated OR no stored token)
   *
   * @example
   * ```typescript
   * const unsubscribe = client.onReadyStateChange((ready) => {
   *   if (ready) {
   *     console.log('Client is ready');
   *     showGreenIndicator();
   *   } else {
   *     console.log('Client is not ready');
   *     showRedIndicator();
   *   }
   * });
   * ```
   *
   * @param callback - Function called when ready state changes
   * @returns Unsubscribe function
   */
  onReadyStateChange(callback: (ready: boolean) => void): () => void {
    this._readyListeners.add(callback);
    // Immediately call with current state
    callback(this._ready);
    return () => {
      this._readyListeners.delete(callback);
    };
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
   * their ID, node path in the repository, and roles.
   *
   * @returns Current user info or null if not connected/authenticated
   */
  getCurrentUser(): CurrentUser | null {
    return this._currentUser;
  }

  /**
   * Re-resolve the current user's EFFECTIVE roles from the server and refresh
   * `getCurrentUser()`.
   *
   * Roles are deliberately not embedded in the JWT (they are resolved live
   * server-side), so when an admin changes a user's roles/groups the token does
   * not change — the client must re-ask. This re-runs the WS JWT auth handshake
   * (which returns freshly resolved effective roles), updates `_currentUser`, and
   * emits `USER_UPDATED` so role-gated UI can react without a reload.
   *
   * Called automatically when the server pushes `permissions:changed` after an
   * access-control write, and exposed for on-demand refresh. Safe no-op when not
   * authenticated.
   */
  async refreshCurrentUser(): Promise<void> {
    // Coalesce concurrent re-resolves: a burst of `permissions:changed` pushes
    // (e.g. during package/role seeding) collapses into a single in-flight
    // handshake instead of a storm.
    if (this.#refreshingUser) return this.#refreshingUser;
    const token = this.authManager.storage.getAccessToken();
    if (!token) return;
    this.#refreshingUser = (async () => {
      const before = JSON.stringify(this._currentUser?.roles ?? null);
      const prevNode = this._currentUser?.node;
      try {
        // authenticate({type:'jwt'}) re-resolves roles and rebuilds _currentUser
        // (without the node); preserve the previously-fetched node.
        await this.authenticate({ type: 'jwt', token });
        if (this._currentUser && prevNode && !this._currentUser.node) {
          this._currentUser.node = prevNode;
        }
        // Only notify when the effective roles actually changed — avoids
        // redundant USER_UPDATED churn when a push carried no role change.
        const after = JSON.stringify(this._currentUser?.roles ?? null);
        if (after !== before) {
          this._emitAuthEvent('USER_UPDATED');
        }
      } catch (err) {
        logger.warn('[refreshCurrentUser] role re-resolve failed:', err);
      } finally {
        this.#refreshingUser = null;
      }
    })();
    return this.#refreshingUser;
  }

  /**
   * Get current user ID
   *
   * @returns User ID or null if not connected/authenticated
   */
  getCurrentUserId(): string | null {
    return this._currentUser?.userId ?? null;
  }

  /**
   * Get current user's node path in the repository
   *
   * @returns User's node path (e.g., '/users/internal/john-at-example-com') or null if not connected
   */
  getCurrentUserPath(): string | null {
    return this._currentUser?.node?.path ?? null;
  }

  /**
   * Fetch user node using SQL RAISIN_CURRENT_USER() function
   *
   * @param repository - Repository name to query
   * @returns User's full node from the repository, or null if query fails
   */
  async fetchUserNode(repository: string): Promise<UserNode | null> {
    logger.info('[fetchUserNode] Fetching user node for repository:', repository);
    try {
      // Use SQL to get the current user node (RAISIN_CURRENT_USER() returns the node as JSON)
      const db = this.database(repository);
      logger.info('[fetchUserNode] Executing SQL: SELECT RAISIN_CURRENT_USER() as user_node');
      const result = await db.executeSql('SELECT RAISIN_CURRENT_USER() as user_node');
      logger.info('[fetchUserNode] SQL result:', JSON.stringify(result));

      if (result.rows && result.rows.length > 0) {
        const row = result.rows[0] as unknown as { user_node: UserNode | null };
        logger.info('[fetchUserNode] User node:', row.user_node ? `path=${row.user_node.path}` : 'null');
        return row.user_node ?? null;
      }
      logger.info('[fetchUserNode] No rows returned');
      return null;
    } catch (err) {
      logger.warn('[fetchUserNode] Error fetching user node via SQL:', err);
      return null;
    }
  }

  /**
   * Get current tenant ID
   */
  getTenantId(): string {
    return this._context.tenant_id;
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

  // ============================================================================
  // Auth State Change API (Firebase/Supabase-compatible)
  // ============================================================================

  /**
   * Listen for authentication state changes (Firebase/Supabase-compatible pattern)
   *
   * This method provides a reactive way to respond to auth events:
   * - SIGNED_IN: User logged in or session restored
   * - SIGNED_OUT: User logged out
   * - TOKEN_REFRESHED: Access token was refreshed
   * - SESSION_EXPIRED: Session expired (auto-logout triggered)
   * - USER_UPDATED: User's home node was updated
   *
   * @param callback - Function called on each auth state change
   * @returns Unsubscribe function to stop listening
   *
   * @example
   * ```typescript
   * const unsubscribe = client.onAuthStateChange(({ event, session }) => {
   *   if (event === 'SIGNED_IN') {
   *     console.log('User signed in:', session.user?.email);
   *   } else if (event === 'SIGNED_OUT') {
   *     console.log('User signed out');
   *   }
   * });
   *
   * // Later, to stop listening:
   * unsubscribe();
   * ```
   */
  onAuthStateChange(callback: AuthStateChangeCallback): () => void {
    this._authListeners.add(callback);
    return () => {
      this._authListeners.delete(callback);
    };
  }

  /**
   * Listen for connection state changes
   *
   * @param callback - Function called on each connection state change
   * @returns Unsubscribe function to stop listening
   *
   * @example
   * ```typescript
   * const unsubscribe = client.onConnectionStateChange((state) => {
   *   console.log('Connection state:', state);
   *   if (state === ConnectionState.Disconnected) {
   *     showOfflineIndicator();
   *   }
   * });
   * ```
   */
  onConnectionStateChange(callback: (state: ConnectionState) => void): () => void {
    const handler = (state: ConnectionState) => callback(state);
    this.connection.on('stateChange', handler);
    return () => {
      this.connection.off('stateChange', handler);
    };
  }

  /**
   * Listen for reconnection events
   *
   * This callback fires after a successful reconnection when:
   * 1. The WebSocket connection is re-established
   * 2. Re-authentication (if needed) is complete
   * 3. Event subscriptions have been restored
   *
   * Use this to refresh application data after a server restart or network recovery.
   *
   * @param callback - Function called after successful reconnection
   * @returns Unsubscribe function to stop listening
   *
   * @example
   * ```typescript
   * const unsubscribe = client.onReconnected(() => {
   *   console.log('Reconnected! Refreshing data...');
   *   // Refresh conversations, sync state, etc.
   *   await refreshData();
   * });
   * ```
   */
  onReconnected(callback: () => void): () => void {
    this._reconnectedListeners.add(callback);
    return () => {
      this._reconnectedListeners.delete(callback);
    };
  }

  /**
   * Listen for changes to the current user's home node
   *
   * This automatically subscribes to real-time updates on the user's home node
   * (e.g., /users/internal/john-at-example-com) when the user logs in.
   *
   * @param callback - Function called when user's home node is updated
   * @returns Unsubscribe function to stop listening
   *
   * @example
   * ```typescript
   * const unsubscribe = client.onUserChange(({ node, changeType }) => {
   *   console.log('User home updated:', changeType);
   *   console.log('New properties:', node.properties);
   *   // Update UI with new avatar, displayName, preferences, etc.
   * });
   * ```
   */
  onUserChange(callback: UserChangeCallback): () => void {
    this._userChangeListeners.add(callback);
    return () => {
      this._userChangeListeners.delete(callback);
    };
  }

  /**
   * Get current session (sync getter)
   *
   * @returns Current session with user and access token, or null if not authenticated
   *
   * @example
   * ```typescript
   * const session = client.getSession();
   * if (session) {
   *   console.log('Logged in as:', session.user?.email);
   * }
   * ```
   */
  getSession(): { user: IdentityUser | null; accessToken: string | null } | null {
    const accessToken = this.authManager.storage.getAccessToken();
    if (!accessToken) return null;
    return {
      user: this.getStoredUser(),
      accessToken,
    };
  }

  /**
   * Get current user (alias for getStoredUser, Supabase-compatible)
   *
   * @returns Current identity user or null
   */
  getUser(): IdentityUser | null {
    return this.getStoredUser();
  }

  /**
   * Emit auth event to all listeners
   * @internal
   */
  private _emitAuthEvent(event: AuthEvent): void {
    const session = {
      user: this.getStoredUser(),
      accessToken: this.authManager.storage.getAccessToken(),
      // Surface effective roles so role-gated consumers can react to
      // SIGNED_IN / USER_UPDATED without calling getCurrentUser() themselves.
      roles: this._currentUser?.roles ?? null,
    };
    const change: AuthStateChange = { event, session };

    for (const callback of this._authListeners) {
      try {
        callback(change);
      } catch (err) {
        logger.error('[_emitAuthEvent] Listener error:', err);
      }
    }

    // Update ready state after auth events
    this._updateReadyState();
  }

  /**
   * Update and emit ready state
   * Ready = connected AND (authenticated OR no stored token)
   * @internal
   */
  private _updateReadyState(): void {
    const connected = this.connection.isConnected();
    const hasToken = this.hasStoredToken();
    const user = this.getStoredUser();
    // Ready if: connected AND (no stored token OR user is signed in)
    const ready = connected && (!hasToken || user !== null);

    if (this._ready !== ready) {
      this._ready = ready;
      logger.debug('[_updateReadyState] Ready state changed:', ready, { connected, hasToken, user: user?.email });
      for (const callback of this._readyListeners) {
        try {
          callback(ready);
        } catch (err) {
          logger.error('[_updateReadyState] Listener error:', err);
        }
      }
    }
  }

  /**
   * Subscribe to user's home node for real-time updates
   * @internal
   */
  private async _subscribeToUserHome(repository: string, userHome: string): Promise<void> {
    // Unsubscribe from previous subscription if any
    await this._unsubscribeFromUserHome();

    // Strip a possible workspace prefix (e.g. "/raisin:access_control/users/...")
    userHome = normalizeHomePath(userHome) ?? userHome;

    try {
      const db = this.database(repository);
      // Extract workspace from path (e.g., "/users/internal/john" -> "users")
      const pathParts = userHome.split('/').filter(p => p.length > 0);
      if (pathParts.length === 0) {
        logger.warn('[_subscribeToUserHome] Invalid user home path:', userHome);
        return;
      }

      const workspace = pathParts[0];
      const events = db.workspace(workspace).events();

      logger.debug('[_subscribeToUserHome] Subscribing to user home:', userHome);
      const subscription = await events.subscribeToPath(userHome, (event) => {
        logger.debug('[_subscribeToUserHome] User home event:', event.event_type);

        // Emit to onUserChange listeners
        for (const callback of this._userChangeListeners) {
          try {
            callback({
              node: event.payload as unknown as UserNode,
              changeType: event.event_type,
            });
          } catch (err) {
            logger.error('[_subscribeToUserHome] Listener error:', err);
          }
        }

        // Also emit USER_UPDATED auth event
        this._emitAuthEvent('USER_UPDATED');
      });

      this._userHomeSubscription = subscription;
      logger.info('[_subscribeToUserHome] Subscribed to user home:', userHome);
    } catch (err) {
      logger.error('[_subscribeToUserHome] Failed to subscribe:', err);
    }
  }

  /**
   * Unsubscribe from user's home node
   * @internal
   */
  private async _unsubscribeFromUserHome(): Promise<void> {
    if (this._userHomeSubscription) {
      try {
        await this._userHomeSubscription.unsubscribe();
      } catch (err) {
        logger.warn('[_unsubscribeFromUserHome] Error during unsubscribe:', err);
      }
      this._userHomeSubscription = null;
    }
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
    return this.sendRequestInternal(
      {
        repository_id: repositoryId,
        description,
        config,
      },
      RequestType.RepositoryCreate
    );
  }

  /**
   * Get a repository by ID
   *
   * @param repositoryId - Repository identifier
   * @returns Repository information
   */
  async getRepository(repositoryId: string): Promise<unknown> {
    return this.sendRequestInternal(
      {
        repository_id: repositoryId,
      },
      RequestType.RepositoryGet
    );
  }

  /**
   * List all repositories
   *
   * @returns Array of repository information
   */
  async listRepositories(): Promise<unknown[]> {
    const result = await this.sendRequestInternal(
      {},
      RequestType.RepositoryList
    );
    return Array.isArray(result) ? result : [];
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
    return this.sendRequestInternal(
      {
        repository_id: repositoryId,
        description,
        config,
      },
      RequestType.RepositoryUpdate
    );
  }

  /**
   * Delete a repository
   *
   * @param repositoryId - Repository identifier
   * @returns Repository deletion result
   */
  async deleteRepository(repositoryId: string): Promise<unknown> {
    return this.sendRequestInternal(
      {
        repository_id: repositoryId,
      },
      RequestType.RepositoryDelete
    );
  }

  // ============================================================================
  // Upload Methods (use HTTP under the hood)
  // ============================================================================

  /**
   * Get or create the upload manager
   * @internal
   */
  private getUploadManager(): UploadManager {
    if (!this._uploadManager) {
      this._uploadManager = new UploadManager({
        baseUrl: this._httpBaseUrl,
        authManager: this.authManager,
        fetchImpl: fetch,
        requestTimeout: this.options.requestTimeout,
      });
    }
    return this._uploadManager;
  }

  /**
   * Create a resumable upload from a File or Blob (Browser)
   *
   * Creates an upload session for large files with progress tracking,
   * pause/resume, and automatic retry support. Uses HTTP under the hood.
   *
   * @param file - File or Blob to upload
   * @param options - Upload options
   * @returns Upload controller
   *
   * @example
   * ```typescript
   * const client = new RaisinClient('raisin://localhost:8080/sys/default');
   * await client.connect();
   *
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
   * pause/resume, and automatic retry support. Uses HTTP under the hood.
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
   * Upload multiple files as a batch
   *
   * Creates a batch upload with concurrency control and aggregate progress tracking.
   * Uses HTTP under the hood.
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
   * Get an existing upload by ID
   *
   * @param uploadId - ID of the upload to retrieve
   * @returns Upload controller or undefined if not found
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

    const response = await this._fetchWithTimeout(`${this._httpBaseUrl}${endpoint}`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(this.authManager.isAuthenticated()
          ? { Authorization: `Bearer ${this.authManager.getAccessToken()}` }
          : {}),
      },
      body: JSON.stringify({
        command: options.command,
        expires_in: options.expiresIn ?? 300,
      }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: 'Unknown error' }));
      throw new Error(error.message || `Failed to sign asset URL: ${response.status}`);
    }

    const data = await response.json();
    return {
      url: `${this._httpBaseUrl}${data.url}`,
      expiresAt: data.expiresAt ?? data.expires_at,
    };
  }

  /**
   * Create an HTTP-only client for server-side rendering
   *
   * This client uses HTTP REST API instead of WebSocket, making it suitable
   * for server-side rendering where WebSocket connections are not available.
   *
   * @param baseUrl - Base HTTP URL (e.g., "http://localhost:8080")
   * @param options - HTTP client options
   * @returns HTTP client instance
   *
   * @example
   * ```typescript
   * // In a React Router loader (server-side)
   * const client = RaisinClient.forSSR('http://localhost:8080', {
   *   tenantId: 'default'
   * });
   * await client.authenticate({ username: 'admin', password: 'admin' });
   * const result = await client.database('social').executeSql('SELECT * FROM nodes');
   * ```
   */
  static forSSR(baseUrl: string, options?: HttpClientOptions): RaisinHttpClient {
    return new RaisinHttpClient(baseUrl, options);
  }

  /**
   * Create an HTTP client (alias for forSSR)
   *
   * @param baseUrl - Base HTTP URL
   * @param options - HTTP client options
   * @returns HTTP client instance
   */
  static createHttpClient(baseUrl: string, options?: HttpClientOptions): RaisinHttpClient {
    return RaisinClient.forSSR(baseUrl, options);
  }
}
