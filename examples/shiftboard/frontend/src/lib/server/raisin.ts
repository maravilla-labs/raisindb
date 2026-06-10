/**
 * Server-side RaisinDB access for SSR.
 *
 * Auth model: the SvelteKit server owns the session. The login form action
 * posts to RaisinDB's identity endpoint (POST /auth/{repo}/login) and stores
 * access token, refresh token and the identity profile in httpOnly cookies.
 * Every SSR load builds a request-scoped RaisinHttpClient from the access
 * cookie; the browser receives the access token via layout data only to
 * authenticate its WebSocket client ({ type: 'jwt' }).
 *
 * SDK APIs used:
 *   RaisinHttpClient (pure HTTP, Node-safe)  - SQL over /api/sql/{repo}
 *   ConversationManager                       - conversation list + history
 *   AuthManager.parseToken                    - JWT expiry inspection
 */
import { env } from '$env/dynamic/private';
import type { Cookies } from '@sveltejs/kit';
import {
  AuthManager,
  ConversationManager,
  InboxApi,
  RaisinHttpClient,
  type IdentityAuthResponse,
  type IdentityUser,
  type InboxTask,
} from '@raisindb/client';

/** Same env vars as the client bundle; baked in at build time. */
const WS_URL =
  import.meta.env.VITE_RAISIN_WS_URL ?? 'ws://localhost:8081/ws/shiftboard';

export const REPOSITORY = import.meta.env.VITE_RAISIN_REPO ?? 'shiftboard';

/** ws(s)://host/ws/{repo} → http(s)://host */
function deriveHttpBase(wsUrl: string): string {
  const url = new URL(wsUrl);
  url.protocol = url.protocol === 'wss:' ? 'https:' : 'http:';
  return url.origin;
}

/** HTTP base for SSR fetches. Runtime-overridable via RAISIN_HTTP_URL. */
export function httpBase(): string {
  return (env.RAISIN_HTTP_URL ?? deriveHttpBase(WS_URL)).replace(/\/$/, '');
}

// ---------------------------------------------------------------------------
// Cookies
// ---------------------------------------------------------------------------

export const COOKIE_ACCESS = 'shiftboard_access';
export const COOKIE_REFRESH = 'shiftboard_refresh';
export const COOKIE_IDENTITY = 'shiftboard_identity';

// Demo runs over plain http://localhost, so `secure` must be off or
// browsers/curl would drop the cookies.
const COOKIE_OPTS = { path: '/', httpOnly: true, sameSite: 'lax', secure: false } as const;

export function setAuthCookies(cookies: Cookies, tokens: IdentityAuthResponse): void {
  const user: IdentityUser = {
    id: tokens.identity.id,
    email: tokens.identity.email,
    displayName: tokens.identity.display_name,
    avatarUrl: tokens.identity.avatar_url,
    emailVerified: tokens.identity.email_verified,
    home: tokens.identity.home,
  };

  // expires_at is a unix timestamp in ms.
  const accessMaxAge = Math.max(60, Math.floor((tokens.expires_at - Date.now()) / 1000));
  const refreshMaxAge = 30 * 24 * 60 * 60; // 30 days

  cookies.set(COOKIE_ACCESS, tokens.access_token, { ...COOKIE_OPTS, maxAge: accessMaxAge });
  cookies.set(COOKIE_REFRESH, tokens.refresh_token, { ...COOKIE_OPTS, maxAge: refreshMaxAge });
  cookies.set(COOKIE_IDENTITY, JSON.stringify(user), { ...COOKIE_OPTS, maxAge: refreshMaxAge });
}

export function clearAuthCookies(cookies: Cookies): void {
  cookies.delete(COOKIE_ACCESS, { path: '/' });
  cookies.delete(COOKIE_REFRESH, { path: '/' });
  cookies.delete(COOKIE_IDENTITY, { path: '/' });
}

// ---------------------------------------------------------------------------
// Identity auth endpoints (plain fetch — login/refresh are pre-auth)
// ---------------------------------------------------------------------------

export async function loginWithEmail(
  email: string,
  password: string,
): Promise<{ ok: true; tokens: IdentityAuthResponse } | { ok: false; error: string }> {
  const res = await fetch(`${httpBase()}/auth/${REPOSITORY}/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email, password }),
  });
  if (!res.ok) {
    const err = await res.json().catch(() => ({}) as { message?: string });
    return { ok: false, error: err.message ?? 'Login failed' };
  }
  return { ok: true, tokens: (await res.json()) as IdentityAuthResponse };
}

export async function refreshTokens(refreshToken: string): Promise<IdentityAuthResponse | null> {
  try {
    const res = await fetch(`${httpBase()}/auth/${REPOSITORY}/refresh`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ refresh_token: refreshToken }),
    });
    if (!res.ok) return null;
    return (await res.json()) as IdentityAuthResponse;
  } catch {
    return null;
  }
}

/** True when the JWT is missing an exp or expires within `windowSec`. */
export function tokenExpiresSoon(token: string, windowSec = 120): boolean {
  const claims = AuthManager.parseToken(token);
  if (typeof claims?.exp !== 'number') return false;
  return claims.exp * 1000 < Date.now() + windowSec * 1000;
}

// ---------------------------------------------------------------------------
// Request-scoped HTTP client
// ---------------------------------------------------------------------------

/**
 * Build a request-scoped RaisinHttpClient carrying the user's bearer token.
 *
 * SDK friction worked around here: `RaisinHttpClient.authenticate({type:'jwt'})`
 * stores the token but never records an expiry, so `isAuthenticated()` stays
 * false and `request()` silently skips the Authorization header. We inject the
 * header via a custom fetch and ALSO seed the token storage so that
 * ConversationManager (which reads the AuthManager directly) is covered.
 */
export function createHttpClient(token: string): RaisinHttpClient {
  const client = new RaisinHttpClient(httpBase(), {
    tenantId: 'default',
    defaultBranch: 'main',
    fetch: (input, init = {}) =>
      fetch(input, {
        ...init,
        headers: { ...(init.headers as Record<string, string> | undefined), Authorization: `Bearer ${token}` },
      }),
  });
  client.getAuthManager().storage.setAccessToken(token);
  return client;
}

/**
 * ConversationManager wired to the request-scoped HTTP client.
 *
 * SQL is routed through HttpDatabase.executeSql (POST /api/sql/{repo}) because
 * the manager's built-in HTTP fallback posts to /api/sql/{repo}/query, which
 * the server interprets as branch "query".
 */
export function createConversationManager(client: RaisinHttpClient): ConversationManager {
  const db = client.database(REPOSITORY);
  return new ConversationManager(
    httpBase(),
    REPOSITORY,
    client.getAuthManager(),
    {},
    (query, params) => db.executeSql(query, params),
  );
}

/**
 * The logged-in user's pending inbox tasks (raisin:InboxTask nodes), for the
 * SSR seed of the task panel. GET /api/inbox/{repo}?status=pending via the
 * SDK's InboxApi — the caller's bearer scopes the list to their own inbox.
 * Inbox failures must not break the page render, so errors yield [].
 */
export async function listPendingTasks(client: RaisinHttpClient): Promise<InboxTask[]> {
  try {
    const inbox = new InboxApi(httpBase(), REPOSITORY, client.getAuthManager());
    const { tasks } = await inbox.listTasks({ status: 'pending' });
    return tasks;
  } catch (err) {
    console.warn('[ssr] loading inbox tasks failed, panel starts empty:', err);
    return [];
  }
}
