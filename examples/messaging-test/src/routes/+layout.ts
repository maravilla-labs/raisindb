/**
 * Root layout load function
 */
import type { LayoutLoad } from './$types';
import { browser } from '$app/environment';
import { initSession, type IdentityUser } from '$lib/raisin';

export const ssr = false;
export const prerender = false;

export const load: LayoutLoad = async () => {
  if (!browser) {
    return {
      user: null as IdentityUser | null,
      error: null as string | null,
    };
  }

  try {
    const user = await initSession();

    return {
      user,
      error: null,
    };
  } catch (e) {
    console.error('[layout.ts] Initialization failed:', e);
    return {
      user: null,
      error: e instanceof Error ? e.message : 'Failed to connect to RaisinDB',
    };
  }
};
