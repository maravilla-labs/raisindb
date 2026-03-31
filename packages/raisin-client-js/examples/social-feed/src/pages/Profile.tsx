import { useParams } from 'react-router';
import { useLiveQuery } from '../hooks/useLiveQuery';
import PostCard from '../components/PostCard';
import type { Post, SocialUser } from '../lib/types';

export default function Profile() {
  const { userId } = useParams<{ userId: string }>();

  const { data: posts, loading } = useLiveQuery<Post>({
    nodeType: 'Post',
    path: '/posts/%',
  });

  // In a real app, we'd fetch the user profile
  const user: SocialUser = {
    id: userId!,
    username: 'demo_user',
    displayName: 'Demo User',
    bio: 'This is a demo user profile',
    followerCount: 42,
    followingCount: 128,
    createdAt: new Date().toISOString(),
  };

  // Filter posts by this user
  const userPosts = posts.filter((p) => p.authorId === userId);

  return (
    <div className="max-w-2xl mx-auto">
      <div className="card mb-6">
        <div className="flex items-center space-x-4">
          <div className="w-20 h-20 rounded-full bg-blue-100 flex items-center justify-center text-3xl">
            👤
          </div>
          <div>
            <h1 className="text-2xl font-bold">{user.displayName}</h1>
            <p className="text-gray-600">@{user.username}</p>
            {user.bio && <p className="mt-2 text-gray-700">{user.bio}</p>}
            <div className="mt-3 flex space-x-4 text-sm">
              <span>
                <strong>{user.followerCount}</strong> followers
              </span>
              <span>
                <strong>{user.followingCount}</strong> following
              </span>
            </div>
          </div>
        </div>
      </div>

      <h2 className="text-xl font-bold mb-4">Posts</h2>

      {loading ? (
        <div className="text-center py-8">
          <div className="inline-block animate-spin rounded-full h-8 w-8 border-b-2 border-blue-600"></div>
        </div>
      ) : userPosts.length === 0 ? (
        <div className="card text-center py-8">
          <p className="text-gray-500">No posts yet</p>
        </div>
      ) : (
        <div>
          {userPosts.map((post) => (
            <PostCard key={post.id} post={post} />
          ))}
        </div>
      )}
    </div>
  );
}
