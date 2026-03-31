import { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { getClient, getConfig } from '../lib/raisin';
import { UserSelectorModal } from './UserSelectorModal';

interface LikeButtonProps {
  postId: string;
}

interface UserWhoLiked {
  userId: string;
  userName: string;
  avatar?: string;
}

export function LikeButton({ postId }: LikeButtonProps) {
  const [usersWhoLiked, setUsersWhoLiked] = useState<UserWhoLiked[]>([]);
  const [showDropdown, setShowDropdown] = useState(false);
  const [showModal, setShowModal] = useState(false);
  const [loading, setLoading] = useState(true);
  const [postPath, setPostPath] = useState<string | null>(null);
  const [flyingHearts, setFlyingHearts] = useState<{ id: string; label: string }[]>([]);
  const dropdownTimeoutRef = useRef<NodeJS.Timeout | null>(null);
  const usersRef = useRef<UserWhoLiked[]>([]);
  const config = getConfig();

  // Fetch users who liked this post
  const fetchLikes = useCallback(async () => {
    try {
      setLoading(true);
      const client = await getClient();
      const db = client.database(config.repository);
      const ws = db.workspace(config.workspace);

      // Resolve the post node via Graph API to avoid SQL edge cases
      const postNode = await ws.nodes().get(postId);
      if (!postNode?.path) {
        console.warn('Post missing path metadata:', postId, postNode);
        setUsersWhoLiked([]);
        return;
      }

      const postPath = postNode.path;
      setPostPath(postPath);

      console.log('Post path:', postPath);

      // Get relationships for this post (incoming "likes" relationships)
      const relationships = await ws.nodes().getRelationships(postPath);

      // Filter for "likes" relationships coming TO this post
      const likeRelations = relationships.incoming.filter(
        (rel) => rel.relation.relation_type === 'likes'
      );

      // Fetch user details for each liker
      const usersRaw = await Promise.all(
        likeRelations.map(async (rel: any) => {
          try {
            const userNode = await ws.nodes().get(rel.source_node_id);
            if (userNode) {
              const displayName = userNode.properties?.displayName;
              const username = userNode.properties?.username;
              const avatar = userNode.properties?.avatar;

              return {
                userId: userNode.id,
                userName: String(displayName || username || 'Unknown'),
                avatar: avatar ? String(avatar) : undefined,
              };
            }
          } catch (err) {
            console.error('Failed to fetch user:', rel.source_node_id, err);
          }
          return null;
        })
      );

      const users: UserWhoLiked[] = usersRaw.filter((u) => u !== null) as UserWhoLiked[];

      setUsersWhoLiked(users);
      usersRef.current = users;
    } catch (error) {
      console.error('Failed to fetch likes:', error);
      setUsersWhoLiked([]);
      usersRef.current = [];
    } finally {
      setLoading(false);
    }
  }, [config.repository, config.workspace, postId]);

  // Fetch likes on mount and when postId changes
  useEffect(() => {
    fetchLikes();
  }, [fetchLikes]);

  // Handle mouse enter for dropdown
  const handleMouseEnter = () => {
    if (dropdownTimeoutRef.current) {
      clearTimeout(dropdownTimeoutRef.current);
    }
    if (usersWhoLiked.length > 0) {
      setShowDropdown(true);
    }
  };

  // Handle mouse leave for dropdown (with delay)
  const handleMouseLeave = () => {
    dropdownTimeoutRef.current = setTimeout(() => {
      setShowDropdown(false);
    }, 300);
  };

  // Cancel timeout when mouse re-enters dropdown
  const handleDropdownMouseEnter = () => {
    if (dropdownTimeoutRef.current) {
      clearTimeout(dropdownTimeoutRef.current);
    }
  };

  // Cleanup timeout on unmount
  useEffect(() => {
    return () => {
      if (dropdownTimeoutRef.current) {
        clearTimeout(dropdownTimeoutRef.current);
      }
    };
  }, []);

  const launchHeart = useCallback((label: string) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
    setFlyingHearts((prev) => [...prev, { id, label }]);
    setTimeout(() => {
      setFlyingHearts((prev) => prev.filter((heart) => heart.id !== id));
    }, 1600);
  }, []);

  useEffect(() => {
    if (!postPath) return;

    let active = true;
    let subscription: { unsubscribe: () => Promise<void> } | null = null;

    (async () => {
      try {
        const client = await getClient();
        if (!active) return;
        const db = client.database(config.repository);
        const ws = db.workspace(config.workspace);
        const eventHandler = db.events();
        subscription = await eventHandler.subscribe(
          {
            workspace: config.workspace,
            path: postPath,
            node_type: 'Post',
            event_types: ['node:relation_added', 'node:relation_removed'],
          },
          async (event) => {
            const relationType = (event.payload as any)?.relation_type;
            const direction = (event.payload as any)?.relation_direction;
            if (relationType !== 'likes' || direction !== 'incoming') {
              return;
            }

            const likerId = (event.payload as any)?.related_node_id;
            if (event.event_type === 'node:relation_added') {
              let likerName: string | undefined = usersRef.current.find((u) => u.userId === likerId)?.userName;
              if (!likerName && likerId) {
                try {
                  const likerNode = await ws.nodes().get(likerId);
                  const displayName = likerNode?.properties?.displayName;
                  const username = likerNode?.properties?.username;
                  likerName = String(displayName || username || 'Someone');
                } catch (err) {
                  console.error('Failed to fetch liker name', err);
                }
              }
              if (likerName) {
                launchHeart(likerName);
              }
            }

            // Refresh likes list so UI stays in sync
            fetchLikes();
          }
        );
      } catch (error) {
        console.error('Failed to subscribe to like events:', error);
      }
    })();

    return () => {
      active = false;
      if (subscription) {
        subscription
          .unsubscribe()
          .catch((err) => console.warn('Failed to unsubscribe from like events', err));
      }
    };
  }, [postPath, config.repository, config.workspace, fetchLikes, launchHeart]);

  const likeCount = usersWhoLiked.length;
  const hasLikes = likeCount > 0;

  return (
    <div className="relative">
      <AnimatePresence>
        {flyingHearts.map((heart) => (
          <motion.div
            key={heart.id}
            initial={{ opacity: 0, y: 10, scale: 0.8 }}
            animate={{ opacity: 1, y: -30, scale: 1 }}
            exit={{ opacity: 0, y: -50, scale: 0.6 }}
            transition={{ duration: 1.2 }}
            className="absolute -top-6 left-1/2 -translate-x-1/2 text-pink-400 font-semibold drop-shadow-lg pointer-events-none"
          >
            ❤️ {heart.label}
          </motion.div>
        ))}
      </AnimatePresence>
      <button
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
        onClick={() => setShowModal(true)}
        className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-white/5 hover:bg-white/10 border border-white/10 transition-all group"
      >
        <motion.span
          className="text-lg"
          whileHover={{ scale: 1.2 }}
          whileTap={{ scale: 0.9 }}
        >
          {hasLikes ? '❤️' : '🤍'}
        </motion.span>
        <span className="text-sm text-gray-300 group-hover:text-white transition-colors">
          {loading ? '...' : likeCount}
        </span>
      </button>

      {/* Hover dropdown showing who liked */}
      <AnimatePresence>
        {showDropdown && usersWhoLiked.length > 0 && (
          <motion.div
            initial={{ opacity: 0, y: -10 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -10 }}
            transition={{ duration: 0.2 }}
            onMouseEnter={handleDropdownMouseEnter}
            onMouseLeave={handleMouseLeave}
            className="absolute bottom-full left-0 mb-2 z-50"
          >
            <div className="bg-gray-900/95 backdrop-blur-xl border border-white/20 rounded-lg shadow-2xl p-3 min-w-[200px] max-w-[300px]">
              <div className="text-xs font-semibold text-gray-400 mb-2">
                Liked by
              </div>
              <div className="space-y-2 max-h-[200px] overflow-y-auto">
                {usersWhoLiked.map((user) => (
                  <div
                    key={user.userId}
                    className="flex items-center gap-2 text-sm"
                  >
                    <div className="w-6 h-6 rounded-full bg-gradient-to-br from-purple-400 to-pink-400 flex items-center justify-center text-white text-xs font-bold flex-shrink-0">
                      {user.avatar ? (
                        <img
                          src={user.avatar}
                          alt={user.userName}
                          className="w-full h-full rounded-full object-cover"
                        />
                      ) : (
                        user.userName?.charAt(0).toUpperCase()
                      )}
                    </div>
                    <span className="text-gray-200 truncate">
                      {user.userName}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* User selector modal */}
      <UserSelectorModal
        isOpen={showModal}
        onClose={() => setShowModal(false)}
        postId={postId}
        currentLikers={usersWhoLiked.map(u => u.userId)}
        onLikeToggled={fetchLikes}
      />
    </div>
  );
}
