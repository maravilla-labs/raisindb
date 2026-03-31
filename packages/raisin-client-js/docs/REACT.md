# React Integration Guide

The RaisinDB client SDK includes a full React integration layer: a Provider component and hooks for auth, connection state, SQL queries, event subscriptions, conversations, and flows.

It uses a "bring your own React" pattern -- no peer dependency on React, full type safety.

## Quick Start

### 1. Set up `raisin.ts`

```ts
import React from 'react';
import { RaisinClient, LocalStorageTokenStorage, createRaisinReact } from '@raisindb/client';

export const client = new RaisinClient('wss://localhost:8443/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myrepo'),
  tenantId: 'default',
  defaultBranch: 'main',
});

export const {
  RaisinProvider,
  useRaisinClient,
  useDatabase,
  useAuth,
  useConnection,
  useSql,
  useSubscription,
  useConversation,
  useConversationList,
  useFlow,
} = createRaisinReact(React);
```

### 2. Wrap your app

```tsx
import { client, RaisinProvider } from './raisin';

export default function App() {
  return (
    <RaisinProvider client={client} repository="myrepo">
      <AuthGate />
    </RaisinProvider>
  );
}
```

### 3. Use hooks in components

```tsx
import { useAuth, useConnection, useSql } from './raisin';

function AuthGate() {
  const { user, isLoading, initSession } = useAuth();
  const { isReady } = useConnection();

  useEffect(() => { initSession('myrepo'); }, []);

  if (isLoading || !isReady) return <Spinner />;
  if (!user) return <LoginPage />;
  return <Dashboard />;
}

function PostList() {
  const { data: posts, isLoading } = useSql<Post>(
    "SELECT * FROM content WHERE node_type = 'Post' ORDER BY created_at DESC",
    [],
    { realtime: { workspace: 'content', nodeType: 'Post' } }
  );

  if (isLoading) return <Spinner />;
  return posts?.map(p => <PostCard key={p.id} post={p} />);
}
```

---

## Provider

### `RaisinProvider`

| Prop | Type | Required | Description |
|------|------|----------|-------------|
| `client` | `RaisinClient` | Yes | The client instance |
| `repository` | `string` | No | Default repository for hooks |
| `children` | `ReactNode` | No | Child components |

The provider is **passive** -- it does not auto-connect or call `initSession()`. You control the lifecycle explicitly via hooks.

```tsx
<RaisinProvider client={client} repository="myrepo">
  <App />
</RaisinProvider>
```

For multi-repo apps, nest providers or pass `repository` to individual hooks:

```tsx
const db = useDatabase('other-repo');
const { data } = useSql('SELECT ...', [], { repository: 'other-repo' });
```

---

## Context Hooks

### `useRaisinClient()`

Returns the `RaisinClient` from the nearest `RaisinProvider`.

```ts
const client = useRaisinClient();
```

### `useDatabase(repository?)`

Returns a `Database` instance. Uses the provider's default repository if none is passed.

```ts
const db = useDatabase();          // uses provider's repository
const db = useDatabase('other');   // explicit repository
```

---

## Authentication (`useAuth`)

```ts
const {
  user,            // IdentityUser | null
  isAuthenticated, // boolean
  isLoading,       // boolean (during login/register/logout/initSession)
  login,           // (email, password, repository) => Promise<IdentityUser>
  register,        // (email, password, repository, displayName?) => Promise<IdentityUser>
  logout,          // (options?) => Promise<void>
  initSession,     // (repository) => Promise<IdentityUser | null>
} = useAuth();
```

Subscribes to `client.onAuthStateChange()` and `client.onUserChange()` for reactive updates.

### Login Form

```tsx
function LoginPage() {
  const { login, isLoading } = useAuth();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    try {
      await login(email, password, 'myrepo');
    } catch (err) {
      setError(err.message);
    }
  };

  return (
    <form onSubmit={handleSubmit}>
      <input value={email} onChange={e => setEmail(e.target.value)} />
      <input type="password" value={password} onChange={e => setPassword(e.target.value)} />
      <button disabled={isLoading}>Log in</button>
      {error && <p>{error}</p>}
    </form>
  );
}
```

### AuthGate Pattern

```tsx
function AuthGate() {
  const { user, isLoading, initSession } = useAuth();
  const { isReady } = useConnection();

  useEffect(() => { initSession('myrepo'); }, []);

  if (isLoading || !isReady) return <Spinner />;
  if (!user) return <LoginPage />;
  return <Dashboard />;
}
```

---

## Connection State (`useConnection`)

```ts
const {
  state,        // ConnectionState ('connected' | 'connecting' | 'disconnected' | ...)
  isConnected,  // boolean
  isReady,      // boolean -- connected AND (authenticated OR no stored token)
  connect,      // () => Promise<void>
  disconnect,   // () => void
} = useConnection();
```

- `isReady` is the "green dot" -- the client can process requests.
- `isConnected` only means the WebSocket is open; `isReady` also accounts for auth.

### Connection Indicator

```tsx
function ConnectionStatus() {
  const { state, isReady } = useConnection();
  return (
    <span className={isReady ? 'text-green-500' : 'text-red-500'}>
      {state}
    </span>
  );
}
```

---

## SQL Queries (`useSql`)

```ts
const { data, isLoading, error, refetch } = useSql<T>(sql, params?, options?);
```

