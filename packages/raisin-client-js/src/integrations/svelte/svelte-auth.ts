import type { RaisinClient } from '../../client';
import type { IdentityUser } from '../../auth';

export interface AuthSnapshot {
  user: IdentityUser | null;
  isAuthenticated: boolean;
  isLoading: boolean;
}

export interface AuthAdapter {
  subscribe: (cb: (snapshot: AuthSnapshot) => void) => () => void;
  getSnapshot: () => AuthSnapshot;
  login: (email: string, password: string, repository: string) => Promise<IdentityUser>;
  register: (email: string, password: string, repository: string, displayName?: string) => Promise<IdentityUser>;
  logout: (options?: { disconnect?: boolean; reconnect?: boolean }) => Promise<void>;
  initSession: (repository: string) => Promise<IdentityUser | null>;
  destroy: () => void;
}

/**
 * Create an auth adapter for Svelte 5.
 *
 * Returns a plain object with `subscribe`, `getSnapshot`, auth actions, and `destroy`.
 * Bind `getSnapshot()` to `$state` in a `.svelte.ts` file for reactivity.
 *
 * @example
 * ```typescript
 * // lib/auth.svelte.ts
 * import { createAuthAdapter } from '@raisindb/client/svelte';
 * import { client } from '$lib/raisin';
 *
 * const adapter = createAuthAdapter(client);
 * let snapshot = $state(adapter.getSnapshot());
 * adapter.subscribe(s => { snapshot = s; });
 *
 * export const auth = {
 *   get user() { return snapshot.user; },
 *   get isAuthenticated() { return snapshot.isAuthenticated; },
 *   get isLoading() { return snapshot.isLoading; },
 *   login: adapter.login,
 *   register: adapter.register,
 *   logout: adapter.logout,
 *   initSession: adapter.initSession,
 * };
 * ```
 */
export function createAuthAdapter(client: RaisinClient): AuthAdapter {
  let snapshot: AuthSnapshot = {
    user: client.getStoredUser(),
    isAuthenticated: client.getStoredUser() !== null,
    isLoading: false,
  };

  const listeners = new Set<(s: AuthSnapshot) => void>();

  function emit() {
    for (const cb of listeners) cb(snapshot);
  }

  function update(partial: Partial<AuthSnapshot>) {
    snapshot = { ...snapshot, ...partial };
    emit();
  }

  const unsubAuth = client.onAuthStateChange(({ event, session }) => {
    if (event === 'SIGNED_IN' || event === 'TOKEN_REFRESHED' || event === 'USER_UPDATED') {
      update({ user: session.user, isAuthenticated: true });
    } else if (event === 'SIGNED_OUT' || event === 'SESSION_EXPIRED') {
      update({ user: null, isAuthenticated: false });
    }
  });

  const unsubUser = client.onUserChange(({ node }) => {
    if (!snapshot.user) return;
    update({
      user: {
        ...snapshot.user,
        home: node.path,
        displayName: (node.properties.display_name as string) ?? snapshot.user.displayName,
        avatarUrl: (node.properties.avatar_url as string) ?? snapshot.user.avatarUrl,
      },
    });
  });

  return {
    subscribe(cb: (s: AuthSnapshot) => void) {
      listeners.add(cb);
      return () => { listeners.delete(cb); };
    },

    getSnapshot: () => snapshot,

    async login(email: string, password: string, repository: string) {
      update({ isLoading: true });
      try {
        return await client.loginWithEmail(email, password, repository);
      } finally {
        update({ isLoading: false });
      }
    },

    async register(email: string, password: string, repository: string, displayName?: string) {
      update({ isLoading: true });
      try {
        return await client.registerWithEmail(email, password, repository, displayName);
      } finally {
        update({ isLoading: false });
      }
    },

    async logout(options?: { disconnect?: boolean; reconnect?: boolean }) {
      update({ isLoading: true });
      try {
        await client.logout(options);
      } finally {
        update({ isLoading: false });
      }
    },

    async initSession(repository: string) {
      update({ isLoading: true });
      try {
        return await client.initSession(repository);
      } finally {
        update({ isLoading: false });
      }
    },

    destroy() {
      unsubAuth();
      unsubUser();
      listeners.clear();
    },
  };
}
