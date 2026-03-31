/**
 * Svelte store for connection ready state.
 *
 * Uses the client-js ready state which tracks:
 * - WebSocket is connected AND
 * - User is authenticated (or no stored token = anonymous is fine)
 *
 * This is the state the green connection dot should show.
 */
import { writable, derived } from 'svelte/store';
import { browser } from '$app/environment';
import {
  onReadyStateChange,
  onConnectionStateChange,
  isReady as checkReady,
  getConnectionState,
  ConnectionState,
} from '$lib/raisin';

// Connection state store
interface ConnectionStoreState {
  /** WebSocket connection state */
  wsState: ConnectionState;
  /** Fully ready: connected AND authenticated (from client-js) */
  ready: boolean;
  /** Store has been initialized */
  initialized: boolean;
}

function createConnectionStore() {
  const { subscribe, set, update } = writable<ConnectionStoreState>({
    wsState: ConnectionState.Disconnected,
    ready: false,
    initialized: false,
  });

  let unsubscribeReady: (() => void) | null = null;
  let unsubscribeConnection: (() => void) | null = null;

  return {
    subscribe,

    /**
     * Initialize connection state tracking
     * Call this after initSession() to start tracking connection state
     */
    init() {
      if (!browser) return;

      // Get initial state
      const currentState = getConnectionState();
      const ready = checkReady();

      set({
        wsState: currentState,
        ready,
        initialized: true,
      });

      // Subscribe to ready state changes from client-js
      // This is the authoritative source for "green dot" state
      unsubscribeReady = onReadyStateChange((ready) => {
        update((s) => ({ ...s, ready }));
      });

      // Subscribe to raw connection state
      unsubscribeConnection = onConnectionStateChange((state) => {
        update((s) => ({ ...s, wsState: state }));
      });
    },

    /**
     * Cleanup subscriptions
     */
    cleanup() {
      unsubscribeReady?.();
      unsubscribeReady = null;
      unsubscribeConnection?.();
      unsubscribeConnection = null;
    },
  };
}

export const connection = createConnectionStore();

// Derived stores for convenience
/** Fully ready: WebSocket connected AND authenticated (use for green dot) */
export const connected = derived(connection, ($conn) => $conn.ready);
/** Raw WebSocket connection state */
export const connectionState = derived(connection, ($conn) => $conn.wsState);

// Re-export ConnectionState for convenience
export { ConnectionState };
