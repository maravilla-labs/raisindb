import { Link } from 'react-router';
import type { SocialUser } from '../lib/types';

interface UserCardProps {
  user: SocialUser;
  onFollow?: (userId: string) => void;
  isFollowing?: boolean;
}

export default function UserCard({ user, onFollow, isFollowing }: UserCardProps) {
  return (
    <div className="card">
      <div className="flex items-center space-x-4">
        <div className="flex-shrink-0">
          <div className="w-16 h-16 rounded-full bg-blue-100 flex items-center justify-center text-2xl">
            {user.avatar || '👤'}
          </div>
        </div>
        <div className="flex-1 min-w-0">
          <Link to={`/profile/${user.id}`} className="hover:underline">
            <h3 className="font-bold text-lg">{user.displayName}</h3>
            <p className="text-gray-500">@{user.username}</p>
          </Link>
          {user.bio && <p className="mt-1 text-sm text-gray-700">{user.bio}</p>}
          <div className="mt-2 flex space-x-4 text-sm text-gray-600">
            <span>{user.followerCount} followers</span>
            <span>{user.followingCount} following</span>
          </div>
        </div>
        {onFollow && (
          <button
            onClick={() => onFollow(user.id)}
            className={isFollowing ? 'btn-secondary' : 'btn-primary'}
          >
            {isFollowing ? 'Unfollow' : 'Follow'}
          </button>
        )}
      </div>
    </div>
  );
}
