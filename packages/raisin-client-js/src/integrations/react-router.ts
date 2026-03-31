/**
 * React Router 7 integration for RaisinDB
 *
 * Provides helpers for server-side rendering with React Router 7, including:
 * - Loader utilities for data fetching
 * - Client context management
 * - Hybrid mode support (HTTP on server, WebSocket on client)
 */

import { RaisinHttpClient, HttpClientOptions } from '../http-client';
import { RaisinClient, ClientOptions } from '../client';
import { Credentials } from '../auth';
import { logger } from '../logger';

/**
 * Configuration for SSR client factory
 */
export interface SSRClientConfig {
  /** Base URL for HTTP API (e.g., "http://localhost:8080") */
  httpBaseUrl: string;
  /** Base URL for WebSocket (e.g., "ws://localhost:8080/sys/default") */
  wsUrl: string;
  /** HTTP client options */
  httpOptions?: HttpClientOptions;
  /** WebSocket client options */
  wsOptions?: ClientOptions;
  /** Default credentials for authentication (optional) */
  credentials?: Credentials;
}

/**
 * Create an SSR-compatible client for React Router loaders
 *
 * This creates an HTTP client for server-side data fetching.
 * Use in loader functions to fetch data during SSR.
 *
 * @param config - SSR client configuration
 * @returns HTTP client instance
 *
 * @example
 * ```typescript
 * // In a React Router loader
 * export async function loader() {
 *   const client = createSSRClient({
 *     httpBaseUrl: 'http://localhost:8080',
 *     wsUrl: 'ws://localhost:8080/sys/default',
 *     credentials: { username: 'admin', password: 'admin' }
 *   });
 *
 *   await client.authenticate(config.credentials);
 *
 *   const posts = await client.database('social')
 *     .executeSql('SELECT * FROM social WHERE node_type = "Post"');
 *
 *   return { posts };
 * }
 * ```
 */
export function createSSRClient(config: SSRClientConfig): RaisinHttpClient {
  const client = RaisinClient.forSSR(config.httpBaseUrl, config.httpOptions);

  // Auto-authenticate if credentials provided
  if (config.credentials) {
    // Note: This is async, but we return the client immediately
    // Callers should await client.authenticate() if needed
    // Or use the credentials in their loader
  }

  return client;
}

/**
 * Create a loader helper that provides an authenticated client
 *
 * This wraps your loader function and provides an authenticated HTTP client.
 *
 * @param config - SSR client configuration
 * @param loaderFn - Your loader function that receives the authenticated client
 * @returns React Router loader function
 *
 * @example
 * ```typescript
 * export const loader = createLoader(
 *   {
 *     httpBaseUrl: process.env.RAISIN_HTTP_URL || 'http://localhost:8080',
 *     wsUrl: process.env.RAISIN_WS_URL || 'ws://localhost:8080/sys/default',
 *     credentials: { username: 'admin', password: 'admin' }
 *   },
 *   async (client, { request, params }) => {
 *     const db = client.database('social');
 *     const posts = await db.executeSql('SELECT * FROM social WHERE node_type = "Post"');
 *     return { posts: posts.rows };
 *   }
 * );
 * ```
 */
export function createLoader<TData = unknown>(
  config: SSRClientConfig,
  loaderFn: (
    client: RaisinHttpClient,
    args: { request: Request; params: Record<string, string | undefined> }
  ) => Promise<TData>
): (args: { request: Request; params: Record<string, string | undefined> }) => Promise<TData> {
  return async (args) => {
    const client = createSSRClient(config);

    // Authenticate if credentials provided
    if (config.credentials) {
      await client.authenticate(config.credentials);
    }

    try {
      return await loaderFn(client, args);
    } catch (error) {
      logger.error('SSR Loader error:', error);
      throw error;
    }
  };
}

/**
 * Create a request-scoped client for SSR
 *
 * This ensures each SSR request gets its own client instance,
 * preventing state leakage between requests.
 *
 * @param httpBaseUrl - Base HTTP URL
 * @param options - HTTP client options
 * @returns New HTTP client instance
 */
export function createRequestScopedClient(
  httpBaseUrl: string,
  options?: HttpClientOptions
): RaisinHttpClient {
  return new RaisinHttpClient(httpBaseUrl, options);
}

/**
 * Helper to convert SQL result rows to objects
 *
 * @param columns - Column names from SQL result
 * @param rows - Row data from SQL result
 * @returns Array of objects with column names as keys
 *
 * @example
 * ```typescript
 * const result = await client.database('social')
 *   .executeSql('SELECT id, title FROM posts');
 * const posts = rowsToObjects(result.columns, result.rows);
 * // [{ id: '1', title: 'First Post' }, ...]
 * ```
 */
export function rowsToObjects<T = Record<string, unknown>>(
  _columns: string[],
  rows: Record<string, unknown>[]
): T[] {
  return rows as T[];
}

/**
 * Extract error message from various error types
 *
 * @param error - Error object
 * @returns Error message string
 */
export function getErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === 'string') {
    return error;
  }
  if (error && typeof error === 'object' && 'message' in error) {
    return String((error as any).message);
  }
  return 'An unknown error occurred';
}

/**
 * Type for hybrid client that can be either HTTP or WebSocket
 */
export type HybridClient = RaisinHttpClient | RaisinClient;

/**
 * Check if client is HTTP-based
 */
export function isHttpClient(client: HybridClient): client is RaisinHttpClient {
  return client instanceof RaisinHttpClient;
}

/**
 * Check if client is WebSocket-based
 */
export function isWebSocketClient(client: HybridClient): client is RaisinClient {
  return client instanceof RaisinClient;
}
