import { useParams } from 'react-router';
import PostCard from '../components/PostCard';
import CommentList from '../components/CommentList';
import type { Post as PostType, Comment } from '../lib/types';

export default function Post() {
  const { postId } = useParams<{ postId: string }>();

  // In a real app, we'd fetch the post and comments
  const post: PostType = {
    id: postId!,
    content: 'This is a demo post showing the detail view with comments.',
    path: `/posts/${postId}`,
    authorId: 'demo-user',
    authorName: 'Demo User',
    likeCount: 5,
    commentCount: 2,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    properties: {
      content: 'This is a demo post showing the detail view with comments.',
      likeCount: 5,
    },
  };

  const comments: Comment[] = [
    {
      id: '1',
      content: 'Great post! RaisinDB makes this so easy.',
      authorId: 'user1',
      authorName: 'Alice',
      postId: postId!,
      likeCount: 2,
      createdAt: new Date().toISOString(),
    },
    {
      id: '2',
      content: 'I love the real-time features!',
      authorId: 'user2',
      authorName: 'Bob',
      postId: postId!,
      likeCount: 1,
      createdAt: new Date().toISOString(),
    },
  ];

  return (
    <div className="max-w-2xl mx-auto">
      <PostCard post={post} />

      <div className="card mt-6">
        <h2 className="text-lg font-bold mb-4">Comments</h2>
        <CommentList comments={comments} />
      </div>
    </div>
  );
}
