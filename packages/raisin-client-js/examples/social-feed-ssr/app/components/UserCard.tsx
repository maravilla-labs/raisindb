import { Link } from 'react-router';
import type { SocialUser } from '~/lib/types';
import { getAvatarUrl, formatNumber } from '~/lib/utils';

interface UserCardProps {
  user: SocialUser;
  onFollow?: (userId: string) => void;
  isFollowing?: boolean;
}

export default function UserCard({ user, onFollow, isFollowing }: UserCardProps) {
  return (
    <div className="glass glass-border rounded-lg p-6 shadow-lg">
      <div className="flex items-center space-x-4">
        <div className="flex-shrink-0">
          <img
            src={getAvatarUrl(user.username, user.avatar)}
            alt={user.displayName}
            className="w-16 h-16 rounded-full"
          />
        </div>
        <div className="flex-1 min-w-0">
          <Link to={`/profile/${user.id}`} className="hover:underline">
            <h3 className="font-bold text-lg text-gray-900 dark:text-gray-100">
              {user.displayName}
            </h3>
            <p className="text-gray-500 dark:text-gray-400">@{user.username}</p>
          </Link>
          {user.bio && (
            <p className="mt-1 text-sm text-gray-700 dark:text-gray-300">{user.bio}</p>
          )}
          <div className="mt-2 flex space-x-4 text-sm text-gray-600 dark:text-gray-400">
            <span>{formatNumber(user.followerCount)} followers</span>
            <span>{formatNumber(user.followingCount)} following</span>
          </div>
        </div>
        {onFollow && (
          <button
            onClick={() => onFollow(user.id)}
            className={`px-4 py-2 rounded-lg font-medium transition-colors ${
              isFollowing
                ? 'bg-gray-200 hover:bg-gray-300 text-gray-800 dark:bg-gray-700 dark:hover:bg-gray-600 dark:text-gray-200'
                : 'bg-blue-500 hover:bg-blue-600 text-white'
            }`}
          >
            {isFollowing ? 'Unfollow' : 'Follow'}
          </button>
        )}
      </div>
    </div>
  );
}
