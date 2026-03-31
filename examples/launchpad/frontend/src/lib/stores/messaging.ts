/**
 * Messaging utilities for social features.
 *
 * Provides functions for:
 * - Friendship requests (send, accept, decline)
 * - Direct messages between friends
 * - Friend list queries using GRAPH_TABLE
 */
import { get } from 'svelte/store';
import { query, queryOne } from '$lib/raisin';
import { user } from './auth';

const ACCESS_CONTROL = 'raisin:access_control';

export interface Message {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    message_type: string;
    subject?: string;
    // Threading fields
    conversation_id?: string;
    sender_id?: string;
    recipient_id?: string;
    sender_display_name?: string;
    relation_type?: string;
    relation_title?: string;
    message?: string;
    accepted?: boolean;
    original_request_id?: string;
    received_at?: string;
    body?: {
      // Common fields
      message?: string;
      message_text?: string;
      thread_id?: string;
      // Direct message fields
      content?: string;
      // Friendship response fields
      response?: 'accepted' | 'declined';
      original_request_id?: string;
      responder_id?: string;
      responder_display_name?: string;
      // Notification fields
      notification_type?: string;
      // Read receipt fields
      original_message_id?: string;
      reader_id?: string;
      message_ids?: string[];
      read_at?: string;
    };
    status: string;
    created_at?: string;
    error?: string;
    client_id?: string;
    // Read tracking for multi-recipient support
    read_by?: Array<{
      id: string;
      read_at: string;
    }>;
  };
}

export interface ConversationNode {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    subject?: string;
    participants: string[];
    participant_details?: Record<string, { display_name?: string }>;
    last_message?: {
        content: string;
        message_text?: string;
        sender_id?: string;
        sender_display_name?: string;
        recipient_id?: string;
        recipient_display_name?: string;
        created_at: string;
    };
    unread_count?: number;
    updated_at?: string;
  }
}

export interface Conversation {
  id: string;                    // conversation_id (node name or ID)
  participantId: string;         // Other participant's user id
  participantDisplayName: string;
  lastMessage?: Message;
  lastMessageAt: string;
  unreadCount: number;
}

export interface Friend {
  id: string;
  path: string;
  properties: {
    email: string;
    display_name?: string;
  };
}

export interface FriendSuggestion extends Friend {
  degree?: number;
  hasPendingRequest?: boolean;
}

/**
 * Generate a deterministic conversation ID from two user ids.
 */
export function getConversationId(id1: string, id2: string): string {
  const sorted = [id1, id2].sort();
  const combined = sorted.join(':');
  let hash = 0;
  for (let i = 0; i < combined.length; i++) {
    const char = combined.charCodeAt(i);
    hash = ((hash << 5) - hash) + char;
    hash = hash & hash;
  }
  return `conv-${Math.abs(hash).toString(36)}`;
}

export async function sendFriendshipRequest(
  recipientEmail: string,
  message?: string
): Promise<{ success: boolean; error?: string }> {
  const currentUser = get(user);

  if (!currentUser?.home) return { success: false, error: 'Not logged in' };

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const outboxPath = `${homePath}/outbox`;

  try {
    const recipient = await queryOne<{ id: string; path: string }>(`
      SELECT id, path FROM '${ACCESS_CONTROL}'
      WHERE node_type = 'raisin:User'
        AND properties->>'email' = $1
      LIMIT 1
    `, [recipientEmail]);

    if (!recipient?.path) {
      return { success: false, error: 'User not found' };
    }

    const sql = `
      INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties)
      VALUES ($1, 'raisin:Message', $2::jsonb)
      RETURNING id, path
    `;

    const messageName = `rel-req-${Date.now()}`;
    const messagePath = `${outboxPath}/${messageName}`;
    const properties = {
      message_type: 'relationship_request',
      subject: 'Friendship Request',
      status: 'pending',
      relation_type: 'FRIENDS_WITH',
      message: message || null,
      body: {
        message: message || null
      },
      sender_id: currentUser.id,
      recipient_id: recipient.id,
      created_at: new Date().toISOString()
    };

    await query(sql, [messagePath, JSON.stringify(properties)]);
    return { success: true };
  } catch (err) {
    console.error('[messaging] Failed to send friendship request:', err);
    return { success: false, error: (err as any).message || 'Failed' };
  }
}

/**
 * Get all conversations by querying raisin:Conversation nodes.
 * 
 * Bucket: /inbox/chats
 */
