import { useEffect, useState, useCallback } from 'react';
import { useRaisinClient } from './useRaisinClient';
import { getConfig } from '../lib/raisin';

interface UseLiveQueryOptions {
  workspace?: string;
  nodeType?: string;
  path?: string;
  eventTypes?: string[];
  enabled?: boolean;
  joinAuthor?: boolean; // If true, will join author information for posts
  queryType?: 'sql' | 'cypher';
  cypherQuery?: string;
  cypherParams?: any[];
}

export function useLiveQuery<T = any>(options: UseLiveQueryOptions = {}) {
  const { client } = useRaisinClient();
  const [data, setData] = useState<T[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  const config = getConfig();
  const workspace = options.workspace || config.workspace;
  const enabled = options.enabled !== false;

  // Fetch a single node by ID (for real-time updates)
  const fetchSingleNode = useCallback(async (nodeId: string): Promise<T | null> => {
    if (!client) return null;

    try {
      const db = client.database(config.repository);

      let query: string;
      const params: unknown[] = [nodeId];

      if (options.queryType === 'cypher') {
          // For Cypher, we need a specific query to fetch single node
          // This is tricky because the main query might be complex.
          // For now, fallback to SQL for single node fetch or use a simple MATCH
          query = `MATCH (n) WHERE n.id = $1 RETURN n`;
      } else if (options.joinAuthor && options.nodeType === 'Post') {
        // Join with author information for posts
        query = `SELECT
          post.*,
          author.properties ->> 'username' as authorName,
          author.properties ->> 'displayName' as authorDisplayName,
          author.properties ->> 'avatar' as authorAvatar
        FROM ${workspace} post
        LEFT JOIN ${workspace} author ON post.properties ->> 'authorId' = author.id
        WHERE post.id = $1 AND author.properties IS NOT NULL AND post.node_type = 'Post'
        ORDER BY post.created_at
        `;
      } else {
        // Standard query without joins
        query = `SELECT * FROM ${workspace} WHERE id = $1 `;

        if (options.nodeType) {
          query += ` AND node_type = $2`;
          params.push(options.nodeType);
        }
      }
      console.log('Fetching single node with query:', query, 'params:', params);

      const result = await db.executeSql(query, params);
      console.log('Fetched single node for ID', nodeId, ':', result);
      if (result.rows.length > 0) {
        // Handle Cypher result where first column might be the node object
        if (options.queryType === 'cypher') {
            const firstCol = result.rows[0][0] as any;
            if (firstCol && typeof firstCol === 'object') {
                return firstCol as T;
            }
        }
        
        return {
          id: result.rows[0][0],
          ...result.rows[0],
        } as T;
      }
      return null;
    } catch (err) {
      console.error('Failed to fetch single node:', err);
      return null;
    }
  }, [client, config.repository, workspace, options.nodeType, options.joinAuthor, options.queryType]);

  // Fetch initial data
  const fetchData = useCallback(async () => {
    if (!client || !enabled) return;

    setLoading(true);
    setError(null);

    try {
      const db = client.database(config.repository);

      // Build query based on options
      let query: string;
      const params: unknown[] = [];

      if (options.queryType === 'cypher' && options.cypherQuery) {
          query = options.cypherQuery;
          if (options.cypherParams) {
              params.push(...options.cypherParams);
          }
      } else if (options.joinAuthor && options.nodeType === 'Post') {
        // Join with author information for posts
        query = `SELECT
          post.*,
          author.properties ->> 'username' as authorName,
          author.properties ->> 'displayName' as authorDisplayName,
          author.properties ->> 'avatar' as authorAvatar
        FROM ${workspace} post
        LEFT JOIN ${workspace} author ON post.properties ->> 'authorId' = author.id
        WHERE post.node_type = 'Post' AND author.properties IS NOT NULL         
        `;
        

        if (options.path) {
          query += ` AND post.path LIKE $${params.length + 1}`;
          params.push(options.path);
        }

        query += ' ORDER BY post.created_at DESC LIMIT 10';
      } else {
        // Standard query without joins
        const conditions: string[] = [];

        if (options.nodeType) {
          conditions.push(`node_type = $${params.length + 1}`);
          params.push(options.nodeType);
        }

        if (options.path) {
          conditions.push(`path LIKE $${params.length + 1}`);
          params.push(options.path);
        }

        query = `SELECT * FROM ${workspace}`;
        if (conditions.length > 0) {
          query += ' WHERE ' + conditions.join(' AND ');
        }
        query += ' ORDER BY created_at DESC LIMIT 10';
      }

      console.log('Executing live query:', query, 'with params:', params);
      const result = await db.executeSql(query, params);
      console.log('Live query result:', result);
      
      let nodes: T[];
      if (options.queryType === 'cypher') {
          // For Cypher, we expect the first column to be the node, and subsequent columns to be joined data
          // We merge them into a single object
          nodes = result.rows.map(row => {
              const node = row[0] as any;
              const extras: any = {};
              
              // Map extra columns if available (based on result.columns if we had them, but here we rely on order)
              // For the specific query in Feed.tsx: RETURN p, u.username, u.displayName, u.avatar
              if (row.length > 1) {
                  // This is specific to the Feed query, ideally we'd use column names
                  // But executeSql result has columns array!
                  result.columns.slice(1).forEach((colName, idx) => {
                      extras[colName] = row[idx + 1];
                  });
              }
              
              return {
                  ...node,
                  ...extras
              };
          });
      } else {
          nodes = result.rows.map(row => ({
            id: row[0],
            ...row,
          })) as T[];
      }

      setData(nodes);
    } catch (err) {
      setError(err as Error);
    } finally {
      setLoading(false);
    }
  }, [client, config.repository, workspace, options.nodeType, options.path, options.joinAuthor, enabled, options.queryType, options.cypherQuery]);


  // Subscribe to live updates
  useEffect(() => {
    if (!client || !enabled) return;

    // Track if cleanup has been called to prevent race conditions with async operations
    let cleanedUp = false;
    let subscription: any = null;

    fetchData();

    // Subscribe to real-time events
    const setupSubscription = async () => {
      try {
        const db = client.database(config.repository);
        const eventHandler = db.events();

        // Build subscription filters
        const filters: any = {
          workspace,
        };

        if (options.nodeType) {
          filters.node_type = options.nodeType;
        }

        if (options.path) {
          filters.path = options.path;
        }

        if (options.eventTypes) {
          filters.event_types = options.eventTypes;
        }

        console.log('🔌 Subscribing to events with filters:', JSON.stringify(filters, null, 2));

        // Subscribe to events
        subscription = await eventHandler.subscribe(filters, async (event: any) => {
          // Prevent processing events if component has been cleaned up
          if (cleanedUp) {
            console.log('⚠️ Event received after cleanup, ignoring:', event.event_type);
            return;
          }

          console.log('📬 Received event:', event);

          // Handle events intelligently to avoid full re-renders
          // Server sends event.payload with metadata (node_id, etc.)
          if (event.event_type === 'node:created' && event.payload?.node_id) {
            console.log('🆕 New node created, fetching:', event.payload.node_id);
            // Fetch the new node with full data (including JOINs)
            const newNode = await fetchSingleNode(event.payload.node_id);
            if (newNode && !cleanedUp) {
              setData(prev => {
                // Avoid duplicates
                if (prev.some(item => (item as any).id === event.payload.node_id)) {
                  console.log('⚠️ Duplicate node, skipping:', event.payload.node_id);
                  return prev;
                }
                console.log('✅ Adding new node to feed:', event.payload.node_id);
                return [newNode, ...prev];
              });
            }
          } else if (event.event_type === 'node:updated' && event.payload?.node_id) {
            console.log('🔄 Node updated, fetching:', event.payload.node_id);
            // Fetch the updated node with full data (including JOINs)
            const updatedNode = await fetchSingleNode(event.payload.node_id);
            if (updatedNode && !cleanedUp) {
              setData(prev => {
                const updated = prev.map(item =>
                  (item as any).id === event.payload.node_id ? updatedNode : item
                );
                console.log('✅ Updated node in feed:', event.payload.node_id);
                return updated;
              });
            }
          } else if (event.event_type === 'node:deleted' && event.payload?.node_id) {
            console.log('🗑️ Node deleted, removing:', event.payload.node_id);
            // Remove deleted node from the list
            if (!cleanedUp) {
              setData(prev => {
                const filtered = prev.filter(item => (item as any).id !== event.payload.node_id);
                console.log('✅ Removed node from feed:', event.payload.node_id);
                return filtered;
              });
            }
          } else if (
            (event.event_type === 'node:relation_added' ||
              event.event_type === 'node:relation_removed') &&
            event.payload?.node_id
          ) {
            const relationType = (event.payload as any).relation_type;
            const nodeTypeFromEvent = (event.payload as any).node_type;
            const relatedNodeId =
              (event.payload as any).related_node_id ||
              (event.payload as any).target_node_id;

            const shouldHandleForNodeType =
              !options.nodeType ||
              options.nodeType === nodeTypeFromEvent ||
              (options.nodeType === 'Post' && relationType === 'likes');

            if (!shouldHandleForNodeType) {
              return;
            }

            const nodeIdToFetch =
              options.nodeType === 'Post' && relationType === 'likes'
                ? event.payload.node_id || relatedNodeId
                : event.payload.node_id;

            if (nodeIdToFetch) {
              console.log('🔄 Relation change affecting node, refetching:', nodeIdToFetch);
              const updatedNode = await fetchSingleNode(nodeIdToFetch);
              if (updatedNode && !cleanedUp) {
                setData(prev => {
                  let updated = false;
                  const next = prev.map(item => {
                    if ((item as any).id === nodeIdToFetch) {
                      updated = true;
                      return updatedNode;
                    }
                    return item;
                  });
                  if (updated) {
                    console.log('✅ Updated node after relationship change:', nodeIdToFetch);
                    return next;
                  }
                  return prev;
                });
              }
            }
          } else {
            // Fallback: refetch for unknown event types or if payload is missing
            console.log('⚠️ Unknown event type or missing payload, refetching...', event);
            if (!cleanedUp) {
              fetchData();
            }
          }
        });

        // Check if cleanup happened while we were setting up the subscription
        if (cleanedUp) {
          console.log('🔌 Cleanup occurred during subscription setup, unsubscribing immediately');
          if (subscription && subscription.isActive()) {
            subscription.unsubscribe();
          }
          return;
        }

        console.log('✅ Subscribed to events with ID:', subscription.id, 'filters:', JSON.stringify(filters, null, 2));
      } catch (err) {
        console.error('Failed to subscribe to events:', err);
      }
    };

    setupSubscription();

    return () => {
      // Mark as cleaned up to prevent any pending async operations from updating state
      cleanedUp = true;

      // Unsubscribe when component unmounts
      if (subscription && subscription.isActive()) {
        console.log('🔌 Unsubscribing from events:', subscription.id);
        subscription.unsubscribe();
      }
    };
  }, [client, workspace, options.nodeType, options.path, options.eventTypes, enabled, config.repository]);

  const refetch = useCallback(() => {
    fetchData();
  }, [fetchData]);

  return {
    data,
    loading,
    error,
    refetch,
  };
}
