import type { PageLoad } from './$types';
import { query } from '$lib/raisin';
import type { Message } from '$lib/stores/messaging';

const ACCESS_CONTROL = 'raisin:access_control';

/**
 * Load inbox messages for the "All Messages" tab.
 * Conversations are handled by messagingStore (single source of truth).
 */
export const load: PageLoad = async ({ parent }) => {
  const { user } = await parent();

  if (!user?.home) {
    return {
      messages: [] as Message[],
      unreadCount: 0,
      error: 'Not logged in',
    };
  }

  const homePath = user.home.replace(`/${ACCESS_CONTROL}`, '');
  const inboxPath = `${homePath}/inbox`;

  try {
    // Fetch inbox messages for "All Messages" tab
    // This includes friendship requests, notifications, etc.
    // Direct messages are handled by messagingStore for the Conversations tab
    const inboxMessages = await query<Message>(`
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL}'
      WHERE DESCENDANT_OF('${inboxPath}')
        AND node_type = 'raisin:Message'
        AND properties->>'message_type'::STRING NOT IN ('read_receipt')
      ORDER BY properties->>'created_at' DESC
    `);

    // Count unread messages (for All Messages tab badge)
    const unreadCount = inboxMessages.filter(
      msg => msg.properties.status !== 'read'
    ).length;

    return {
      messages: inboxMessages,
      unreadCount,
      error: null,
    };
  } catch (error) {
    console.error('[inbox] Load error:', error);
    return {
      messages: [] as Message[],
      unreadCount: 0,
      error: error instanceof Error ? error.message : 'Failed to load data',
    };
  }
};
