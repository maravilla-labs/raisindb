/**
 * Root layout load function
 *
 * This runs BEFORE any child +page.ts load functions, ensuring
 * authentication is complete before page data is fetched.
 */
import type { LayoutLoad } from './$types';
import { browser } from '$app/environment';
import { initSession, getNavigation, query, type IdentityUser, type NavItem } from '$lib/raisin';

// SPA mode - disable SSR and prerendering
export const ssr = false;
export const prerender = false;

const ACCESS_CONTROL = 'raisin:access_control';

export const load: LayoutLoad = async () => {
  // Only run in browser (not during SSR build)
  if (!browser) {
    return {
      user: null as IdentityUser | null,
      navigationItems: [] as NavItem[],
      unreadCount: 0,
      error: null as string | null
    };
  }

  try {
    // Check for token in URL params (for Quest browser auth)
    if (typeof window !== 'undefined') {
      const urlParams = new URLSearchParams(window.location.search);
      const tokenFromUrl = urlParams.get('token');
      if (tokenFromUrl) {
        // Inject token into localStorage so initSession() picks it up
        localStorage.setItem('launchpad-next_access_token', tokenFromUrl);

        // Clean up URL (remove token from address bar for security)
        const cleanUrl = window.location.pathname;
        window.history.replaceState({}, '', cleanUrl);
      }
    }

    // 1. Initialize auth session - this connects WebSocket and authenticates
    const user = await initSession();

    // 2. Load navigation items (now authenticated)
    const navigationItems = await getNavigation();

    // 3. Load unread inbox count if user is logged in
    let unreadCount = 0;
    if (user?.home) {
      try {
        const homePath = user.home.replace(`/${ACCESS_CONTROL}`, '');
        const inboxPath = `${homePath}/inbox`;
        const result = await query<{ count: number }>(`
          SELECT COUNT(*) as count
          FROM '${ACCESS_CONTROL}'
          WHERE DESCENDANT_OF('${inboxPath}')
            AND node_type = 'raisin:Message'
            AND properties->>'status'::STRING NOT IN ('read', 'accepted', 'declined')
        `);
        unreadCount = result[0]?.count ?? 0;
      } catch (err) {
        console.error('[layout.ts] Failed to load unread count:', err);
      }
    }

    return {
      user,
      navigationItems,
      unreadCount,
      error: null
    };
  } catch (e) {
    console.error('[layout.ts] Initialization failed:', e);
    return {
      user: null,
      navigationItems: [],
      unreadCount: 0,
      error: e instanceof Error ? e.message : 'Failed to connect to RaisinDB'
    };
  }
};
