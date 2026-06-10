import type { VueLike, VueRef, UseConnectionReturn } from './types';
import type { RaisinClient } from '../../client';
// Framework-free snapshot adapter shared with the Svelte integration.
import { createConnectionAdapter, type ConnectionSnapshot } from '../svelte/svelte-connection';

export function createUseConnection(vue: VueLike): (client: RaisinClient) => UseConnectionReturn {
  return function useConnection(client: RaisinClient): UseConnectionReturn {
    const adapter = createConnectionAdapter(client);
    const snapshot = vue.ref(adapter.getSnapshot()) as VueRef<ConnectionSnapshot>;
    const unsubscribe = adapter.subscribe((s) => { snapshot.value = s; });

    vue.onUnmounted(() => {
      unsubscribe();
      adapter.destroy();
    });

    return {
      state: vue.computed(() => snapshot.value.state),
      isConnected: vue.computed(() => snapshot.value.isConnected),
      isReady: vue.computed(() => snapshot.value.isReady),
      connect: adapter.connect,
      disconnect: adapter.disconnect,
    };
  };
}
