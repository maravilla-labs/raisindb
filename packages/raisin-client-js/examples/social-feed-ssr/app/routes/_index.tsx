/**
 * Feed Route with SSR Support
 *
 * This route demonstrates:
 * 1. Server-side data fetching via HTTP (loader)
 * 2. Client-side hydration with initial data
 * 3. Automatic upgrade to WebSocket for real-time updates
 */

import { useState, useEffect } from 'react';
import { useLoaderData } from 'react-router';
import type { Route } from './+types/_index';
import { createLoader, rowsToObjects } from '@raisindb/client';
import { getRaisinConfig, REPOSITORY, WORKSPACE } from '~/lib/config';
import type { Post } from '~/lib/types';
import PostCard from '~/components/PostCard';
import CreatePost from '~/components/CreatePost';
import LiveIndicator from '~/components/LiveIndicator';
import { useHybridClient } from '~/hooks/useHybridClient';

// Meta tags for SEO
export function meta() {
  return [
    { title: 'Social Feed - RaisinDB SSR Example' },
    { name: 'description', content: 'A server-side rendered social feed powered by RaisinDB' },
  ];
}

// Server-side data loader
export const loader = createLoader(
  getRaisinConfig(),
  async (client) => {
    try {
      const db = client.database(REPOSITORY);

      // Fetch posts with author information via SQL join (same query as social-feed example)
      const result = await db.executeSql(`
        SELECT
          post.*,
          author.properties ->> 'username' as authorName,
          author.properties ->> 'displayName' as authorDisplayName,
          author.properties ->> 'avatar' as authorAvatar
        FROM ${WORKSPACE} post
        LEFT JOIN ${WORKSPACE} author ON post.properties ->> 'authorId' = author.id
        WHERE post.id != '0f1e6144-096a-4eac-9839-33ec3bffd2cd'
          AND post.node_type = 'Post'
        ORDER BY post.created_at DESC
        LIMIT 50
      `);

      const posts = rowsToObjects<Post>(result.columns, result.rows);

      return {
        posts,
        ssrTimestamp: new Date().toISOString(),
      };
    } catch (error) {
      console.error('[Feed Loader] Error fetching posts:', error);
      return {
        posts: [],
        ssrTimestamp: new Date().toISOString(),
        error: error instanceof Error ? error.message : 'Failed to load posts',
      };
    }
  }
);

