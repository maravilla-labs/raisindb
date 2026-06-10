/**
 * Layout load: validate the cookie session against the live server and hand
 * the access token + user to the page (and, after hydration, to the browser's
 * WebSocket client).
 *
 * Validation uses SQL `RAISIN_CURRENT_USER()` over HTTP — the natural
 * "who am I" check for identity JWTs (it also yields the user's home path,
 * which drives the inbox subscription). A 401/403 means the token is no
 * longer good: clear the cookies and render the login form.
 */
import type { LayoutServerLoad } from './$types';
import { REPOSITORY, clearAuthCookies, createHttpClient } from '$lib/server/raisin';

interface UserNodeJson {
  path?: string;
}

export const load: LayoutServerLoad = async ({ locals, cookies }) => {
  if (!locals.session) return { session: null };

  const client = createHttpClient(locals.session.token);
  try {
    const result = await client
      .database(REPOSITORY)
      .executeSql('SELECT RAISIN_CURRENT_USER() as user_node');

    // Over HTTP the node arrives as a JSON string.
    const raw = (result.rows?.[0] as { user_node?: unknown } | undefined)?.user_node;
    const node: UserNodeJson | null =
      typeof raw === 'string' ? (JSON.parse(raw) as UserNodeJson) : ((raw as UserNodeJson) ?? null);
    if (!node?.path) throw Object.assign(new Error('Not authenticated'), { status: 401 });

    return {
      session: {
        token: locals.session.token,
        user: { ...locals.session.user, home: locals.session.user.home ?? node.path },
      },
    };
  } catch (err) {
    const status = (err as { status?: number }).status;
    if (status === 401 || status === 403) {
      clearAuthCookies(cookies);
      return { session: null };
    }
    throw err;
  }
};
