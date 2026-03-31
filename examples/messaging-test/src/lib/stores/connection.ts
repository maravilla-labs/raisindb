/**
 * Svelte store for WebSocket connection state.
 */
import { writable, derived } from 'svelte/store';
import { browser } from '$app/environment';
import { onConnectionStateChange, getConnectionState, onReconnected, ConnectionState } from '$lib/raisin';

interface ConnectionStoreState {
  state: ConnectionState;
  initialized: boolean;
}

function createConnectionStore() {
  const { subscribe, set, update } = writable<ConnectionStoreState>({
    state: ConnectionState.Disconnected,
    initialized: false,
  });

  let connectionUnsubscribe: (() => void) | null = null;
  let reconnectedUnsubscribe: (() => void) | null = null;

  return {
    subscribe,

    init() {
      if (!browser) return;

      // Get initial state
      set({
        state: getConnectionState(),
        initialized: true,
      });

      // Subscribe to connection changes
      connectionUnsubscribe = onConnectionStateChange((state) => {
        update((s) => ({ ...s, state }));
      });

      // Subscribe to reconnection events
      reconnectedUnsubscribe = onReconnected(() => {
        console.log('[connection] Reconnected');
      });
    },

    cleanup() {
      connectionUnsubscribe?.();
      reconnectedUnsubscribe?.();
      connectionUnsubscribe = null;
      reconnectedUnsubscribe = null;
    },
  };
}

export const connection = createConnectionStore();

export const connectionState = derived(connection, ($c) => $c.state);
export const connected = derived(connection, ($c) => $c.state === ConnectionState.Connected);
