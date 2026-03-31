import type { RaisinClient } from '../../client';
import { ConnectionState } from '../../connection';

export interface ConnectionSnapshot {
  state: ConnectionState;
  isConnected: boolean;
  isReady: boolean;
}

export interface ConnectionAdapter {
  subscribe: (cb: (snapshot: ConnectionSnapshot) => void) => () => void;
  getSnapshot: () => ConnectionSnapshot;
  connect: () => Promise<void>;
  disconnect: () => void;
  destroy: () => void;
}

/**
 * Create a connection adapter for Svelte 5.
 *
 * Tracks WebSocket connection state and ready state. Bind to `$state` in
 * a `.svelte.ts` file for reactivity.
 *
 * @example
 * ```typescript
 * // lib/connection.svelte.ts
 * import { createConnectionAdapter } from '@raisindb/client/svelte';
 * import { client } from '$lib/raisin';
 *
 * const adapter = createConnectionAdapter(client);
 * let snapshot = $state(adapter.getSnapshot());
 * adapter.subscribe(s => { snapshot = s; });
 *
 * export const connection = {
 *   get state() { return snapshot.state; },
 *   get isConnected() { return snapshot.isConnected; },
 *   get isReady() { return snapshot.isReady; },
 *   connect: adapter.connect,
 *   disconnect: adapter.disconnect,
 * };
 * ```
 */
export function createConnectionAdapter(client: RaisinClient): ConnectionAdapter {
  let snapshot: ConnectionSnapshot = {
    state: client.getConnectionState(),
    isConnected: client.getConnectionState() === ConnectionState.Connected,
    isReady: client.isReady(),
  };

  const listeners = new Set<(s: ConnectionSnapshot) => void>();

  function emit() {
    for (const cb of listeners) cb(snapshot);
  }

  const unsubConn = client.onConnectionStateChange((newState) => {
    snapshot = {
      state: newState,
      isConnected: newState === ConnectionState.Connected,
      isReady: snapshot.isReady,
    };
    emit();
  });

  const unsubReady = client.onReadyStateChange((ready) => {
    snapshot = { ...snapshot, isReady: ready };
    emit();
  });

  return {
    subscribe(cb: (s: ConnectionSnapshot) => void) {
      listeners.add(cb);
      return () => { listeners.delete(cb); };
    },

    getSnapshot: () => snapshot,

    async connect() {
      await client.connect();
    },

    disconnect() {
      client.disconnect();
    },

    destroy() {
      unsubConn();
      unsubReady();
      listeners.clear();
    },
  };
}
