import type { PageLoad } from './$types';
import { query } from '$lib/raisin';
import type { Message, Friend, FriendSuggestion } from '$lib/stores/messaging';

const ACCESS_CONTROL = 'raisin:access_control';

export const load: PageLoad = async ({ parent }) => {
  const { user } = await parent();

  if (!user?.home) {
    return {
      friends: [] as Friend[],
      requests: [] as Message[],
      suggestions: [] as FriendSuggestion[],
      sentRequests: [] as Message[],
      error: 'Not logged in',
    };
  }

  const homePath = user.home.replace(`/${ACCESS_CONTROL}`, '');
  const inboxPath = `${homePath}/inbox`;
  const outboxPath = `${homePath}/outbox`;

  try {
    const [friends, requests, suggestions, sentRequests] = await Promise.all([
      // Get friends using GRAPH_TABLE
      query<Friend>(`
        SELECT * FROM GRAPH_TABLE(
          MATCH (me)-[:FRIENDS_WITH]->(friend)
          WHERE me.path = '${homePath}'
          COLUMNS (
            friend.id AS id,
            friend.path AS path,
            friend.properties AS properties
          )
        ) AS g
      `).catch(() => [] as Friend[]),

      // Get pending friend requests from inbox
      query<Message>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL}'
        WHERE DESCENDANT_OF('${inboxPath}')
          AND node_type = 'raisin:Message'
          AND properties->>'message_type'::STRING = 'relationship_request_received'
          AND properties->>'relation_type'::STRING = 'FRIENDS_WITH'
          AND properties->>'status'::STRING IN ('pending', 'delivered')
        ORDER BY properties->>'received_at' DESC
      `),

      // Get friend suggestions (2nd and 3rd degree) with path length using CARDINALITY
      query<FriendSuggestion>(`
        SELECT DISTINCT * FROM GRAPH_TABLE(
          MATCH (me)-[r:FRIENDS_WITH*2..3]->(fof)
          WHERE me.path = '${homePath}'
            AND fof.path <> '${homePath}'
          COLUMNS (
            fof.id AS id,
            fof.path AS path,
            fof.properties AS properties,
            CARDINALITY(r) AS degree
          )
        ) AS g
        ORDER BY degree ASC
        LIMIT 10
      `).catch(() => [] as FriendSuggestion[]),

      // Get sent friend requests from outbox and sent folder
      query<Message>(`
        SELECT id, path, name, node_type, properties
        FROM '${ACCESS_CONTROL}'
        WHERE (DESCENDANT_OF('${outboxPath}') OR DESCENDANT_OF('${homePath}/sent'))
          AND node_type = 'raisin:Message'
          AND properties->>'message_type'::STRING = 'relationship_request'
          AND properties->>'relation_type'::STRING = 'FRIENDS_WITH'
        ORDER BY properties->>'created_at' DESC
      `),
    ]);

    // Filter out suggestions who are already friends,
    // mark those with pending requests, and deduplicate by ID
    const friendPaths = new Set(friends.map(f => f.path));
    const sentRequestIds = new Set(
      sentRequests.map(r => r.properties.recipient_id).filter(Boolean)
    );
    const seenIds = new Set<string>();
    const filteredSuggestions = suggestions
      .filter(s => {
        if (friendPaths.has(s.path)) return false;
        if (seenIds.has(s.id)) return false;
        seenIds.add(s.id);
        return true;
      })
      .map(s => ({
        ...s,
        hasPendingRequest: sentRequestIds.has(s.id)
      }));

    return {
      friends,
      requests,
      suggestions: filteredSuggestions,
      sentRequests,
      error: null,
    };
  } catch (error) {
    console.error('[friends] Load error:', error);
    return {
      friends: [] as Friend[],
      requests: [] as Message[],
      suggestions: [] as FriendSuggestion[],
      sentRequests: [] as Message[],
      error: error instanceof Error ? error.message : 'Failed to load data',
    };
  }
};