export async function getConversations(): Promise<Conversation[]> {
  const currentUser = get(user);
  if (!currentUser?.home) return [];

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const chatsPath = `${homePath}/inbox/chats`;

  try {
    const sql = `
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL}'
      WHERE CHILD_OF('${chatsPath}')
      ORDER BY properties->>'updated_at' DESC
    `;

    const convNodes = await query<ConversationNode>(sql);
    const conversations: Conversation[] = [];

    for (const node of convNodes) {
        // Determine "other" participant (not me)
        const participants = node.properties.participants || [];
        const otherId = participants.find(p => p !== currentUser.id) || participants[0]; // fallback

        const lm = node.properties.last_message;

        // Get other participant's details from participant_details cache
        const otherDetails = node.properties.participant_details?.[otherId];

        // Get other participant's display name - NOT the last message sender
        const otherDisplayName = otherDetails?.display_name ||
                                 (lm?.sender_id === otherId ? lm?.sender_display_name : lm?.recipient_display_name) ||
                                 otherId ||
                                 'User';

        // Construct a "fake" last message object for UI compatibility
        const lastMessage: Message = {
            id: 'latest',
            path: node.path,
            name: 'latest',
            node_type: 'raisin:Message',
            properties: {
                message_type: 'chat',
                status: 'read', // strictly for preview
                created_at: lm?.created_at,
                body: {
                    message_text: lm?.content || lm?.message_text || '',
                    content: lm?.content || lm?.message_text || ''
                }
            }
        };

        conversations.push({
            id: node.name, // "conv-XYZ"
            participantId: otherId,
            participantDisplayName: otherDisplayName,
            lastMessage: lastMessage,
            lastMessageAt: node.properties.updated_at || '',
            unreadCount: node.properties.unread_count || 0
        });
    }

    return conversations;

  } catch (err) {
    console.error('[messaging] Failed to get conversations:', err);
    return [];
  }
}

/**
 * Get messages for a specific conversation.
 * 
 * Bucket: /inbox/chats/{convId}
 */
export async function getConversationMessages(conversationId: string): Promise<Message[]> {
  const currentUser = get(user);
  if (!currentUser?.home) return [];

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  // Helper: Try to find where the conversation is (Inbox/Chats)
  // We assume conversationId is the name (e.g., "conv-xyz")
  const conversationPath = `${homePath}/inbox/chats/${conversationId}`;

  try {
    const sql = `
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL}'
      WHERE CHILD_OF('${conversationPath}')
        AND node_type = 'raisin:Message'
      ORDER BY properties->>'created_at' ASC
    `;

    return await query<Message>(sql);
  } catch (err) {
    console.error('[messaging] Failed to get conversation messages:', err);
    return [];
  }
}

/**
 * Get total unread message count across all conversations.
 * 
 * Bucket: /inbox/chats
 */
export async function getUnreadConversationCount(): Promise<number> {
  const currentUser = get(user);
  if (!currentUser?.home) return 0;

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const chatsPath = `${homePath}/inbox/chats`;

  try {
    const sql = `
      SELECT SUM((properties->>'unread_count')::int) as count
      FROM '${ACCESS_CONTROL}'
      WHERE CHILD_OF('${chatsPath}')
    `;

    const result = await queryOne<{ count: number }>(sql);
    return result?.count ?? 0;
  } catch (err) {
    console.error('[messaging] Failed to get unread conversation count:', err);
    return 0;
  }
}

/**
 * Mark conversation as read.
 *
 * Bucket: /inbox/chats/{convId}
 *
 * Optimized to send a SINGLE batch read receipt instead of N individual receipts.
 */
export async function markConversationAsRead(conversationId: string): Promise<boolean> {
  const currentUser = get(user);
  if (!currentUser?.home) return false;

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const conversationPath = `${homePath}/inbox/chats/${conversationId}`;
  const outboxPath = `${homePath}/outbox`;

  try {
    // 1. Reset unread count on Conversation Node
    await query(`
        UPDATE '${ACCESS_CONTROL}'
        SET properties = properties || '{"unread_count": 0}'
        WHERE path = $1
    `, [conversationPath]);

    // 2. Find unread messages from OTHER users (not our own messages)
    const messages = await query<Message>(`
      SELECT id, path, name, node_type, properties
      FROM '${ACCESS_CONTROL}'
      WHERE CHILD_OF('${conversationPath}')
        AND properties->>'status' != 'read'
        AND properties->>'sender_id' != $1
    `, [currentUser.id]);

    if (messages.length === 0) {
      return true; // Nothing to mark as read
    }

    // 3. Mark all messages as read locally (status update)
    await query(`
      UPDATE '${ACCESS_CONTROL}'
      SET properties = properties || '{"status": "read"}'::jsonb
      WHERE CHILD_OF('${conversationPath}')
        AND properties->>'status' != 'read'
    `);

    // 4. Send a SINGLE batch read receipt instead of N individual receipts
    const receiptName = `batch-read-receipt-${Date.now()}`;
    const receiptPath = `${outboxPath}/${receiptName}`;
    const receiptProperties = {
      message_type: 'batch_read_receipt',
      status: 'pending',
      conversation_id: conversationId,
      body: {
        message_ids: messages.map(m => m.id),
        reader_id: currentUser.id,
        read_at: new Date().toISOString()
      },
      sender_id: currentUser.id,
      created_at: new Date().toISOString()
    };

    await query(`
      INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties)
      VALUES ($1, 'raisin:Message', $2::jsonb)
    `, [receiptPath, JSON.stringify(receiptProperties)]);

    return true;
  } catch (err) {
    console.error('[messaging] Failed to mark conversation as read:', err);
    return false;
  }
}

