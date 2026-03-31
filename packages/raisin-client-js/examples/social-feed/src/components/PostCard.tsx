import { forwardRef } from 'react';
import { Link } from 'react-router';
import { motion } from 'framer-motion';
import type { Post } from '../lib/types';
import { LikeButton } from './LikeButton';

interface PostCardProps {
  post: Post;
  isUpdated?: boolean;
}

const PostCard = forwardRef<HTMLDivElement, PostCardProps>(({ post, isUpdated = false }, ref) => {
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
        backgroundColor: isUpdated ? '#dbeafe' : '#ffffff',
      }}
      exit={{ opacity: 0, scale: 0.95 }}
      transition={{
        duration: 0.3,
        backgroundColor: { duration: 0.8, ease: 'easeOut' }
      }}
      className="card mb-4 hover:shadow-md transition-shadow"
    >
      <div className="flex items-start space-x-3">
        <div className="flex-shrink-0">
          {avatar ? (
            <div
              
              title={displayName}
              className="w-10 h-10 rounded-full object-cover text-4xl"
            >{avatar}</div>
          ) : (
            <div className="w-10 h-10 rounded-full bg-blue-100 flex items-center justify-center text-lg">
              👤
            </div>
          )}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center space-x-2">
            <Link
              to={`/profile/${post.authorId}`}
              className="font-semibold hover:underline"
            >
              {displayName}
            </Link>
            {post.authorName && post.authorDisplayName && (
              <span className="text-sm text-gray-500">
                @{post.authorName}
              </span>
            )}
            <span className="text-sm text-gray-500">
              {new Date(post.created_at).toLocaleString()}
            </span>
          </div>
          <Link to={`/post${post.path}`}>
            <p className="mt-2 text-gray-900">{post.properties.content}</p>
          </Link>
          <div className="mt-3 flex items-center space-x-4 text-sm">
            <LikeButton postId={post.id} />
            <Link
              to={`/post/${post.id}`}
              className="flex items-center space-x-1 text-gray-500 hover:text-blue-500 transition-colors"
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
