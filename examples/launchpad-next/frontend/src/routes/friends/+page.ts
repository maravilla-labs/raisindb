import type { PageLoad } from './$types';
import { query } from '$lib/raisin';

const ACCESS_CONTROL = 'raisin:access_control';

// ---------------------------------------------------------------------------
// Types (previously in messaging-utils.ts)
// ---------------------------------------------------------------------------

export interface Message {
  id: string;
  path: string;
  name: string;
  node_type: string;
  properties: {
    message_type: string;
    subject?: string;
    sender_id?: string;
    recipient_id?: string;
    sender_display_name?: string;
    relation_type?: string;
    message?: string;
    accepted?: boolean;
    original_request_id?: string;
    received_at?: string;
    body?: {
      message?: string;
      response?: 'accepted' | 'declined';
      original_request_id?: string;
    };
    status: string;
    created_at?: string;
    error?: string;
  };
}

export interface Friend {
  id: string;
  path: string;
  identity_id: string;
  properties: {
    email: string;
    display_name?: string;
  };
}

export interface FriendSuggestion extends Friend {
  degree?: number;
  hasPendingRequest?: boolean;
}

// ---------------------------------------------------------------------------
// SQL Queries
// ---------------------------------------------------------------------------

const FRIENDS_SQL = (homePath: string) => `SELECT * FROM GRAPH_TABLE(
  MATCH (me)-[:FRIENDS_WITH]->(friend)
  WHERE me.path = '${homePath}'
  COLUMNS (
    friend.id AS id,
    friend.path AS path,
    friend.properties AS properties
  )
) AS g`;

const REQUESTS_SQL = (inboxPath: string) => `SELECT id, path, name, node_type, properties
FROM '${ACCESS_CONTROL}'
WHERE DESCENDANT_OF('${inboxPath}')
  AND node_type = 'raisin:Message'
  AND properties->>'message_type'::STRING = 'relationship_request_received'
  AND properties->>'relation_type'::STRING = 'FRIENDS_WITH'
  AND properties->>'status'::STRING IN ('pending', 'delivered')
ORDER BY properties->>'received_at' DESC`;

const SUGGESTIONS_SQL = (homePath: string) => `SELECT DISTINCT * FROM GRAPH_TABLE(
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
LIMIT 10`;

const SENT_REQUESTS_SQL = (outboxPath: string, homePath: string) => `SELECT id, path, name, node_type, properties
FROM '${ACCESS_CONTROL}'
WHERE (DESCENDANT_OF('${outboxPath}') OR DESCENDANT_OF('${homePath}/sent'))
  AND node_type = 'raisin:Message'
  AND properties->>'message_type'::STRING = 'relationship_request'
  AND properties->>'relation_type'::STRING = 'FRIENDS_WITH'
ORDER BY properties->>'created_at' DESC`;

export const load: PageLoad = async ({ parent }) => {
  const { user } = await parent();

  if (!user?.home) {
    return {
      friends: [] as Friend[],
      requests: [] as Message[],
      suggestions: [] as FriendSuggestion[],
      sentRequests: [] as Message[],
      queries: null,
      error: 'Not logged in',
    };
  }

  const homePath = user.home.replace(`/${ACCESS_CONTROL}`, '');
  const inboxPath = `${homePath}/inbox`;
  const outboxPath = `${homePath}/outbox`;

  try {
    const [friends, requests, suggestions, sentRequests] = await Promise.all([
      query<Friend>(FRIENDS_SQL(homePath)).catch(() => [] as Friend[]),
      query<Message>(REQUESTS_SQL(inboxPath)),
      query<FriendSuggestion>(SUGGESTIONS_SQL(homePath)).catch(() => [] as FriendSuggestion[]),
      query<Message>(SENT_REQUESTS_SQL(outboxPath, homePath)),
    ]);

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
      queries: {
        friends: FRIENDS_SQL(homePath),
        requests: REQUESTS_SQL(inboxPath),
        suggestions: SUGGESTIONS_SQL(homePath),
        sentRequests: SENT_REQUESTS_SQL(outboxPath, homePath),
      },
      error: null,
    };
  } catch (error) {
    console.error('[friends] Load error:', error);
    return {
      friends: [] as Friend[],
      requests: [] as Message[],
      suggestions: [] as FriendSuggestion[],
      sentRequests: [] as Message[],
      queries: null,
      error: error instanceof Error ? error.message : 'Failed to load data',
    };
  }
};
