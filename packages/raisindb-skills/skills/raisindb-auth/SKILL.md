---
name: raisindb-auth
description: "Authentication flows for RaisinDB apps: anonymous access, login, register, session management, auth state listeners. Use when adding authentication to your frontend."
---

# RaisinDB Authentication

RaisinDB authentication runs over WebSocket. The client connects, gets an anonymous session automatically, and can upgrade to an authenticated session via email/password login or registration. Tokens are persisted in localStorage and restored on reload via `initSession()`.

## Session Initialization

Call `initSession()` once at app startup, before any queries. It restores a stored token (if any) and connects the WebSocket. Returns the authenticated user or `null` for anonymous.

```typescript
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';

const client = new RaisinClient('ws://localhost:8080/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myapp'),
  tenantId: 'default',
  defaultBranch: 'main',
  connection: { autoReconnect: true, heartbeatInterval: 30000 },
});

const user = await client.initSession('myrepo');
// user is IdentityUser | null
```

In a SvelteKit app, wrap this in a connection gate so queries wait until the session is ready. Create a promise that resolves when `initSession()` completes, and await it in `getDatabase()`:

```typescript
// lib/raisin.ts -- connection gate pattern
let connectionResolve: (() => void) | null = null;
let connectionPromise = new Promise<void>((r) => { connectionResolve = r; });

export async function initSession(): Promise<IdentityUser | null> {
  try {
    const user = await getClient().initSession(REPOSITORY);
    if (!user && !getClient().isConnected()) await getClient().connect();
    return user;
  } finally { connectionResolve?.(); }
}

export async function getDatabase(): Promise<Database> {
  await connectionPromise; // blocks until initSession finishes
  // ...return db instance
}
```

## Anonymous Access

When `initSession()` finds no stored token, the server issues an anonymous session. `user` is `null` but queries work for public data — **if anonymous access is properly configured**.

**Required setup for anonymous access to work:**

1. **Enable anonymous access** in the admin console: Access Control > Settings > Enable Anonymous Access
2. **Add read permission** to the anonymous role: Access Control > Roles > anonymous > add permission with workspace name, path `/**`, and Read operation checked
3. The anonymous role only has read access to the `launchpad` workspace by default — you MUST add permissions for your own workspaces

If queries return 0 rows with `isAuthenticated: false`, the anonymous role is missing read permissions for your workspace.

## Login

```typescript
const user = await client.loginWithEmail(email, password, 'myrepo');
// Returns IdentityUser, emits SIGNED_IN event
```

The SDK stores the access and refresh tokens automatically when `tokenStorage` is configured.

## Register

```typescript
const user = await client.registerWithEmail(email, password, 'myrepo', 'Display Name');
// Returns IdentityUser, emits SIGNED_IN event. Password min 8 chars.
```

## Logout

```typescript
await client.logout();
// Clears tokens, disconnects, reconnects as anonymous. Emits SIGNED_OUT event.
```

## Auth State Changes

Subscribe to auth events (Firebase/Supabase pattern). Returns an unsubscribe function.

```typescript
const unsubscribe = client.onAuthStateChange(({ event, session }) => {
  // event: 'SIGNED_IN' | 'SIGNED_OUT' | 'TOKEN_REFRESHED' | 'USER_UPDATED' | 'SESSION_EXPIRED'
  // session.user is IdentityUser | null
});
```

Events: `SIGNED_IN` (login/register), `SIGNED_OUT` (logout), `TOKEN_REFRESHED` (silent refresh), `USER_UPDATED` (profile node changed), `SESSION_EXPIRED` (refresh token expired, now anonymous).

## User Object

```typescript
interface IdentityUser {
  id: string;           // unique user ID
  email: string;        // user's email
  display_name: string; // display name
  home: string;         // path in access_control workspace, e.g. '/raisin:access_control/users/abc123'
}
```

The `home` path lets you query the user's inbox, outbox, and profile in the `raisin:access_control` workspace.

## Connection State

Track WebSocket readiness for UI indicators (the "green dot" pattern). Ready = connected AND authenticated (or anonymous with no stored token).

```typescript
const ready = client.isReady();

const unsubReady = client.onReadyStateChange((ready) => {
  ready ? showGreenDot() : showRedDot();
});

const unsubConn = client.onConnectionStateChange((state) => {
  // state: ConnectionState.Connected | Connecting | Disconnected
});
```

## SvelteKit Auth Store Pattern

Wraps SDK calls in a Svelte writable store. The store updates reactively via `onAuthStateChange` -- you never manually set user state after login/logout.

