/**
 * RaisinDB client singleton for launchpad frontend.
 *
 * Uses the @raisindb/client SDK with LocalStorageTokenStorage for
 * persistent authentication across page reloads.
 */
import {
  RaisinClient,
  LocalStorageTokenStorage,
  ConnectionState,
  type IdentityUser,
  type Database,
  type AuthStateChange,
  type UserChangeEvent,
} from '@raisindb/client';
import { browser } from '$app/environment';
import { localeClause } from '$lib/stores/locale';

// Configuration
const RAISIN_URL = 'wss://192.168.1.180:8443/sys/default/launchpad-next';
const TENANT_ID = 'default';
const REPOSITORY = 'launchpad-next';
const WORKSPACE_NAME = 'launchpad';

// Singleton client instance
let clientInstance: RaisinClient | null = null;
let dbInstance: Database | null = null;

// Connection gate - queries wait for this before executing
let connectionPromise: Promise<void> | null = null;
let connectionResolve: (() => void) | null = null;

/**
 * Get or create the RaisinDB client instance
 */
export function getClient(): RaisinClient {
  if (!browser) {
    throw new Error('RaisinClient can only be used in the browser');
  }

  if (!clientInstance) {
    clientInstance = new RaisinClient(RAISIN_URL, {
      tokenStorage: new LocalStorageTokenStorage('launchpad-next'),
      tenantId: TENANT_ID,
      defaultBranch: 'main',
      connection: {
        autoReconnect: true,
        heartbeatInterval: 30000,
      },
      requestTimeout: 30000,
    });
  }

  return clientInstance;
}

/**
 * Initialize session - restores auth from localStorage and connects WebSocket
 *
 * This MUST be called before any queries. Creates a connection gate that
 * other functions will wait for.
 *
 * @returns The authenticated user or null if no valid session
 */
export async function initSession(): Promise<IdentityUser | null> {
  // Create connection gate - queries will wait for this
  if (!connectionPromise) {
    connectionPromise = new Promise((resolve) => {
      connectionResolve = resolve;
    });
  }

  const client = getClient();

  try {
    const user = await client.initSession(REPOSITORY);
    if (!user) {
      // Connect anyway for anonymous access
      if (!client.isConnected()) {
        await client.connect();
      }
    }

    // Open the gate - queries can now proceed
    if (connectionResolve) {
      connectionResolve();
    }

    return user;
  } catch (error) {
    // Open the gate even on error so queries don't hang forever
    if (connectionResolve) {
      connectionResolve();
    }
    throw error;
  }
}

/**
 * Login with email and password
 */
export async function login(email: string, password: string): Promise<IdentityUser> {
  const client = getClient();
  return client.loginWithEmail(email, password, REPOSITORY);
}

/**
 * Register a new user
 */
export async function register(email: string, password: string, displayName?: string): Promise<IdentityUser> {
  const client = getClient();
  return client.registerWithEmail(email, password, REPOSITORY, displayName);
}

/**
 * Logout - clears tokens and reconnects as anonymous
 */
export async function logout(): Promise<void> {
  const client = getClient();

  // SDK handles: clear tokens, disconnect, reconnect as anonymous
  await client.logout();

  // Clear cached instances so they get recreated
  dbInstance = null;
}

/**
 * Get the current authenticated user (sync)
 */
export function getUser(): IdentityUser | null {
  const client = getClient();
  return client.getUser();
}

/**
 * Get current session info
 */
export function getSession(): { user: IdentityUser | null; accessToken: string | null } | null {
  const client = getClient();
  return client.getSession();
}

/**
 * Check if there's a stored token
 */
export function hasStoredToken(): boolean {
  const client = getClient();
  return client.hasStoredToken();
}

/**
 * Subscribe to auth state changes (Firebase/Supabase pattern)
 * Returns an unsubscribe function
 */
export function onAuthStateChange(callback: (change: AuthStateChange) => void): () => void {
  const client = getClient();
  return client.onAuthStateChange(callback);
}

/**
 * Subscribe to user home node changes
 * Returns an unsubscribe function
 */
export function onUserChange(callback: (event: UserChangeEvent) => void): () => void {
  const client = getClient();
  return client.onUserChange(callback);
}

/**
 * Subscribe to connection state changes
 * Returns an unsubscribe function
 */
export function onConnectionStateChange(callback: (state: ConnectionState) => void): () => void {
  const client = getClient();
  return client.onConnectionStateChange(callback);
}

/**
 * Get current connection state
 */
export function getConnectionState(): ConnectionState {
  const client = getClient();
  return client.getConnectionState();
}

/**
 * Check if connected
 */
export function isConnected(): boolean {
  const client = getClient();
  return client.isConnected();
}

/**
 * Check if ready (connected AND authenticated)
 * Use this for UI indicators
 */
export function isReady(): boolean {
  const client = getClient();
  return client.isReady();
}

/**
 * Subscribe to ready state changes
 * Ready = connected AND (authenticated OR no stored token)
 */
export function onReadyStateChange(callback: (ready: boolean) => void): () => void {
  const client = getClient();
  return client.onReadyStateChange(callback);
}

/**
 * Subscribe to reconnection events
 *
 * This callback fires after a successful reconnection when:
 * 1. The WebSocket connection is re-established
 * 2. Re-authentication (if needed) is complete
 * 3. Event subscriptions have been restored by the SDK
 *
 * Use this to refresh application data after a server restart or network recovery.
 * Note: The SDK automatically restores event subscriptions, so you only need to
 * refresh your data queries here.
 *
 * @param callback - Function called after successful reconnection
 * @returns Unsubscribe function to stop listening
 */
