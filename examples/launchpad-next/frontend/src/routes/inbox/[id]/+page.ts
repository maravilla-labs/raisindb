import type { PageLoad } from './$types';
import { query } from '$lib/raisin';

const ACCESS_CONTROL = 'raisin:access_control';

/** Conversation metadata loaded from the node tree (SSR). */
export interface ConversationData {
  id: string;
  subject?: string;
  participantId: string;
  participantDisplayName: string;
  unreadCount: number;
  conversationPath: string;
}

/** Raw message row from SQL query (SSR). */
export interface MessageRow {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: Record<string, any>;
}

interface ConversationNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    subject?: string;
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
      myWorkspaceNodeId: '',
      error: 'Not logged in',
    };
  }

  const homePath = user.home.replace(`/${ACCESS_CONTROL}`, '');
  const conversationPath = `${homePath}/inbox/chats/${params.id}`;

  try {
    // Query the current user's workspace node ID
    const userNodes = await query<{ id: string }>(`
      SELECT id FROM '${ACCESS_CONTROL}'
      WHERE path = '${homePath}'
        AND node_type = 'raisin:User'
    `);
    const myWorkspaceNodeId = userNodes[0]?.id || '';

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
        myWorkspaceNodeId,
        error: 'Conversation not found',
      };
    }

    // Query messages for this conversation
    const messages = await query<MessageRow>(`
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL}'
      WHERE CHILD_OF('${conversationPath}')
        AND node_type = 'raisin:Message'
      ORDER BY properties->>'created_at' ASC
    `);

    // Build conversation object from node
    const participants = convNode.properties.participants || [];
    const otherId = participants.find(p => p !== myWorkspaceNodeId) || participants[0];
    const lm = convNode.properties.last_message;
    const otherDetails = convNode.properties.participant_details?.[otherId];

    const otherDisplayName = otherDetails?.display_name ||
      (lm?.sender_id === otherId ? lm?.sender_display_name : lm?.recipient_display_name) ||
      otherId ||
      'User';

    const conversation: ConversationData = {
      id: convNode.name,
      subject: convNode.properties.subject,
      participantId: otherId,
      participantDisplayName: otherDisplayName,
      unreadCount: convNode.properties.unread_count || 0,
      conversationPath,
    };

    return {
      conversationId: params.id,
      myWorkspaceNodeId,
      conversation,
      messages,
      conversationPath,
      error: null,
    };
  } catch (err) {
    console.error('[inbox/[id]] Load failed:', err);
    return {
      conversationId: params.id,
      myWorkspaceNodeId: '',
      error: 'Failed to load conversation',
    };
  }
};
