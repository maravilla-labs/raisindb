import type { PageLoad } from './$types';

/**
 * Inbox page loader — conversations are loaded by ConversationListStore,
 * so the SSR loader just passes through user context.
 */
export const load: PageLoad = async ({ parent }) => {
  const { user } = await parent();

  return {
    loggedIn: !!user?.home,
  };
};
