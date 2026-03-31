# RaisinDB Social Feed Example

A Twitter-like social feed application that demonstrates the full power of RaisinDB's real-time, graph-based database capabilities.

## Features

- 🚀 **Real-time Updates** - See new posts instantly with live subscriptions
- 📊 **SQL & Cypher Queries** - Dual query support for flexibility
- 🔗 **Graph Relationships** - Follow system using graph relations
- 📝 **CRUD Operations** - Create, read, update, delete posts and comments
- 👥 **User Profiles** - View user profiles with follower counts
- ⚡ **Live Connection Status** - Real-time connection indicator
- 🎨 **Modern UI** - Built with React Router 7 and Tailwind CSS

## Prerequisites

1. **Node.js** v18 or higher
2. **RaisinDB Server** running on `localhost:8080`
3. **Credentials**:
   - Tenant: `default`
   - Username: `example`
   - Password: `Examples12345678!`

## Installation

### Option 1: From This Directory

From this directory (`packages/raisin-client-js/examples/social-feed/`):

```bash
# First, ensure the parent package is built
cd ../..
npm install
npm run build

# Then install example dependencies
cd examples/social-feed
npm install
```

### Option 2: Quick Start (if parent is already built)

```bash
npm install
```

## Database Setup

Initialize the database with demo data:

```bash
npm run init-db
```

This will:
- Create the `social_feed` repository
- Create the `social` workspace
- Define NodeTypes (SocialUser, Post, Comment)
- Create demo users (Alice, Bob, Carol)
- Create follow relationships using graph relations
- Create sample posts

## Running the App

```bash
npm run dev
```

The app will open at [http://localhost:3000](http://localhost:3000)

## Project Structure

```
src/
├── lib/                  # Core library code
│   ├── raisin.ts        # RaisinDB client singleton
│   ├── init-database.ts # Database initialization script
│   └── types.ts         # TypeScript type definitions
├── hooks/               # Custom React hooks
│   ├── useRaisinClient.ts  # Access to RaisinDB client
│   ├── useAuth.ts          # Authentication state
│   └── useLiveQuery.ts     # Real-time data subscriptions
├── components/          # Reusable UI components
│   ├── LiveIndicator.tsx   # Connection status indicator
│   ├── CreatePost.tsx      # Post creation form
│   ├── PostCard.tsx        # Post display card
│   ├── CommentList.tsx     # Comment thread
│   └── UserCard.tsx        # User profile card
├── pages/               # Route pages
│   ├── Feed.tsx        # Main feed with live updates
│   ├── Profile.tsx     # User profile page
│   ├── Post.tsx        # Single post detail view
│   └── Admin.tsx       # Admin panel (repositories, NodeTypes)
└── styles/             # CSS styles
    └── index.css       # Tailwind CSS configuration
```

## Key Concepts Demonstrated

### 1. Real-time Subscriptions

```typescript
const { data: posts, loading } = useLiveQuery({
  nodeType: 'Post',
  path: '/posts/%'
});
```

### 2. SQL Queries

```typescript
const result = await db.executeSql(
  "SELECT * FROM nodes WHERE node_type = 'Post' ORDER BY created_at DESC"
);
```

### 3. Graph Relationships

```typescript
// Create a follow relationship
await ws.nodes().addRelation(
  followerPath,
  'follows',
  followingPath
);

// Query using Cypher-style syntax
MATCH (u:User)-[:FOLLOWS]->(author:User)-[:POSTED]->(p:Post)
RETURN p
```

### 4. CRUD Operations

```typescript
// Create a post
await ws.nodes().create({
  type: 'Post',
  path: `/posts/post_${Date.now()}`,
  properties: {
    content: 'Hello RaisinDB!',
    likeCount: 0
  }
});

// Update a post
await ws.nodes().update(postId, {
  properties: {
    likeCount: post.likeCount + 1
  }
});
```

### 5. NodeType Management

```typescript
// Create a NodeType
await db.nodeTypes().create('Post', {
  properties: {
    content: { type: 'string', required: true },
    likeCount: { type: 'number', default: 0 }
  }
});

// Publish NodeType
await db.nodeTypes().publish('Post');
```

## Architecture

### Client Initialization

The app uses a singleton pattern for the RaisinDB client (`src/lib/raisin.ts`):

```typescript
const client = new RaisinClient('ws://localhost:8080/sys/default', {
  tenantId: 'default',
  defaultBranch: 'main'
});

await client.connect();
await client.authenticate(credentials);
```

### Data Flow

1. **Connection** - Client connects to RaisinDB WebSocket server
2. **Authentication** - User credentials validated
3. **Database Access** - Get database and workspace handles
4. **Operations** - Perform CRUD operations, queries, subscriptions
5. **Real-time Updates** - Receive live updates via subscriptions

### Component Hierarchy

```
App
├── Navigation (with LiveIndicator)
└── Routes
    ├── Feed (CreatePost + PostCards + useLiveQuery)
    ├── Profile (UserCard + PostCards)
    ├── Post (PostCard + CommentList)
    └── Admin (Repository/NodeType management)
```

## API Examples

### Repository Management

```typescript
// Create repository
await client.createRepository('social_feed', 'Description');

// List repositories
const repos = await client.listRepositories();
```

### Workspace Management

```typescript
const db = client.database('social_feed');

// Create workspace
await db.createWorkspace('social', 'Workspace description');

// List workspaces
const workspaces = await db.listWorkspaces();
```

### Branch Operations

```typescript
// Create branch
await db.branches().create('feature-branch');

// List branches
const branches = await db.branches().list();

// Compare branches
const diff = await db.branches().compare('feature', 'main');
```

## Development

### Build for Production

```bash
npm run build
```

### Preview Production Build

```bash
npm run preview
```

## Troubleshooting

### Connection Issues

1. Ensure RaisinDB server is running on `localhost:8080`
2. Check credentials match configuration
3. Verify network connectivity

### Database Not Initialized

If you see "No posts found", run:

```bash
npm run init-db
```

### Type Errors

```bash
# Ensure all dependencies are installed
npm install

# Check TypeScript configuration
npx tsc --noEmit
```

## Learn More

- [RaisinDB Documentation](https://docs.raisindb.com)
- [Client SDK API Reference](../../README.md)
- [React Router Documentation](https://reactrouter.com)
- [Tailwind CSS Documentation](https://tailwindcss.com)

## License

MIT
