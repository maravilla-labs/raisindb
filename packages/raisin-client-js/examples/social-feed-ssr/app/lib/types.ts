// TypeScript interfaces for the Social Feed SSR app

export interface SocialUser {
  id: string;
  username: string;
  displayName: string;
  bio?: string;
  avatar?: string;
  followerCount: number;
  followingCount: number;
  createdAt: string;
}

export interface Post {
  id: string;
  content: string;
  path: string;
  authorId: string;
  authorName?: string;
  authorDisplayName?: string;
  authorAvatar?: string;
  likeCount: number;
  commentCount: number;
  createdAt: string;
  updatedAt: string;
  properties: {
    content: string;
    likeCount: number;
  };
}

export interface Comment {
  id: string;
  content: string;
  authorId: string;
  authorName?: string;
  postId: string;
  likeCount: number;
  createdAt: string;
}

export interface FollowRelation {
  followerId: string;
  followingId: string;
  createdAt: string;
}

export type QueryMode = 'sql' | 'cypher';

export interface QueryExample {
  title: string;
  description: string;
  sql?: string;
  cypher?: string;
}
