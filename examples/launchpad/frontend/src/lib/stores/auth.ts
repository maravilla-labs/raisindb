/**
 * Svelte store for authentication state.
 *
 * Uses the SDK's onAuthStateChange for reactive updates (Firebase/Supabase pattern).
 * The actual auth logic is handled by the raisin-client-js SDK.
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

// Auth state store
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

  // Track unsubscribe functions
  let authUnsubscribe: (() => void) | null = null;
  let userChangeUnsubscribe: (() => void) | null = null;

  /**
   * Setup reactive auth state listeners (call once on app init)
   */
  function setupListeners() {
    if (!browser) return;

    // Subscribe to auth state changes from SDK
    authUnsubscribe = onAuthStateChange(({ event, session }: AuthStateChange) => {
      switch (event) {
        case 'SIGNED_IN':
          set({ user: session.user, loading: false, initialized: true });
          break;
        case 'SIGNED_OUT':
          set({ user: null, loading: false, initialized: true });
          break;
        case 'TOKEN_REFRESHED':
          // Token refreshed, user stays the same
          update((s) => ({ ...s, user: session.user }));
          break;
        case 'USER_UPDATED':
          // User profile changed (from userHome subscription)
          update((s) => ({ ...s, user: session.user }));
          break;
        case 'SESSION_EXPIRED':
          set({ user: null, loading: false, initialized: true });
          break;
      }
    });

    // Subscribe to user home node changes
    userChangeUnsubscribe = onUserChange((_event: UserChangeEvent) => {
      // The SDK emits USER_UPDATED via onAuthStateChange
    });
  }

  /**
   * Cleanup listeners (call on app destroy)
   */
  function cleanup() {
    authUnsubscribe?.();
    userChangeUnsubscribe?.();
    authUnsubscribe = null;
    userChangeUnsubscribe = null;
  }

  return {
    subscribe,

    /**
     * Set user from layout load data (initial load)
     */
    setUser(user: IdentityUser | null) {
      set({
        user,
        loading: false,
        initialized: true,
      });
    },

    /**
     * Initialize auth state - restores session and sets up listeners
     */
    async init() {
      set({ user: null, loading: true, initialized: false });

      // Setup reactive listeners
      setupListeners();

      try {
        // initSession will emit SIGNED_IN event if session restored
        const user = await initSession();
        // Set state in case no event was emitted (anonymous user)
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

    /**
     * Login with email and password
     * The SDK will emit SIGNED_IN event which updates the store reactively
     */
    async login(
      email: string,
      password: string
    ): Promise<{ success: true } | { success: false; error: { code: string; message: string } }> {
      update((s) => ({ ...s, loading: true }));

      try {
        // loginWithEmail emits SIGNED_IN which triggers our listener
        await raisinLogin(email, password);
        // State is updated by onAuthStateChange listener
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

    /**
     * Register a new user
     * The SDK will emit SIGNED_IN event which updates the store reactively
     */
    async register(
      email: string,
      password: string,
      displayName?: string
    ): Promise<{ success: true } | { success: false; error: { code: string; message: string } }> {
      update((s) => ({ ...s, loading: true }));

      try {
        // registerWithEmail emits SIGNED_IN which triggers our listener
        await raisinRegister(email, password, displayName);
        // State is updated by onAuthStateChange listener
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

    /**
     * Logout
     * The SDK will emit SIGNED_OUT event which updates the store reactively
     */
    async logout() {
      // logout emits SIGNED_OUT which triggers our listener
      await raisinLogout();
      // State is updated by onAuthStateChange listener
    },

    /**
     * Get stored user (sync, from client)
     */
    getStoredUser(): IdentityUser | null {
      return getUser();
    },

    /**
     * Cleanup listeners (for SSR/unmount)
     */
    cleanup,
  };
}

export const auth = createAuthStore();

// Derived stores for convenience
export const user = derived(auth, ($auth) => $auth.user);
export const isAuthenticated = derived(auth, ($auth) => $auth.user !== null);
export const isLoading = derived(auth, ($auth) => $auth.loading);
export const isInitialized = derived(auth, ($auth) => $auth.initialized);
