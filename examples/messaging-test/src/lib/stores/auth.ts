/**
 * Svelte store for authentication state.
 */
import { writable, derived } from 'svelte/store';
import { browser } from '$app/environment';
import {
  initSession,
  login as raisinLogin,
  register as raisinRegister,
  logout as raisinLogout,
  getUser,
  onAuthStateChange,
  onUserChange,
  type IdentityUser,
  type AuthStateChange,
  type UserChangeEvent,
} from '$lib/raisin';

interface AuthState {
  user: IdentityUser | null;
  loading: boolean;
  initialized: boolean;
}

function createAuthStore() {
  const { subscribe, set, update } = writable<AuthState>({
    user: null,
    loading: false,
    initialized: false,
  });

  let authUnsubscribe: (() => void) | null = null;
  let userChangeUnsubscribe: (() => void) | null = null;

  function setupListeners() {
    if (!browser) return;

    authUnsubscribe = onAuthStateChange(({ event, session }: AuthStateChange) => {
      switch (event) {
        case 'SIGNED_IN':
          set({ user: session.user, loading: false, initialized: true });
          break;
        case 'SIGNED_OUT':
          set({ user: null, loading: false, initialized: true });
          break;
        case 'TOKEN_REFRESHED':
        case 'USER_UPDATED':
          update((s) => ({ ...s, user: session.user }));
          break;
        case 'SESSION_EXPIRED':
          set({ user: null, loading: false, initialized: true });
          break;
      }
    });

    userChangeUnsubscribe = onUserChange((_event: UserChangeEvent) => {
      // The SDK emits USER_UPDATED via onAuthStateChange
    });
  }

  function cleanup() {
    authUnsubscribe?.();
    userChangeUnsubscribe?.();
    authUnsubscribe = null;
    userChangeUnsubscribe = null;
  }

  return {
    subscribe,

    setUser(user: IdentityUser | null) {
      set({
        user,
        loading: false,
        initialized: true,
      });
    },

    async init() {
      set({ user: null, loading: true, initialized: false });
      setupListeners();

      try {
        const user = await initSession();
        set({
          user,
          loading: false,
          initialized: true,
        });
      } catch (err) {
        console.error('[auth] Init error:', err);
        set({ user: null, loading: false, initialized: true });
      }
    },

    async login(
      email: string,
      password: string
    ): Promise<{ success: true } | { success: false; error: { code: string; message: string } }> {
      update((s) => ({ ...s, loading: true }));

      try {
        await raisinLogin(email, password);
        return { success: true };
      } catch (err: unknown) {
        update((s) => ({ ...s, loading: false }));
        const error = err as { code?: string; message?: string };
        return {
          success: false,
          error: {
            code: error.code || 'LOGIN_FAILED',
            message: error.message || 'Login failed',
          },
        };
      }
    },

    async register(
      email: string,
      password: string,
      displayName?: string
    ): Promise<{ success: true } | { success: false; error: { code: string; message: string } }> {
      update((s) => ({ ...s, loading: true }));

      try {
        await raisinRegister(email, password, displayName);
        return { success: true };
      } catch (err: unknown) {
        update((s) => ({ ...s, loading: false }));
        const error = err as { code?: string; message?: string };
        return {
          success: false,
          error: {
            code: error.code || 'REGISTRATION_FAILED',
            message: error.message || 'Registration failed',
          },
        };
      }
    },

    async logout() {
      await raisinLogout();
    },

    getStoredUser(): IdentityUser | null {
      return getUser();
    },

    cleanup,
  };
}

export const auth = createAuthStore();
export const user = derived(auth, ($auth) => $auth.user);
export const isAuthenticated = derived(auth, ($auth) => $auth.user !== null);
export const isLoading = derived(auth, ($auth) => $auth.loading);
export const isInitialized = derived(auth, ($auth) => $auth.initialized);