/**
 * @deprecated Use messagingStore from './messaging-store' instead.
 * Subscription is now handled internally by the unified messaging store.
 */
export async function subscribeToConversations(
  _callback: (event: any) => void
): Promise<() => void> {
  console.warn('[messaging] subscribeToConversations is deprecated. Use messagingStore instead.');
  return () => {};
}

export async function markAsRead(messagePath: string): Promise<boolean> {
  try {
    await query(`
      UPDATE '${ACCESS_CONTROL}'
      SET properties = properties || CAST('{"status": "read"}' AS JSONB)
      WHERE path LIKE $1
    `, [messagePath]);
    return true;
  } catch (err) {
    console.error('[messaging] Failed to mark as read:', err);
    return false;
  }
}

export async function markDirectMessageAsRead(message: Message): Promise<boolean> {
  const currentUser = get(user);
  if (!currentUser?.home) return false;
  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const outboxPath = `${homePath}/outbox`;
  const senderId = message.properties.sender_id;
  
  if (!senderId || senderId === currentUser.id) {
    return markAsRead(message.path);
  }

  try {
    await markAsRead(message.path);
    // Send Receipt
    const receiptName = `read-receipt-${Date.now()}`;
    const receiptPath = `${outboxPath}/${receiptName}`;
    const receiptProperties = {
      message_type: 'read_receipt',
      status: 'pending',
      conversation_id: message.properties.conversation_id,
      body: {
        original_message_id: message.id,
        reader_id: currentUser.id,
        read_at: new Date().toISOString()
      },
      sender_id: currentUser.id,
      created_at: new Date().toISOString()
    };

    await query(`
      INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties)
      VALUES ($1, 'raisin:Message', $2::jsonb)
    `, [receiptPath, JSON.stringify(receiptProperties)]);
    return true;
  } catch (err) {
    console.error('[messaging] Receipt failed:', err);
    return true;
  }
}

export async function sendDirectMessage(
  recipientId: string,
  content: string,
  subject?: string,
  clientId?: string
): Promise<{ success: boolean; error?: string; conversationId?: string }> {
  const currentUser = get(user);
  if (!currentUser?.home) return { success: false, error: 'Not logged in' };

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const outboxPath = `${homePath}/outbox`;
  const conversationId = getConversationId(currentUser.id, recipientId);

  try {
    const sql = `
      INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties)
      VALUES ($1, 'raisin:Message', $2::jsonb)
      RETURNING id, path
    `;

    const messageName = `chat-${Date.now()}`;
    const messagePath = `${outboxPath}/${messageName}`;
    const properties = {
      message_type: 'chat',
      subject: subject || 'Chat',
      status: 'pending',
      conversation_id: conversationId,
      body: {
        message_text: content,
        content: content
      },
      sender_id: currentUser.id,
      recipient_id: recipientId,
      created_at: new Date().toISOString(),
      client_id: clientId // Add client_id for optimistic UI matching
    };

    await query(sql, [messagePath, JSON.stringify(properties)]);
    return { success: true, conversationId };
  } catch (err) {
    console.error('[messaging] Failed to send DM:', err);
    return { success: false, error: (err as any).message };
  }
}

export async function sendReply(
  conversationId: string,
  recipientId: string,
  content: string,
  clientId?: string
): Promise<{ success: boolean; error?: string }> {
  return sendDirectMessage(recipientId, content, undefined, clientId);
}

export async function getFriends(): Promise<Friend[]> {
    const currentUser = get(user);
    if (!currentUser?.home) return [];
    const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
    try {
        const sql = `SELECT * FROM GRAPH_TABLE(MATCH (me)-[:FRIENDS_WITH]->(friend) WHERE me.path = '${homePath}' COLUMNS (friend.id AS id, friend.path AS path, friend.properties AS properties)) AS g`;
        const result = await query<Friend>(sql);
        return result || [];
    } catch (err) { return []; }
}

