import { forwardRef } from 'react';
import { Link } from 'react-router';
import { motion } from 'framer-motion';
import type { Post } from '~/lib/types';
import { formatRelativeTime, getAvatarUrl } from '~/lib/utils';

interface PostCardProps {
  post: Post;
  onLike?: (postId: string) => void;
  isUpdated?: boolean;
}

const PostCard = forwardRef<HTMLDivElement, PostCardProps>(({ post, onLike, isUpdated = false }, ref) => {
  const displayName = post.authorDisplayName || post.authorName || 'Unknown';
  const avatar = post.authorAvatar;

  return (
    <motion.div
      ref={ref}
      layout
      initial={{ opacity: 0, y: 20 }}
      animate={{
        opacity: 1,
        y: 0,
        backgroundColor: isUpdated ? '#dbeafe' : 'transparent',
      }}
      exit={{ opacity: 0, scale: 0.95 }}
      transition={{
        duration: 0.3,
        backgroundColor: { duration: 0.8, ease: 'easeOut' }
      }}
      className="glass glass-border rounded-lg p-6 mb-4 hover:shadow-lg transition-shadow"
    >     
      <div className="flex items-start space-x-3">
        <div className="flex-shrink-0">
          {avatar ? (
            <div
              title={displayName}
              className="w-10 h-10 rounded-full text-4xl flex items-center justify-center"
            >
              {avatar}
            </div>
          ) : (
            <img
              src={getAvatarUrl(displayName)}
              alt={displayName}
              className="w-10 h-10 rounded-full"
            />
          )}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center space-x-2">
            <Link
              to={`/profile/${post.authorId}`}
              className="font-semibold hover:underline text-gray-900 dark:text-gray-100"
            >
              {displayName}
            </Link>
            {post.authorName && post.authorDisplayName && (
              <span className="text-sm text-gray-500 dark:text-gray-400">
                @{post.authorName}
              </span>
            )}
            <span className="text-sm text-gray-500 dark:text-gray-400">
              {formatRelativeTime(post.createdAt)}
            </span>
          </div>
          <Link to={`/post/${post.id}`}>
            <p className="mt-2 text-gray-900 dark:text-gray-100 whitespace-pre-wrap">
              {post.properties?.content}
            </p>
          </Link>
          <div className="mt-3 flex items-center space-x-4 text-sm">
            <button
              onClick={() => onLike?.(post.id)}
              className="flex items-center space-x-1 text-gray-500 hover:text-red-500 dark:text-gray-400 dark:hover:text-red-400 transition-colors"
            >
              <span>❤️</span>
              <span>{post?.properties?.likeCount}</span>
            </button>
            <Link
              to={`/post/${post.id}`}
              className="flex items-center space-x-1 text-gray-500 hover:text-blue-500 dark:text-gray-400 dark:hover:text-blue-400 transition-colors"
            >
              <span>💬</span>
              <span>{post.commentCount}</span>
            </Link>
          </div>
        </div>
      </div>
    </motion.div>
  );
});

PostCard.displayName = 'PostCard';

export default PostCard;
