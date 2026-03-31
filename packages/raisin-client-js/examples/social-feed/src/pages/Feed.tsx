import { useState, useEffect, useRef } from 'react';
import { AnimatePresence } from 'framer-motion';
import { useLiveQuery } from '../hooks/useLiveQuery';
import CreatePost from '../components/CreatePost';
import PostCard from '../components/PostCard';
import type { Post, QueryMode } from '../lib/types';

export default function Feed() {
  const [queryMode, setQueryMode] = useState<QueryMode>('sql');
  const { data: posts, loading } = useLiveQuery<Post>({
    nodeType: 'Post',
    path: '/posts/%',
    joinAuthor: true,
    queryType: queryMode,
    cypherQuery: queryMode === 'cypher' ? `
      MATCH (p:Post)
      WHERE p.path STARTS WITH '/users/'
      MATCH (u:SocialUser)-[:AUTHORED]->(p)
      RETURN p, u.username as authorName, u.displayName as authorDisplayName, u.avatar as authorAvatar
      ORDER BY p.created_at DESC
      LIMIT 10
    ` : undefined
  });

  // Track updated posts
  const [updatedPosts, setUpdatedPosts] = useState<Set<string>>(new Set());
  const previousPostsRef = useRef<Map<string, Post>>(new Map());

  // Detect changes in posts
  useEffect(() => {
    if (posts.length === 0) return;

    const newUpdatedPosts = new Set<string>();
    const previousPosts = previousPostsRef.current;

    posts.forEach((post) => {
      const prevPost = previousPosts.get(post.id);
      if (prevPost) {
        // Check if any relevant properties changed
        const contentChanged = prevPost.properties.content !== post.properties.content;
        const likeCountChanged = prevPost.properties.likeCount !== post.properties.likeCount;
        const commentCountChanged = prevPost.commentCount !== post.commentCount;

        if (contentChanged || likeCountChanged || commentCountChanged) {
          newUpdatedPosts.add(post.id);
        }
      }
    });

    if (newUpdatedPosts.size > 0) {
      setUpdatedPosts(newUpdatedPosts);
      // Clear the updated state after animation duration
      setTimeout(() => {
        setUpdatedPosts(new Set());
      }, 1000);
    }

    // Update the reference
    const newPostsMap = new Map(posts.map((p) => [p.id, p]));
    previousPostsRef.current = newPostsMap;
  }, [posts]);

  return (
    <div className="max-w-2xl mx-auto">
      <div className="mb-6">
        <h1 className="text-3xl font-bold mb-2">Social Feed</h1>
        <p className="text-gray-600">
          Real-time updates powered by RaisinDB
        </p>
      </div>

      {/* Query Mode Toggle */}
      <div className="card mb-6">
        <div className="flex items-center justify-between">
          <span className="font-medium">Query Mode:</span>
          <div className="flex space-x-2">
            <button
              onClick={() => setQueryMode('sql')}
              className={queryMode === 'sql' ? 'btn-primary' : 'btn-secondary'}
            >
              SQL
            </button>
            <button
              onClick={() => setQueryMode('cypher')}
              className={queryMode === 'cypher' ? 'btn-primary' : 'btn-secondary'}
            >
              Cypher
            </button>
          </div>
        </div>
        <div className="mt-3 p-3 bg-gray-50 rounded text-sm font-mono">
          {queryMode === 'sql' ? (
            <div>
              <div className="text-gray-600 mb-1">Current SQL Query:</div>
              <code>SELECT * FROM social WHERE node_type = 'Post' ORDER BY created_at DESC</code>
            </div>
          ) : (
            <div>
              <div className="text-gray-600 mb-1">Example Cypher Query:</div>
              <code>MATCH (p:Post) WHERE p.path STARTS WITH '/users/' MATCH (u:SocialUser)-[:AUTHORED]-&gt;(p) RETURN p...</code>
              <div className="mt-2 text-xs text-gray-500">
                (Uses Path-First Traversal and Graph Relations)
              </div>
            </div>
          )}
        </div>
      </div>

      <CreatePost />

      {loading ? (
        <div className="text-center py-8">
          <div className="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
        </div>
      ) : posts.length === 0 ? (
        <div className="card text-center py-8">
          <p className="text-gray-500">
            No posts yet. Be the first to post!
          </p>
        </div>
      ) : (
        <div>
          <div className="mb-4 text-sm text-gray-600">
            {posts.length} post{posts.length !== 1 ? 's' : ''} • Live updates enabled
          </div>
          <AnimatePresence mode="popLayout" initial={false}>
            {posts.map((post) => (
              <PostCard
                key={post.id}
                post={post}
                isUpdated={updatedPosts.has(post.id)}
              />
            ))}
          </AnimatePresence>
        </div>
      )}
    </div>
  );
}
