/**
 * Connection status (Svelte 5 runes) for the header dot.
 *
 * Uses the SDK's `createConnectionAdapter`, which wraps
 * client.onConnectionStateChange() + client.onReadyStateChange()
 * into a single snapshot-subscribe object.
 *
 * Lazy: the adapter is attached in init() after the WS client exists
 * (this module is also imported during SSR, where there is no client).
 */
import {
  createConnectionAdapter,
  ConnectionState,
  type ConnectionSnapshot,
} from '@raisindb/client';
import { getClient } from '../raisin';

class ConnectionStore {
  snapshot = $state<ConnectionSnapshot>({
    state: ConnectionState.Disconnected,
    isConnected: false,
    isReady: false,
  });

  #initialized = false;

  /** Attach to the WS client. Call once after initClient(). */
  init(): void {
    if (this.#initialized) return;
    this.#initialized = true;
    const adapter = createConnectionAdapter(getClient());
    this.snapshot = adapter.getSnapshot();
    adapter.subscribe((s) => {
      this.snapshot = s;
    });
  }

  /** 'connected' | 'reconnecting' | 'disconnected' for the status dot. */
  get status(): 'connected' | 'reconnecting' | 'disconnected' {
    switch (this.snapshot.state) {
      case ConnectionState.Connected:
        return 'connected';
      case ConnectionState.Connecting:
      case ConnectionState.Reconnecting:
        return 'reconnecting';
      default:
        return 'disconnected';
    }
  }
}

export const connection = new ConnectionStore();