Named `useSql` (not `useQuery`) to avoid collision with TanStack Query, Apollo, etc.

### Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | `boolean` | `true` | Skip execution when false |
| `repository` | `string` | Provider default | Override repository |
| `refetchOnReconnect` | `boolean` | `true` | Auto-refetch after reconnection |
| `realtime` | `object` | - | Subscribe to events and auto-refetch |
| `realtime.workspace` | `string` | - | Workspace to subscribe to |
| `realtime.eventTypes` | `string[]` | - | Filter event types |
| `realtime.path` | `string` | - | Filter by path pattern |
| `realtime.nodeType` | `string` | - | Filter by node type |

### Basic Query

```tsx
const { data: users } = useSql<User>(
  "SELECT * FROM 'raisin:access_control' WHERE node_type = 'raisin:User'"
);
```

### Parameterized Query

```tsx
const { data: post } = useSql<Post>(
  "SELECT * FROM content WHERE properties->>'slug'::String = $1",
  [slug],
);
```

### Conditional Query

```tsx
const { isReady } = useConnection();
const { data } = useSql(
  "SELECT * FROM content",
  [],
  { enabled: isReady },
);
```

### Real-time Updates

```tsx
const { data: posts } = useSql<Post>(
  "SELECT * FROM content WHERE node_type = 'Post' ORDER BY created_at DESC",
  [],
  {
    realtime: {
      workspace: 'content',
      nodeType: 'Post',
    },
  },
);
```

When any `Post` node changes in the `content` workspace, the query automatically re-runs.

### Using with TanStack Query

`useSql` and TanStack's `useQuery` coexist without collision:

```tsx
import { useQuery } from '@tanstack/react-query';
import { useSql } from './raisin';

function Dashboard() {
  // RaisinDB query
  const { data: posts } = useSql<Post>("SELECT * FROM content");

  // TanStack query (external API)
  const { data: weather } = useQuery({
    queryKey: ['weather'],
    queryFn: () => fetch('/api/weather').then(r => r.json()),
  });
}
```

---

## Event Subscriptions (`useSubscription`)

```ts
useSubscription(options, callback);
```

Auto-subscribes on mount, unsubscribes on unmount. Uses the `callbackRef` pattern to avoid re-subscribing when callback identity changes.

### Options

All `SubscriptionFilters` fields plus:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | `boolean` | `true` | Conditionally subscribe |

### Toast Notifications

```tsx
function Notifications() {
  const { toast } = useToast();

  useSubscription(
    { workspace: 'content', node_type: 'Post', include_node: true },
    (event) => {
      toast({ title: `New post: ${event.payload?.name}` });
    },
  );

  return null;
}
```

---

## Conversations (`useConversation` / `useConversationList`)

Pre-bound wrappers -- no need to pass `React` as the first argument.

### AI Chat

```tsx
function Chat() {
  const db = useDatabase();
  const chat = useConversation({
    database: db,
    conversationPath: '/home/conversations/support-chat',
  });

  return (
    <div>
      {chat.messages.map((m, i) => (
        <div key={i}>{m.content}</div>
      ))}
      {chat.isStreaming && <p>{chat.streamingText}</p>}
      <input onKeyDown={(e) => {
        if (e.key === 'Enter') chat.sendMessage(e.currentTarget.value);
      }} />
    </div>
  );
}
```

### Conversation List

```tsx
function Inbox() {
  const db = useDatabase();
  const { conversations, totalUnreadCount } = useConversationList({
    database: db,
    type: 'ai_chat',
    realtime: true,
  });

  return (
    <div>
      <h2>Inbox ({totalUnreadCount} unread)</h2>
      {conversations.map(c => (
        <div key={c.path}>{c.subject} - {c.unreadCount} unread</div>
      ))}
    </div>
  );
}
```

---

## Flows (`useFlow`)

Pre-bound wrapper for running and monitoring flows.

```tsx
function OrderProcessor() {
  const db = useDatabase();
  const flow = useFlow({ database: db });

  return (
    <div>
      <button
        onClick={() => flow.run('/flows/process-order', { orderId: '123' })}
        disabled={flow.isRunning}
      >
        Process Order
      </button>
      <p>Status: {flow.status}</p>
      {flow.status === 'waiting' && (
        <button onClick={() => flow.resume({ approved: true })}>
          Approve
        </button>
      )}
      {flow.error && <p className="error">{flow.error}</p>}
    </div>
  );
}
```

---

## Common Patterns

### Real-time Dashboard

```tsx
function Dashboard() {
  const { isReady } = useConnection();

  const { data: stats } = useSql<Stats>(
    "SELECT COUNT(*) as total, node_type FROM content GROUP BY node_type",
    [],
    {
      enabled: isReady,
      realtime: { workspace: 'content' },
    },
  );

  useSubscription(
    { workspace: 'content', include_node: true },
    (event) => {
      console.log('Activity:', event.type, event.payload?.name);
    },
  );

  return <StatsGrid stats={stats} />;
}
```

### Conditional Queries with Auth

```tsx
function UserProfile() {
  const { user } = useAuth();
  const { data: profile } = useSql<Profile>(
    "SELECT * FROM 'raisin:access_control' WHERE path = $1",
    [user?.home],
    { enabled: !!user?.home },
  );

  if (!profile?.[0]) return null;
  return <ProfileCard profile={profile[0]} />;
}
```
