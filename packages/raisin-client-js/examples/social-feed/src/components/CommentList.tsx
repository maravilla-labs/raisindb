import type { Comment } from '../lib/types';

interface CommentListProps {
  comments: Comment[];
}

export default function CommentList({ comments }: CommentListProps) {
  if (comments.length === 0) {
    return (
      <div className="text-center py-8 text-gray-500">
        No comments yet. Be the first to comment!
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {comments.map((comment) => (
        <div key={comment.id} className="border-l-2 border-gray-200 pl-4">
          <div className="flex items-center space-x-2">
            <span className="font-semibold">{comment.authorName || 'Unknown'}</span>
            <span className="text-sm text-gray-500">
              {new Date(comment.createdAt).toLocaleString()}
            </span>
          </div>
          <p className="mt-1 text-gray-900">{comment.content}</p>
          <button className="mt-2 text-sm text-gray-500 hover:text-red-500">
            ❤️ {comment.likeCount}
          </button>
        </div>
      ))}
    </div>
  );
}
