import type { PageLoad } from './$types';
import { query } from '$lib/raisin';
import type { Conversation, Message } from '$lib/stores/messaging-store';

const ACCESS_CONTROL = 'raisin:access_control';

interface ConversationNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    participants: string[];
    participant_details?: Record<string, { display_name?: string }>;
    last_message?: {
      content?: string;
      sender_id?: string;
      sender_display_name?: string;
      recipient_id?: string;
      recipient_display_name?: string;
      created_at?: string;
    };
    updated_at?: string;
    unread_count?: number;
  };
}

/**
 * Load a specific conversation by ID.
 * Queries directly using user from parent to avoid Svelte store timing issues.
 */
export const load: PageLoad = async ({ params, parent }) => {
  const { user } = await parent();

  if (!user?.home) {
    return {
      conversationId: params.id,
      error: 'Not logged in',
    };
  }

  const homePath = user.home.replace(`/${ACCESS_CONTROL}`, '');
  const conversationPath = `${homePath}/inbox/chats/${params.id}`;

  try {
    // Query conversation node directly
    const convNodes = await query<ConversationNode>(`
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL}'
      WHERE path = '${conversationPath}'
    `);

    const convNode = convNodes[0];
    if (!convNode) {
      return {
        conversationId: params.id,
        error: 'Conversation not found',
      };
    }

    // Query messages for this conversation
    const messages = await query<Message>(`
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL}'
      WHERE CHILD_OF('${conversationPath}')
        AND node_type = 'raisin:Message'
      ORDER BY properties->>'created_at' ASC
    `);

    // Build conversation object from node
    const participants = convNode.properties.participants || [];
    const otherId = participants.find(p => p !== user.id) || participants[0];
    const lm = convNode.properties.last_message;
    const otherDetails = convNode.properties.participant_details?.[otherId];

    const otherDisplayName = otherDetails?.display_name ||
      (lm?.sender_id === otherId ? lm?.sender_display_name : lm?.recipient_display_name) ||
      otherId ||
      'User';

    const conversation: Conversation = {
      id: convNode.name,
      participantId: otherId,
      participantDisplayName: otherDisplayName,
      lastMessage: null,
      lastMessageAt: convNode.properties.updated_at || '',
      unreadCount: convNode.properties.unread_count || 0,
    };

    return {
      conversationId: params.id,
      conversation,
      messages,
      error: null,
    };
  } catch (err) {
    console.error('[inbox/[id]] Load failed:', err);
    return {
      conversationId: params.id,
      error: 'Failed to load conversation',
    };
  }
};
