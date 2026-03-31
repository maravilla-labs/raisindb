import { useState, useEffect, useCallback, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { getClient, getConfig } from '../lib/raisin';

interface UserSelectorModalProps {
  isOpen: boolean;
  onClose: () => void;
  postId: string;
  currentLikers: string[];
  onLikeToggled: () => void;
}

interface User {
  id: string;
  displayName: string;
  avatar?: string;
  path: string;
}

export function UserSelectorModal({
  isOpen,
  onClose,
  postId,
  currentLikers,
  onLikeToggled,
}: UserSelectorModalProps) {
  const [users, setUsers] = useState<User[]>([]);
  const [loading, setLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [processingUserId, setProcessingUserId] = useState<string | null>(null);
  const config = getConfig();

  // Fetch all users
  const fetchUsers = useCallback(async () => {
    try {
      setLoading(true);
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

      console.log('Executing user fetch query:', query);
      const result = await db.executeSql(query);
      console.log('User fetch result:', result);

      const allUsers: User[] = result.rows.map((row: any) => ({
        id: row.id || row[0],
        displayName: String(row.displayName || row.name || 'Unknown'),
        avatar: row.avatar ? String(row.avatar) : undefined,
        path: String(row.path || ''),
      }));

      setUsers(allUsers);
    } catch (error) {
      console.error("Workspace", config.workspace);
      console.error('Failed to fetch users:', error);
    } finally {
      setLoading(false);
    }
  }, [config.repository, config.workspace]);

  useEffect(() => {
    if (isOpen) {
      fetchUsers();
      setSearchQuery('');
    }
  }, [isOpen, fetchUsers]);

  // Handle like/unlike toggle
  const handleToggleLike = async (user: User, isLiked: boolean) => {
    if (processingUserId) return;

    try {
      setProcessingUserId(user.id);
      const client = await getClient();
      const db = client.database(config.repository);
      const ws = db.workspace(config.workspace);

      // Resolve the post to find its path via Graph API
      const postNode = await ws.nodes().get(postId);
      if (!postNode?.path) {
        console.error('Post not found or missing path metadata', postId, postNode);
        return;
      }
      const postPath = postNode.path;

      if (isLiked) {
        // Unlike: remove relationship
        console.log('🔓 Removing like relationship:', {
          userPath: user.path,
          postPath,
          relationType: 'likes'
        });
        await ws.nodes().removeRelation(user.path, postPath);
      } else {
        // Like: add relationship
        console.log('❤️ Adding like relationship:', {
          userPath: user.path,
          postPath,
          relationType: 'likes'
        });
        await ws.nodes().addRelation(user.path, 'likes', postPath);
      }

      // Refresh the likes list
      onLikeToggled();
    } catch (error) {
      console.error('Failed to toggle like:', error);
      console.error('User:', user);
      console.error('Post ID:', postId);
    } finally {
      setProcessingUserId(null);
    }
  };

  const filteredUsers = useMemo(() => {
    const trimmed = searchQuery.trim().toLowerCase();
    if (!trimmed) {
      return users;
    }
    return users.filter((user) =>
      (user.displayName || '').toLowerCase().includes(trimmed)
    );
  }, [users, searchQuery]);

  // Handle ESC key
  useEffect(() => {
    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen) {
        onClose();
      }
    };

    window.addEventListener('keydown', handleEsc);
    return () => window.removeEventListener('keydown', handleEsc);
  }, [isOpen, onClose]);

  if (!isOpen) return null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        onClick={onClose}
        className="fixed inset-0 bg-black/50 backdrop-blur-sm flex items-center justify-center z-50 p-4"
      >
        <motion.div
          initial={{ scale: 0.9, opacity: 0 }}
          animate={{ scale: 1, opacity: 1 }}
          exit={{ scale: 0.9, opacity: 0 }}
          onClick={(e) => e.stopPropagation()}
          className="bg-gray-900/95 backdrop-blur-xl border border-white/20 rounded-2xl shadow-2xl w-full max-w-md overflow-hidden"
        >
          {/* Header */}
          <div className="p-6 border-b border-white/10">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold text-white">Select User to Like</h2>
              <button
                onClick={onClose}
                className="text-gray-400 hover:text-white transition-colors"
              >
                <svg className="w-6 h-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>

            {/* Search input */}
            <input
              type="text"
              placeholder="Search users..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="w-full px-4 py-2 bg-white/5 border border-white/10 rounded-lg text-white placeholder-gray-400 focus:outline-none focus:border-white/30 transition-colors"
            />
          </div>

          {/* User list */}
          <div className="p-4 max-h-[400px] overflow-y-auto">
            {loading ? (
              <div className="text-center py-8 text-gray-400">Loading users...</div>
            ) : filteredUsers.length === 0 ? (
              <div className="text-center py-8 text-gray-400">No users found</div>
            ) : (
              <div className="space-y-2">
                {filteredUsers.map((user) => {
                  const isLiked = currentLikers.includes(user.id);
                  const isProcessing = processingUserId === user.id;

                  return (
                    <button
                      key={user.id}
                      onClick={() => handleToggleLike(user, isLiked)}
                      disabled={isProcessing}
                      className={`
                        w-full flex items-center gap-3 p-3 rounded-lg transition-all
                        ${isLiked
                          ? 'bg-pink-500/20 border border-pink-500/50 hover:bg-pink-500/30'
                          : 'bg-white/5 border border-white/10 hover:bg-white/10'
                        }
                        ${isProcessing ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'}
                      `}
                    >
                      <div className="w-10 h-10 rounded-full bg-gradient-to-br from-purple-400 to-pink-400 flex items-center justify-center text-white font-bold flex-shrink-0">
                        {user.avatar ? (
                          
                          <div  className="w-full h-full rounded-full object-cover text-4xl">
                            {user.avatar}
                            </div>
                          
                        ) : (
                          user.displayName?.charAt(0).toUpperCase()
                        )}
                      </div>
                      <div className="flex-1 text-left">
                        <div className="text-white font-medium">{user.displayName}</div>
                        
                        <div className="text-xs text-gray-400">
                          {isProcessing ? 'Processing...' : isLiked ? 'Liked' : 'Click to like'}
                        </div>
                      </div>
                      <div className="text-2xl">
                        {isLiked ? '❤️' : '🤍'}
                      </div>
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {/* Footer */}
          <div className="p-4 border-t border-white/10">
            <button
              onClick={onClose}
              className="w-full px-4 py-2 bg-white/10 hover:bg-white/20 border border-white/20 rounded-lg text-white transition-colors"
            >
              Close
            </button>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
