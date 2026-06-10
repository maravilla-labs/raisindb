/**
 * Session state (Svelte 5 runes).
 *
 * With SSR the session is owned by the server (httpOnly cookies set by the
 * ?/login form action, cleared by ?/logout). This store just mirrors the
 * user from layout data so components like Header can read it reactively.
 * Login/logout are plain HTML form posts — see +page.server.ts.
 */
import type { IdentityUser } from '@raisindb/client';

class SessionState {
  user = $state<IdentityUser | null>(null);
}

export const session = new SessionState();