export default function Feed({ loaderData }: Route.ComponentProps) {
  const initialData = loaderData as Awaited<ReturnType<typeof loader>>;
  const [posts, setPosts] = useState<Post[]>(initialData.posts);
  const [updatedPostIds, setUpdatedPostIds] = useState<Set<string>>(new Set());
  const { wsClient, isRealtime } = useHybridClient();

  // Subscribe to real-time updates after WebSocket connection
  useEffect(() => {
    if (!isRealtime || !wsClient) {
      return;
    }

    console.log('[Feed] Setting up real-time subscriptions...');

    const db = wsClient.database(REPOSITORY);
    const eventHandler = db.events();

    // Subscribe to Post events
    const subscription = eventHandler.subscribe(
      {
        workspace: WORKSPACE,
        node_type: 'Post',
      },
      (event) => {
        console.log('[Feed] Real-time event received:', event.event_type, event.payload);

        if (event.event_type === 'node:created') {
          // Fetch the full post data with author info
          db.executeSql(`
            SELECT
              p.id,
              p.name,
              p.path,
              p.properties->>'content' as content,
              p.properties->>'authorId' as authorId,
              CAST(p.properties->>'likeCount' as INTEGER) as likeCount,
              CAST(p.properties->>'commentCount' as INTEGER) as commentCount,
              p.created_at as createdAt,
              p.updated_at as updatedAt,
              u.properties->>'username' as authorName,
              u.properties->>'displayName' as authorDisplayName,
              u.properties->>'avatar' as authorAvatar
            FROM social p
            LEFT JOIN social u ON u.id = (p.properties->>'authorId')
            WHERE p.id = $1
          `, [(event.payload as any).node_id])
            .then((result) => {
              console.log('[Feed] New post fetched:', result);
              const newPosts = rowsToObjects<Post>(result.columns, result.rows);
              if (newPosts.length > 0) {
                setPosts((prev) => [newPosts[0], ...prev]);
              }
            })
            .catch((err) => console.error('[Feed] Failed to fetch new post:', err));
        } else if (event.event_type === 'node:updated') {
          const nodeId = (event.payload as any).node_id;

          // Mark as updated for visual feedback
          setUpdatedPostIds((prev) => new Set(prev).add(nodeId));
          setTimeout(() => {
            setUpdatedPostIds((prev) => {
              const next = new Set(prev);
              next.delete(nodeId);
              return next;
            });
          }, 2000);

          // Fetch updated post data
          db.executeSql(`
            SELECT
              p.id,
              p.name,
              p.path,
              p.properties->>'content' as content,
              p.properties->>'authorId' as authorId,
              CAST(p.properties->>'likeCount' as INTEGER) as likeCount,
              CAST(p.properties->>'commentCount' as INTEGER) as commentCount,
              p.created_at as createdAt,
              p.updated_at as updatedAt,
              u.properties->>'username' as authorName,
              u.properties->>'displayName' as authorDisplayName,
              u.properties->>'avatar' as authorAvatar
            FROM social p
            LEFT JOIN social u ON u.id = (p.properties->>'authorId')
            WHERE p.id = $1
          `, [nodeId])
            .then((result) => {
              const updatedPosts = rowsToObjects<Post>(result.columns, result.rows);
              if (updatedPosts.length > 0) {
                setPosts((prev) =>
                  prev.map((p) => (p.id === nodeId ? updatedPosts[0] : p))
                );
              }
            })
            .catch((err) => console.error('[Feed] Failed to fetch updated post:', err));
        } else if (event.event_type === 'node:deleted') {
          const nodeId = (event.payload as any).node_id;
          setPosts((prev) => prev.filter((p) => p.id !== nodeId));
        }
      }
    );

    return () => {
      console.log('[Feed] Unsubscribing from real-time updates');
      subscription.unsubscribe();
    };
  }, [isRealtime, wsClient]);

  const handleLike = async (postId: string) => {
    if (!wsClient || !isRealtime) return;

    try {
      const db = wsClient.database(REPOSITORY);
      const ws = db.workspace(WORKSPACE);

      // Find the post and increment like count
      const post = posts.find((p) => p.id === postId);
      if (!post) return;

      await ws.nodes().update(postId, {
        properties: {
          ...post.properties,
          likeCount: post.properties.likeCount + 1,
        },
      });
    } catch (error) {
      console.error('[Feed] Failed to like post:', error);
    }
  };

  return (
    <div className="min-h-screen bg-gradient-to-br from-blue-50 via-purple-50 to-pink-50 dark:from-gray-900 dark:via-gray-800 dark:to-gray-900">
      <div className="max-w-2xl mx-auto py-8 px-4">
        {/* Header */}
        <div className="glass glass-border rounded-lg p-6 mb-6 shadow-lg">
          <div className="flex items-center justify-between mb-4">
            <h1 className="text-3xl font-bold text-gray-900 dark:text-gray-100">
              Social Feed (SSR)
            </h1>
            <LiveIndicator />
          </div>
          <p className="text-gray-600 dark:text-gray-400">
            Server-side rendered with React Router 7 + RaisinDB
          </p>
          {initialData.ssrTimestamp && (
            <p className="text-sm text-gray-500 dark:text-gray-500 mt-2">
              Initial render: {new Date(initialData.ssrTimestamp).toLocaleString()}
            </p>
          )}
          {initialData.error && (
            <p className="text-sm text-red-500 mt-2">
              Error: {initialData.error}
            </p>
          )}
        </div>

        {/* Create Post */}
        <CreatePost />

        {/* Posts Feed */}
        <div className="space-y-4">
          {posts.length === 0 ? (
            <div className="glass glass-border rounded-lg p-12 text-center">
              <p className="text-gray-500 dark:text-gray-400 text-lg">
                No posts yet. Be the first to post!
              </p>
            </div>
          ) : (
            posts.map((post) => (
              <PostCard
                key={post.id}
                post={post}
                onLike={handleLike}
                isUpdated={updatedPostIds.has(post.id)}
              />
            ))
          )}
        </div>
      </div>
    </div>
  );
}