export function onReconnected(callback: () => void): () => void {
  const client = getClient();
  return client.onReconnected(callback);
}

/**
 * Get the database instance for SQL queries
 *
 * This waits for initSession() to complete before returning,
 * ensuring the connection is ready.
 */
export async function getDatabase(): Promise<Database> {
  // Wait for initSession() to complete
  if (connectionPromise) {
    await connectionPromise;
  }

  if (!dbInstance) {
    const client = getClient();
    if (!client.isConnected()) {
      await client.connect();
    }
    dbInstance = client.database(REPOSITORY);
  }
  return dbInstance;
}

/**
 * Execute a SQL query and return results
 */
export async function query<T = Record<string, unknown>>(
  sql: string,
  params?: unknown[]
): Promise<T[]> {
  const db = await getDatabase();
  const result = await db.executeSql(sql, params);
  return (result.rows ?? []) as T[];
}

/**
 * Execute a SQL query and return first result
 */
export async function queryOne<T = Record<string, unknown>>(
  sql: string,
  params?: unknown[]
): Promise<T | null> {
  const rows = await query<T>(sql, params);
  return rows[0] ?? null;
}

/**
 * Get a signed URL for an asset (for display or download)
 *
 * @param nodePath - Full node path (e.g., '/launchpad/files/image.jpg')
 * @param command - 'display' for viewing, 'download' for downloading
 * @param options - Optional settings: propertyPath (e.g., 'thumbnail' to get thumbnail instead of main file)
 * @returns Signed URL object with url property
 */
export async function signAssetUrl(
  nodePath: string,
  command: 'display' | 'download' = 'display',
  options?: { propertyPath?: string }
): Promise<{ url: string }> {
  const db = await getDatabase();
  const ws = db.workspace(WORKSPACE_NAME);
  return ws.signAssetUrl(nodePath, command, options);
}

/**
 * Reorder a node relative to another sibling node
 *
 * @param sourcePath - Full path of the node to move
 * @param targetPath - Full path of the target sibling node
 * @param position - 'above' to place before target, 'below' to place after target
 */
export async function reorderNode(
  sourcePath: string,
  targetPath: string,
  position: 'above' | 'below'
): Promise<void> {
  const db = await getDatabase();
  const positionKeyword = position === 'above' ? 'ABOVE' : 'BELOW';
  await db.executeSql(
    `ORDER ${WORKSPACE_NAME} SET path=$1 ${positionKeyword} path=$2`,
    [sourcePath, targetPath]
  );
}

/**
 * Move a node (and all its descendants) into a target folder
 *
 * Uses the MOVE SQL statement which properly relocates the entire tree,
 * preserving node IDs and updating all child paths automatically.
 *
 * @param sourcePath - Full path of the node to move
 * @param targetFolderPath - Full path of the destination folder
 */
export async function moveNode(
  sourcePath: string,
  targetFolderPath: string
): Promise<void> {
  const db = await getDatabase();
  await db.executeSql(
    `MOVE ${WORKSPACE_NAME} SET path=$1 TO path=$2`,
    [sourcePath, targetFolderPath]
  );
}

/**
 * Update asset properties (description, alt_text, keywords, etc.)
 *
 * Uses JSONB merge to preserve existing properties while updating specified ones.
 *
 * @param nodePath - Full path of the asset node
 * @param properties - Properties to update (merged with existing)
 */
export async function updateAsset(
  nodePath: string,
  properties: Record<string, unknown>
): Promise<void> {
  const db = await getDatabase();
  await db.executeSql(
    `UPDATE ${WORKSPACE_NAME} SET properties = properties || $2::jsonb WHERE path = $1`,
    [nodePath, JSON.stringify(properties)]
  );
}

/**
 * Fetch a page by its path using SQL
 */
export async function getPageByPath(path: string): Promise<PageNode | null> {
  const normalizedPath = path.startsWith('/') ? path.slice(1) : path;
  const nodePath = normalizedPath ? `/${WORKSPACE_NAME}/${normalizedPath}` : `/${WORKSPACE_NAME}`;

  const sql = `
    SELECT id, path, name, node_type, archetype, properties
    FROM ${WORKSPACE_NAME}
    WHERE path = $1
      ${localeClause()}
    LIMIT 1
  `;

  return queryOne<PageNode>(sql, [nodePath]);
}

/**
 * Fetch navigation items (top-level pages) using SQL
 */
export async function getNavigation(): Promise<NavItem[]> {
  const sql = `
    SELECT id, path, name, node_type, properties
    FROM ${WORKSPACE_NAME}
    WHERE CHILD_OF('/${WORKSPACE_NAME}')
      AND node_type = 'launchpad:Page'
      ${localeClause()}
  `;

  try {
    return await query<NavItem>(sql);
  } catch (error) {
    console.error('[raisin] getNavigation error:', error);
    return [];
  }
}

// Types
export interface PageNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  archetype?: string;
  properties: {
    title: string;
    slug?: string;
    description?: string;
    order?: number;
    content?: Element[];
  };
}

export interface NavItem {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    title: string;
    slug?: string;
    order?: number;
  };
}

export interface Element {
  uuid: string;
  element_type: string;
  // Flat format: element fields are at the root level (no content wrapper)
  [key: string]: unknown;
}

/**
 * Get the HTTP base URL from the RaisinDB client.
 * Useful for components that need to make direct HTTP calls (e.g., AI chat flows).
 */
export function getHttpBaseUrl(): string {
  return getClient().httpBaseUrl;
}

// Re-export types for convenience
export { ConnectionState };
export type { IdentityUser, AuthStateChange, UserChangeEvent };
