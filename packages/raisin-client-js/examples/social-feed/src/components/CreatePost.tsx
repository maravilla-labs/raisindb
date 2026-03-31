import { useState, useEffect, useCallback } from 'react';
import { useRaisinClient } from '../hooks/useRaisinClient';
import { getConfig, getClient } from '../lib/raisin';

interface CreatePostProps {
  onPostCreated?: () => void;
}

interface User {
  id: string;
  displayName: string;
  avatar?: string;
  path: string;
}

export default function CreatePost({ onPostCreated }: CreatePostProps) {
  const { client } = useRaisinClient();
  const [content, setContent] = useState('');
  const [loading, setLoading] = useState(false);
  const [users, setUsers] = useState<User[]>([]);
  const [selectedUserId, setSelectedUserId] = useState<string>('7be99b2d-a9ad-401e-81c1-011df009f800');
  const [loadingUsers, setLoadingUsers] = useState(true);

  // Fetch all users on component mount
  const fetchUsers = useCallback(async () => {
    try {
      setLoadingUsers(true);
      const config = getConfig();
      const client = await getClient();
      const db = client.database(config.repository);

      // Query all SocialUsers with their paths
      const query = `
        SELECT
          id,
          properties ->> 'displayName' AS displayName,
          properties ->> 'avatar' AS avatar,
          path
        FROM ${config.workspace}
        WHERE node_type = 'SocialUser' AND path NOT LIKE ''
        ORDER BY name
      `;

      const result = await db.executeSql(query);

      const allUsers: User[] = result.rows.map((row: any) => ({
        id: row.id || row[0],
        displayName: String(row.displayName || row.name || 'Unknown'),
        avatar: row.avatar ? String(row.avatar) : undefined,
        path: String(row.path || ''),
      }));

      setUsers(allUsers);

      // Set the first user as default if no user is selected
      if (allUsers.length > 0 && !selectedUserId) {
        setSelectedUserId(allUsers[0].id);
      }
    } catch (error) {
      console.error('Failed to fetch users:', error);
    } finally {
      setLoadingUsers(false);
    }
  }, [selectedUserId]);

  useEffect(() => {
    fetchUsers();
  }, [fetchUsers]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!content.trim() || !client) return;

    setLoading(true);
    try {
      const config = getConfig();
      const db = client.database(config.repository);
      const ws = db.workspace(config.workspace);
      
      await ws.nodes().create({
        type: 'Post',
        path: `/posts/post_${Date.now()}`,
        properties: {
          content: content.trim(),
          authorId: selectedUserId,
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

  const selectedUser = users.find(u => u.id === selectedUserId);

  return (
    <form onSubmit={handleSubmit} className="card mb-6">
      <textarea
        className="textarea mb-3"
        placeholder="What's on your mind?"
        value={content}
        onChange={(e) => setContent(e.target.value)}
        maxLength={280}
        rows={3}
        disabled={loading}
      />
      <div className="flex justify-between items-center gap-3">
        <span className="text-sm text-gray-500">
          {content.length}/280
        </span>

        <div className="flex items-center gap-3">
          {/* User Selector Dropdown */}
          <div className="flex items-center gap-2">
            <span className="text-sm text-gray-400">Post as:</span>
            <select
              value={selectedUserId}
              onChange={(e) => setSelectedUserId(e.target.value)}
              disabled={loading || loadingUsers}
              className="px-3 py-2 bg-white/5 border border-white/10 rounded-lg text-white text-sm focus:outline-none focus:border-white/30 transition-colors"
            >
              {loadingUsers ? (
                <option>Loading users...</option>
              ) : users.length === 0 ? (
                <option>No users available</option>
              ) : (
                users.map((user) => (
                  <option key={user.id} value={user.id}>
                    {user.avatar ? `${user.avatar} ` : ''}{user.displayName}
                  </option>
                ))
              )}
            </select>
          </div>

          <button
            type="submit"
            className="btn-primary"
            disabled={!content.trim() || loading || !selectedUserId}
          >
            {loading ? 'Posting...' : 'Post'}
          </button>
        </div>
      </div>
    </form>
  );
}