/**
 * Get friend requests from inbox (relationship_request_received)
 */
export async function getFriendRequests(): Promise<Message[]> {
  const currentUser = get(user);
  if (!currentUser?.home) return [];
  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const inboxPath = `${homePath}/inbox`;
  
  const sql = `
    SELECT id, path, name, node_type, properties
    FROM '${ACCESS_CONTROL}'
    WHERE DESCENDANT_OF('${inboxPath}')
      AND node_type = 'raisin:Message'
      AND properties->>'message_type'::STRING = 'relationship_request_received'
      AND properties->>'relation_type'::STRING = 'FRIENDS_WITH'
      AND properties->>'status'::STRING NOT IN ('accepted', 'declined')
    ORDER BY properties->>'received_at' DESC
  `;
  return await query<Message>(sql).catch(() => []);
}

export async function areFriends(friendPath: string): Promise<boolean> {
    const currentUser = get(user);
    if (!currentUser?.home) return false;
    const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
    try {
        const sql = `SELECT COUNT(*) as count FROM GRAPH_TABLE(MATCH (me)-[:FRIENDS_WITH]->(friend) WHERE me.path = '${homePath}' AND friend.path = '${friendPath}' COLUMNS (friend.id)) AS g`;
        const result = await queryOne<{count: number}>(sql);
        return (result?.count ?? 0) > 0;
    } catch { return false; }
}

export async function getFriendsOfFriends(): Promise<FriendSuggestion[]> {
  const currentUser = get(user);
  if (!currentUser?.home) return [];
  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  try {
    const sql = `
      SELECT DISTINCT * FROM GRAPH_TABLE(
        MATCH (me)-[:FRIENDS_WITH]->(friend)-[:FRIENDS_WITH]->(fof)
        WHERE me.path = '${homePath}'
          AND fof.path <> '${homePath}'
        COLUMNS (
          fof.id AS id,
          fof.path AS path,
          fof.properties AS properties
        )
      ) AS g
      WHERE g.path NOT IN (
        SELECT * FROM GRAPH_TABLE(
          MATCH (me2)-[:FRIENDS_WITH]->(already)
          WHERE me2.path = '${homePath}'
          COLUMNS (already.path)
        ) AS existing
      )
      LIMIT 10
    `;
    return await query<FriendSuggestion>(sql);
  } catch { return []; }
}

export async function respondToFriendRequest(
  originalMessage: Message,
  accept: boolean
): Promise<{ success: boolean; error?: string }> {
  const currentUser = get(user);
  if (!currentUser?.home) return { success: false, error: 'Not logged in' };

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');
  const outboxPath = `${homePath}/outbox`;
  const originalRequestId = originalMessage.properties.original_request_id || originalMessage.properties.body?.original_request_id;

  if (!originalRequestId) {
    return { success: false, error: 'Missing original request reference' };
  }

  try {
    const messageName = `friend-resp-${Date.now()}`;
    const messagePath = `${outboxPath}/${messageName}`;
    const properties = {
      message_type: 'relationship_response',
      subject: accept ? 'Friend Request Accepted' : 'Friend Request Declined',
      status: 'pending',
      accepted: accept,
      original_request_id: originalRequestId,
      body: {
        response: accept ? 'accepted' : 'declined',
        original_request_id: originalRequestId
      },
      sender_id: currentUser.id,
      created_at: new Date().toISOString()
    };

    await query(`INSERT INTO '${ACCESS_CONTROL}' (path, node_type, properties) VALUES ($1, 'raisin:Message', $2::jsonb)`,
        [messagePath, JSON.stringify(properties)]);
    await markAsRead(originalMessage.path);
    return { success: true };
  } catch (err) {
    return { success: false, error: (err as any).message };
  }
}

/**
 * Unfriend a user by sending an unfriend message to the outbox.
 * This will be processed by a trigger to remove the FRIENDS_WITH relation.
 */
export async function unfriend(
  friendPath: string
): Promise<{ success: boolean; error?: string }> {
  const currentUser = get(user);
  if (!currentUser?.home) return { success: false, error: 'Not logged in' };

  const homePath = currentUser.home.replace(`/${ACCESS_CONTROL}`, '');

  try {
    await query(`
      UNRELATE
        FROM path='${homePath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TO path='${friendPath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TYPE 'FRIENDS_WITH'
    `);

    await query(`
      UNRELATE
        FROM path='${friendPath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TO path='${homePath}' IN WORKSPACE '${ACCESS_CONTROL}'
        TYPE 'FRIENDS_WITH'
    `);

    return { success: true };
  } catch (err) {
    console.error('[messaging] Failed to unfriend:', err);
    return { success: false, error: (err as any).message };
  }
}
