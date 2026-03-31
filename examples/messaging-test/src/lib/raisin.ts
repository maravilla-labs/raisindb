/**
 * RaisinDB client for messaging test frontend.
 *
 * Connects to the xxx3 workspace for testing the messaging system.
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

// Configuration
const TENANT_ID = 'default';
const REPOSITORY = 't3';
const RAISIN_URL = `ws://localhost:8081/sys/${TENANT_ID}/${REPOSITORY}`;
export const ACCESS_CONTROL_WORKSPACE = 'raisin:access_control';

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
      tokenStorage: new LocalStorageTokenStorage('messaging-test'),
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
 */
export async function initSession(): Promise<IdentityUser | null> {
  // Create connection gate
  if (!connectionPromise) {
    connectionPromise = new Promise((resolve) => {
      connectionResolve = resolve;
    });
  }

  const client = getClient();

  try {
    const user = await client.initSession(REPOSITORY);
    if (!user) {
      if (!client.isConnected()) {
        await client.connect();
      }
    }

    if (connectionResolve) {
      connectionResolve();
    }

    return user;
  } catch (error) {
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
 * Logout
 */
export async function logout(): Promise<void> {
  const client = getClient();
  await client.logout();
  dbInstance = null;
}

/**
 * Get the current authenticated user
 */
export function getUser(): IdentityUser | null {
  const client = getClient();
  return client.getUser();
}

/**
 * Subscribe to auth state changes
 */
export function onAuthStateChange(callback: (change: AuthStateChange) => void): () => void {
  const client = getClient();
  return client.onAuthStateChange(callback);
}

/**
 * Subscribe to user changes
 */
export function onUserChange(callback: (event: UserChangeEvent) => void): () => void {
  const client = getClient();
  return client.onUserChange(callback);
}

/**
 * Subscribe to connection state changes
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
 * Subscribe to reconnection events
 */
export function onReconnected(callback: () => void): () => void {
  const client = getClient();
  return client.onReconnected(callback);
}

/**
 * Get the database instance for SQL queries
 */
export async function getDatabase(): Promise<Database> {
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
 * Subscribe to node events for a specific path pattern
 */
export async function subscribeToPath(
  pathPattern: string,
  callback: (event: { kind: string; node: Record<string, unknown> }) => void
): Promise<() => void> {
  const db = await getDatabase();
  const ws = db.workspace(ACCESS_CONTROL_WORKSPACE);
  return ws.subscribeToDescendants(pathPattern, callback);
}

// Types
export interface MessageNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    message_type: string;
    status: string;
    subject?: string;
    sender_id?: string;
    recipient_id?: string;
    sender_display_name?: string;
    body?: Record<string, unknown>;
    created_at?: string;
    [key: string]: unknown;
  };
}

export interface UserNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    email?: string;
    display_name?: string;
    [key: string]: unknown;
  };
}

// Re-export types
export { ConnectionState };
export type { IdentityUser, AuthStateChange, UserChangeEvent };
