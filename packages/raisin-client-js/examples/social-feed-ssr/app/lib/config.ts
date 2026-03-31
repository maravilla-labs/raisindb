/**
 * Configuration for RaisinDB client
 *
 * This file provides configuration for both SSR (HTTP) and client-side (WebSocket) modes.
 */

import { SSRClientConfig } from '@raisindb/client';

/**
 * Get RaisinDB configuration
 *
 * On the server: Uses HTTP API for SSR data fetching
 * On the client: Uses WebSocket for real-time updates
 */
export function getRaisinConfig(): SSRClientConfig {
  // These can be overridden by environment variables
  const httpBaseUrl = typeof window === 'undefined'
    ? (process.env.RAISIN_HTTP_URL || 'http://localhost:8080')
    : (window.ENV?.RAISIN_HTTP_URL || 'http://localhost:8080');

  const wsUrl = typeof window === 'undefined'
    ? (process.env.RAISIN_WS_URL || 'ws://localhost:8080/sys/default')
    : (window.ENV?.RAISIN_WS_URL || 'ws://localhost:8080/sys/default');

  return {
    httpBaseUrl,
    wsUrl,
    httpOptions: {
      tenantId: 'default',
      defaultBranch: 'main',
    },
    wsOptions: {
      tenantId: 'default',
      defaultBranch: 'main',
    },
    credentials: {
      username: 'admin',
      password: 'kXTrED%L&N2N*YZ#',
    },
  };
}

/**
 * Repository and workspace names
 */
export const REPOSITORY = 'social_feed_demo';
export const WORKSPACE = 'social';

/**
 * Type augmentation for window object to include environment variables
 */
declare global {
  interface Window {
    ENV?: {
      RAISIN_HTTP_URL?: string;
      RAISIN_WS_URL?: string;
    };
  }
}
