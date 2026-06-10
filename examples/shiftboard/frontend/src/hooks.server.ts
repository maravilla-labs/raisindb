/**
 * Resolves the auth session from httpOnly cookies on every request.
 *
 * - Access token near expiry (or missing) + refresh cookie → refresh via
 *   POST /auth/{repo}/refresh and rotate the cookies.
 * - Valid access token + identity cookie → event.locals.session.
 * - Anything else → locals.session = null (page renders the login form).
 */
import type { Handle } from '@sveltejs/kit';
import type { IdentityUser } from '@raisindb/client';
import {
  COOKIE_ACCESS,
  COOKIE_IDENTITY,
  COOKIE_REFRESH,
  clearAuthCookies,
  refreshTokens,
  setAuthCookies,
  tokenExpiresSoon,
} from '$lib/server/raisin';

export const handle: Handle = async ({ event, resolve }) => {
  const { cookies } = event;
  let access = cookies.get(COOKIE_ACCESS);
  const refresh = cookies.get(COOKIE_REFRESH);

  if ((!access || tokenExpiresSoon(access)) && refresh) {
    const tokens = await refreshTokens(refresh);
    if (tokens) {
      setAuthCookies(cookies, tokens);
      access = tokens.access_token;
    } else {
      clearAuthCookies(cookies);
      access = undefined;
    }
  }

  event.locals.session = null;
  const identityRaw = cookies.get(COOKIE_IDENTITY);
  if (access && identityRaw) {
    try {
      event.locals.session = { token: access, user: JSON.parse(identityRaw) as IdentityUser };
    } catch {
      clearAuthCookies(cookies);
    }
  }

  return resolve(event);
};
