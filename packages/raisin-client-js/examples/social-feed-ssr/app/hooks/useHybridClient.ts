/**
 * useHybridClient hook
 *
 * Manages the transition from HTTP client (SSR) to WebSocket client (real-time updates).
 *
 * Usage:
 * 1. During SSR, data is fetched via HTTP in loaders
 * 2. After hydration, this hook establishes a WebSocket connection
 * 3. Components can then subscribe to real-time updates
 */

import { useState, useEffect, useRef } from 'react';
import { RaisinClient, RaisinHttpClient, ConnectionState } from '@raisindb/client';
import { getRaisinConfig } from '~/lib/config';

export type ClientMode = 'http' | 'websocket' | 'connecting' | 'error';

export interface HybridClientState {
  /** Current client mode */
  mode: ClientMode;
  /** HTTP client (always available) */
  httpClient: RaisinHttpClient;
  /** WebSocket client (available after upgrade) */
  wsClient: RaisinClient | null;
  /** Connection state */
  connectionState: ConnectionState;
  /** Whether client is ready for real-time updates */
  isRealtime: boolean;
  /** Error message if connection failed */
  error: string | null;
  /** Manually trigger reconnection */
  reconnect: () => void;
}

/**
 * Hook to manage hybrid client (HTTP + WebSocket)
 *
 * @param autoUpgrade - Whether to automatically upgrade to WebSocket after mount (default: true)
 * @returns Hybrid client state
 *
 * @example
 * ```tsx
 * function MyComponent() {
 *   const { mode, wsClient, isRealtime } = useHybridClient();
 *
 *   useEffect(() => {
 *     if (isRealtime && wsClient) {
 *       // Subscribe to real-time updates
 *       const subscription = wsClient.database('social')
 *         .events()
 *         .subscribe(
 *           { workspace: 'default', node_type: 'Post' },
 *           (event) => {
 *             console.log('Real-time event:', event);
 *           }
 *         );
 *
 *       return () => subscription.unsubscribe();
 *     }
 *   }, [isRealtime, wsClient]);
 *
 *   return <div>Mode: {mode}</div>;
 * }
 * ```
 */
export function useHybridClient(autoUpgrade: boolean = true): HybridClientState {
  const config = getRaisinConfig();

  // HTTP client (always available, created once)
  const [httpClient] = useState(() => RaisinClient.forSSR(config.httpBaseUrl, config.httpOptions));

  // WebSocket client state
  const [wsClient, setWsClient] = useState<RaisinClient | null>(null);
  const [connectionState, setConnectionState] = useState<ConnectionState>(ConnectionState.Disconnected);
  const [error, setError] = useState<string | null>(null);

  // Ref to track if we've already upgraded (prevent double upgrade)
  const hasUpgraded = useRef(false);
  const reconnectAttempts = useRef(0);
  const maxReconnectAttempts = 3;

  // Upgrade to WebSocket
  const upgradeToWebSocket = async () => {
    if (hasUpgraded.current) {
      return;
    }

    hasUpgraded.current = true;
    setError(null);

    try {
      console.log('[useHybridClient] Upgrading to WebSocket...');
      setConnectionState(ConnectionState.Connecting);

      // Create WebSocket client
      const client = new RaisinClient(config.wsUrl, config.wsOptions);

      // Set up connection state listener
      client.on('stateChange', (state: ConnectionState) => {
        console.log('[useHybridClient] Connection state:', state);
        setConnectionState(state);
      });

      // Set up error listener
      client.on('error', (err: Error) => {
        console.error('[useHybridClient] Connection error:', err);
        setError(err.message);
      });

      // Connect
      await client.connect();

      // Authenticate if credentials provided
      if (config.credentials) {
        await client.authenticate(config.credentials);
      }

      console.log('[useHybridClient] WebSocket upgrade complete');
      setWsClient(client);
      reconnectAttempts.current = 0;
    } catch (err) {
      console.error('[useHybridClient] Failed to upgrade to WebSocket:', err);
      setError(err instanceof Error ? err.message : 'Failed to connect');
      setConnectionState(ConnectionState.Closed);
      hasUpgraded.current = false; // Allow retry

      // Auto-retry with exponential backoff
      if (reconnectAttempts.current < maxReconnectAttempts) {
        reconnectAttempts.current++;
        const delay = Math.min(1000 * Math.pow(2, reconnectAttempts.current), 10000);
        console.log(`[useHybridClient] Retrying in ${delay}ms... (attempt ${reconnectAttempts.current}/${maxReconnectAttempts})`);
        setTimeout(upgradeToWebSocket, delay);
      }
    }
  };

  // Reconnect function
  const reconnect = () => {
    console.log('[useHybridClient] Manual reconnect triggered');
    reconnectAttempts.current = 0;
    hasUpgraded.current = false;
    setError(null);

    // Disconnect existing client
    if (wsClient) {
      wsClient.disconnect();
      setWsClient(null);
    }

    upgradeToWebSocket();
  };

  // Auto-upgrade on mount (client-side only)
  useEffect(() => {
    if (autoUpgrade && typeof window !== 'undefined') {
      console.log('[useHybridClient] Auto-upgrading to WebSocket...');
      upgradeToWebSocket();
    }

    // Cleanup on unmount
    return () => {
      if (wsClient) {
        console.log('[useHybridClient] Disconnecting WebSocket on unmount');
        wsClient.disconnect();
      }
    };
  }, [autoUpgrade]);

  // Determine current mode
  let mode: ClientMode = 'http';
  if (error && !wsClient) {
    mode = 'error';
  } else if (connectionState === ConnectionState.Connecting || connectionState === ConnectionState.Reconnecting) {
    mode = 'connecting';
  } else if (wsClient && connectionState === ConnectionState.Connected) {
    mode = 'websocket';
  }

  return {
    mode,
    httpClient,
    wsClient,
    connectionState,
    isRealtime: mode === 'websocket',
    error,
    reconnect,
  };
}