```typescript
// lib/stores/auth.ts
import { writable, derived } from 'svelte/store';
import { initSession, login, register, logout, onAuthStateChange } from '$lib/raisin';
import type { IdentityUser, AuthStateChange } from '@raisindb/client';

function createAuthStore() {
  const { subscribe, set, update } = writable<{
    user: IdentityUser | null; loading: boolean; initialized: boolean;
  }>({ user: null, loading: false, initialized: false });

  let unsub: (() => void) | null = null;

  return {
    subscribe,
    async init() {
      set({ user: null, loading: true, initialized: false });
      unsub = onAuthStateChange(({ event, session }: AuthStateChange) => {
        if (event === 'SIGNED_IN')
          set({ user: session.user, loading: false, initialized: true });
        else if (event === 'SIGNED_OUT' || event === 'SESSION_EXPIRED')
          set({ user: null, loading: false, initialized: true });
        else // TOKEN_REFRESHED, USER_UPDATED
          update((s) => ({ ...s, user: session.user }));
      });
      const user = await initSession();
      set({ user, loading: false, initialized: true });
    },
    async login(email: string, password: string) {
      update((s) => ({ ...s, loading: true }));
      try { await login(email, password); return { success: true as const }; }
      catch (err: unknown) {
        update((s) => ({ ...s, loading: false }));
        const e = err as { code?: string; message?: string };
        return { success: false as const, error: { code: e.code ?? 'LOGIN_FAILED', message: e.message ?? 'Login failed' } };
      }
    },
    async register(email: string, password: string, displayName?: string) {
      update((s) => ({ ...s, loading: true }));
      try { await register(email, password, displayName); return { success: true as const }; }
      catch (err: unknown) {
        update((s) => ({ ...s, loading: false }));
        const e = err as { code?: string; message?: string };
        return { success: false as const, error: { code: e.code ?? 'REGISTRATION_FAILED', message: e.message ?? 'Registration failed' } };
      }
    },
    async logout() { await logout(); },
    cleanup() { unsub?.(); unsub = null; },
  };
}

export const auth = createAuthStore();
export const user = derived(auth, ($a) => $a.user);
export const isAuthenticated = derived(auth, ($a) => $a.user !== null);
export const isLoading = derived(auth, ($a) => $a.loading);
```

Initialize in root layout:

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import { auth } from '$lib/stores/auth';
  onMount(() => { auth.init(); return () => auth.cleanup(); });
</script>
<slot />
```

## React Auth Pattern

```tsx
import { createContext, useContext, useEffect, useState } from 'react';
import { RaisinClient, LocalStorageTokenStorage, type IdentityUser } from '@raisindb/client';

const client = new RaisinClient('ws://localhost:8080/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myapp'),
});
const AuthContext = createContext<{
  user: IdentityUser | null; loading: boolean;
  login: (email: string, password: string) => Promise<IdentityUser>;
  logout: () => Promise<void>;
}>(null!);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [user, setUser] = useState<IdentityUser | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    client.initSession('myrepo').then(setUser).finally(() => setLoading(false));
    return client.onAuthStateChange(({ session }) => setUser(session.user));
  }, []);

  return (
    <AuthContext.Provider value={{
      user, loading,
      login: (e, p) => client.loginWithEmail(e, p, 'myrepo'),
      logout: () => client.logout(),
    }}>{children}</AuthContext.Provider>
  );
}
export const useAuth = () => useContext(AuthContext);
```

## Login Page

```svelte
<script lang="ts">
  import { goto } from '$app/navigation';
  import { auth } from '$lib/stores/auth';

  let email = $state('');
  let password = $state('');
  let error = $state<string | null>(null);
  let submitting = $state(false);

  async function handleSubmit(e: Event) {
    e.preventDefault();
    error = null;
    submitting = true;
    const result = await auth.login(email, password);
    if (result.success) goto('/');
    else { error = result.error.message; submitting = false; }
  }
</script>

<form onsubmit={handleSubmit}>
  <input type="email" bind:value={email} required disabled={submitting} />
  <input type="password" bind:value={password} required disabled={submitting} />
  {#if error}<p class="error">{error}</p>{/if}
  <button type="submit" disabled={submitting}>
    {submitting ? 'Signing in...' : 'Sign In'}
  </button>
  <p>No account? <a href="/auth/register">Register</a></p>
</form>
```

## Register Page

Same pattern with password confirmation. Validate passwords match and min 8 chars before calling `auth.register(email, password, displayName)`. The register method works identically to login -- the SDK emits `SIGNED_IN` and the store updates reactively. Add fields for display name, email, password, and confirm password.

## Protected Routes

Redirect unauthenticated users to login, then send them back after login:

```svelte
<!-- Protected page: routes/dashboard/+page.svelte -->
<script lang="ts">
  import { goto } from '$app/navigation';
  import { user, isAuthenticated, isLoading } from '$lib/stores/auth';
  import { page } from '$app/stores';

  $effect(() => {
    if (!$isLoading && !$isAuthenticated)
      goto(`/auth/login?redirect=${encodeURIComponent($page.url.pathname)}`);
  });
</script>

{#if $isAuthenticated}
  <h1>Welcome, {$user?.display_name}</h1>
{/if}
```

On the login page, read the redirect param and navigate back:

```typescript
const result = await auth.login(email, password);
if (result.success) {
  const redirect = $page.url.searchParams.get('redirect') || '/';
  goto(redirect);
}
```
