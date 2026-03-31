import { useState } from 'react';
import { useHybridClient } from '~/hooks/useHybridClient';
import { REPOSITORY, WORKSPACE } from '~/lib/config';

interface CreatePostProps {
  onPostCreated?: () => void;
}

export default function CreatePost({ onPostCreated }: CreatePostProps) {
  const { wsClient, isRealtime } = useHybridClient();
  const [content, setContent] = useState('');
  const [loading, setLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!content.trim() || !wsClient || !isRealtime) return;

    setLoading(true);
    try {
      const db = wsClient.database(REPOSITORY);
      const ws = db.workspace(WORKSPACE);

      await ws.nodes().create({
        type: 'Post',
        path: `/posts/post_${Date.now()}`,
        properties: {
          content: content.trim(),
          authorId: '7be99b2d-a9ad-401e-81c1-011df009f800',
          likeCount: 0,
          commentCount: 0,
        },
      });

      setContent('');
      onPostCreated?.();
    } catch (error) {
      console.error('Failed to create post:', error);
      alert('Failed to create post');
    } finally {
      setLoading(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="glass glass-border rounded-lg p-6 mb-6">
      <textarea
        className="w-full px-4 py-3 border border-gray-300 dark:border-gray-600 rounded-lg focus:outline-none focus:ring-2 focus:ring-blue-500 bg-white dark:bg-gray-800 text-gray-900 dark:text-gray-100 mb-3"
        placeholder={isRealtime ? "What's on your mind?" : "Connect to WebSocket to post..."}
        value={content}
        onChange={(e) => setContent(e.target.value)}
        maxLength={280}
        rows={3}
        disabled={loading || !isRealtime}
      />
      <div className="flex justify-between items-center">
        <span className="text-sm text-gray-500 dark:text-gray-400">
          {content.length}/280
        </span>
        <button
          type="submit"
          className="px-4 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg font-medium disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          disabled={!content.trim() || loading || !isRealtime}
        >
          {loading ? 'Posting...' : 'Post'}
        </button>
      </div>
      {!isRealtime && (
        <p className="mt-2 text-sm text-yellow-600 dark:text-yellow-400">
          Connecting to real-time updates...
        </p>
      )}
    </form>
  );
}
