import type { VueLike, VueRef, UseAuthReturn } from './types';
import type { RaisinClient } from '../../client';
// Framework-free snapshot adapter shared with the Svelte integration.
import { createAuthAdapter, type AuthSnapshot } from '../svelte/svelte-auth';

export function createUseAuth(vue: VueLike): (client: RaisinClient) => UseAuthReturn {
  return function useAuth(client: RaisinClient): UseAuthReturn {
    const adapter = createAuthAdapter(client);
    const snapshot = vue.ref(adapter.getSnapshot()) as VueRef<AuthSnapshot>;
    const unsubscribe = adapter.subscribe((s) => { snapshot.value = s; });

    vue.onUnmounted(() => {
      unsubscribe();
      adapter.destroy();
    });

    return {
      user: vue.computed(() => snapshot.value.user),
      isAuthenticated: vue.computed(() => snapshot.value.isAuthenticated),
      isLoading: vue.computed(() => snapshot.value.isLoading),
      login: adapter.login,
      register: adapter.register,
      logout: adapter.logout,
      initSession: adapter.initSession,
    };
  };
}
