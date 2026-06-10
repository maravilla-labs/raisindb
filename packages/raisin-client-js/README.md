# RaisinDB JavaScript/TypeScript Client SDK

A universal WebSocket client for RaisinDB that works seamlessly in browser, Node.js, and serverless environments.

## Features

- **Universal Compatibility**: Works in browser, Node.js, and serverless environments
- **TypeScript First**: Full TypeScript support with comprehensive type definitions
- **Auto-Reconnection**: Automatic reconnection with exponential backoff
- **Event Subscriptions**: Real-time event streaming with filtered subscriptions
- **MessagePack Protocol**: Efficient binary protocol using MessagePack
- **JWT Authentication**: Built-in authentication and token management
- **Identity Auth**: Email/password login, registration, session management
- **Auth State Changes**: Firebase/Supabase-style `onAuthStateChange()` listener
- **User Home Subscriptions**: Auto-subscribe to user profile changes
- **SQL Queries**: Template literal support for safe SQL queries
- **Type-Safe**: Full type safety for all operations
- **Framework Agnostic**: Works with React, Vue, Svelte, Angular, or vanilla JS

## Installation

```bash
npm install @raisindb/client
```

For Node.js environments, also install the WebSocket library:

```bash
npm install ws
```

## Quick Start

```typescript
import { RaisinClient } from '@raisindb/client';

// Create client
const client = new RaisinClient('raisin://localhost:8080/sys/default');

// Connect and authenticate
await client.connect();
await client.authenticate({
  username: 'admin',
  password: 'password'
});

// Get database and workspace
const db = client.database('my_repo');
const ws = db.workspace('content');

// Create a node
const node = await ws.nodes().create({
  type: 'Page',
  path: '/home',
  properties: {
    title: 'Home Page',
    description: 'Welcome to our site'
  }
});

console.log('Created node:', node);
```

## Usage Examples

### Admin Authentication

```typescript
import { RaisinClient, ConnectionState } from '@raisindb/client';

const client = new RaisinClient('raisin://localhost:8080/sys/default', {
  connection: {
    autoReconnect: true,
    heartbeatInterval: 30000,
  },
  requestTimeout: 30000,
  defaultBranch: 'main',
});

// Listen to connection state changes (public API)
const unsubscribe = client.onConnectionStateChange((state) => {
  console.log('Connection state:', state);
  if (state === ConnectionState.Disconnected) {
    showOfflineIndicator();
  }
});

// Connect and authenticate (admin user)
await client.connect();
await client.authenticate({ username: 'admin', password: 'password' });

console.log('Authenticated:', client.isAuthenticated());
```

### Identity Authentication (Email/Password)

For user-facing apps with email/password authentication:

```typescript
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';

// Create client with persistent token storage
const client = new RaisinClient('ws://localhost:8081/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myapp'),
});

// Login with email/password
const user = await client.loginWithEmail('user@example.com', 'password', 'myrepo');
console.log('Logged in as:', user.email);
console.log('User home path:', user.userHome); // e.g., '/users/internal/john-at-example-com'

// Register new user
const newUser = await client.registerWithEmail(
  'newuser@example.com',
  'password123',
  'myrepo',
  'John Doe'  // display name
);

// Logout (reconnects as anonymous by default)
await client.logout();

// Or fully disconnect
await client.logout({ disconnect: true });
```

### Session Restoration

Restore user session on app startup:

```typescript
const client = new RaisinClient('ws://localhost:8081/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myapp'),
});

// Initialize session from stored tokens
const user = await client.initSession('myrepo');
if (user) {
  console.log('Session restored for:', user.email);
} else {
  // No stored session - redirect to login
  redirectToLogin();
}
```

### Auth State Changes (Firebase/Supabase Pattern)

Listen reactively to authentication events:

```typescript
import { RaisinClient, AuthEvent } from '@raisindb/client';

// Subscribe to auth state changes
const unsubscribe = client.onAuthStateChange(({ event, session }) => {
  switch (event) {
    case 'SIGNED_IN':
      console.log('User signed in:', session.user?.email);
      break;
    case 'SIGNED_OUT':
      console.log('User signed out');
      redirectToLogin();
      break;
    case 'TOKEN_REFRESHED':
      console.log('Token refreshed');
      break;
    case 'SESSION_EXPIRED':
      console.log('Session expired');
      break;
    case 'USER_UPDATED':
      console.log('User profile updated:', session.user);
      break;
  }
});

// Later, to stop listening:
unsubscribe();
```

### User Home Changes

Listen for real-time updates to the user's profile node:

```typescript
// Auto-subscribes to user's home node when logged in
const unsubscribe = client.onUserChange(({ node, changeType }) => {
  console.log('User home updated:', changeType);
  console.log('New properties:', node.properties);
  // Update UI with new avatar, displayName, preferences, etc.
});
```

### Helper Methods

```typescript
// Get current session (sync)
const session = client.getSession();
if (session) {
  console.log('User:', session.user?.email);
  console.log('Token:', session.accessToken);
}

// Get current user (alias for getStoredUser)
const user = client.getUser();

// Check if user has stored token
if (client.hasStoredToken()) {
  await client.initSession('myrepo');
}
```

### Node Operations

```typescript
const db = client.database('my_repo');
const ws = db.workspace('content');
const nodes = ws.nodes();

// Create a node
const page = await nodes.create({
  type: 'Page',
  path: '/blog/my-post',
  properties: {
    title: 'My First Post',
    author: 'John Doe',
    published: true,
  },
});

// Get a node by ID
const node = await nodes.get(page.id);

// Get a node by path
const nodeByPath = await nodes.getByPath('/blog/my-post');

// Update a node
const updated = await nodes.update(page.id, {
  properties: {
    title: 'My Updated Post',
    updatedAt: new Date().toISOString(),
  },
});

// Query nodes by type
const pages = await nodes.queryByType('Page', 10);

// Query nodes by property
const publishedPages = await nodes.queryByProperty('published', true);

// Get children of a node
const children = await nodes.getChildren(page.id);

// Delete a node
await nodes.delete(page.id);
```

### SQL Queries

```typescript
const db = client.database('my_repo');

// Template literal queries (automatically parameterized)
const nodeType = 'Page';
const results = await db.sql`
  SELECT * FROM nodes
  WHERE node_type = ${nodeType}
  ORDER BY created_at DESC
  LIMIT 10
`;

console.log('Columns:', results.columns);
console.log('Rows:', results.rows);
console.log('Row count:', results.row_count);

// Raw SQL with explicit parameters
const sqlQuery = db.getSqlQuery();
const results2 = await sqlQuery.execute(
  'SELECT * FROM nodes WHERE node_type = $1 AND created_at > $2',
  ['Page', '2024-01-01']
);

// Raw SQL without parameters (use with caution)
const results3 = await sqlQuery.raw('SELECT COUNT(*) FROM nodes');
```

### Event Subscriptions

```typescript
const db = client.database('my_repo');
const ws = db.workspace('content');
const events = ws.events();

// Subscribe to all events in workspace
const subscription1 = await events.subscribe({}, (event) => {
  console.log('Event:', event.event_type);
  console.log('Payload:', event.payload);
});

// Subscribe to specific event types
const subscription2 = await events.subscribeToTypes(
  ['node:created', 'node:updated'],
  (event) => {
    console.log('Node changed:', event.payload);
  }
);

// Subscribe to events for a specific path pattern
const subscription3 = await events.subscribeToPath('/blog/*', (event) => {
  console.log('Blog event:', event);
});

// Subscribe to events for a specific node type
const subscription4 = await events.subscribeToNodeType('Page', (event) => {
  console.log('Page event:', event);
});

// Unsubscribe
await subscription1.unsubscribe();
```

### Build a chatbox

Everything chat lives behind `db.conversations` (low-level) and
`ConversationStore` (framework-agnostic state container). The store creates
the conversation lazily on the first message, streams the agent's reply over
SSE, tracks in-flight tool calls, and projects plans for approval UIs.

```typescript
import { ConversationStore } from '@raisindb/client';

const db = client.database('myrepo');

const store = new ConversationStore({
  database: db,
  // Conversation is created on the first sendMessage(). The participant is
  // an agent path, so this becomes an `ai_chat` conversation.
  createOptions: { participant: '/agents/support' },
});

store.subscribe((s) => {
  renderMessages(s.messages);                      // ChatMessage[]
  if (s.isStreaming) renderPartial(s.streamingText); // live tokens

  // Tool-call badges: "running" entries are in-flight
  for (const tc of s.activeToolCalls) {
    renderToolBadge(tc.functionName, tc.status);   // running | completed | failed
  }

  // Plans awaiting approval
  for (const plan of s.plans) {
    if (plan.status === 'pending_approval') {
      renderPlanCard(plan, {
        approve: () => store.approvePlan(plan.planPath),
        reject: (why) => store.rejectPlan(plan.planPath, why),
      });
    }
  }

  if (s.error) showError(s.error);
});

await store.sendMessage('Plan next week\'s shifts');
// ...
store.stop();    // abort the current turn in the UI
store.destroy(); // on teardown
```

To resume an existing conversation, pass `conversationPath` instead of (or in
addition to) `createOptions` and call `store.loadMessages()` for history. To
build an inbox-style list of conversations, use `ConversationListStore`
(`load()`, `createConversation()`, `markAsRead()`, `realtime: true`).

### Listen to the inbox (notifications without an extra API)

Server-side messaging delivers items into the logged-in user's home inbox in
the `raisin:access_control` workspace. A plain node subscription on
`${home}/inbox/**` is all an inbox bell needs — no polling, no extra API.

```typescript
import { normalizeHomePath } from '@raisindb/client';

const user = await client.initSession('myrepo');

// user.home may be workspace-prefixed (/raisin:access_control/users/...).
// Subscription paths must be workspace-relative — always normalize.
const home = normalizeHomePath(user!.home)!;

await db.workspace('raisin:access_control').events().subscribe(
  {
    path: `${home}/inbox/**`,
    event_types: ['node:created'],
    include_node: true, // deliver the full node so we can render a title
  },
  (event) => {
    const props = event.payload.node?.properties ?? {};
    if (props.role === 'user') return; // skip the user's own chat messages
    bumpBadge();
    showToast((props.title as string) ?? (props.subject as string) ?? 'New inbox item');
  },
);
```

**Path filter semantics** (important — there is no implicit prefix matching):

| Pattern | Matches |
|---------|---------|
| `/users/u1/inbox` | exactly that node, nothing below it |
| `/users/u1/inbox/*` | direct children only (`*` = one path segment) |
| `/users/u1/inbox/**` | the whole subtree, any depth (`**` = recursive) |

Inbox items nest (e.g. `inbox/chats/<conversation>/<message>`), so you almost
always want `/**`.

### Workspace Management

```typescript
const db = client.database('my_repo');
const workspaces = db.workspaces();

// Create a workspace
const workspace = await workspaces.create({
  name: 'blog',
  description: 'Blog content workspace',
});

// Get workspace metadata
const ws = await workspaces.get('blog');
console.log('Workspace:', ws);

// List all workspaces
const allWorkspaces = await workspaces.list();

// Update workspace
const updated = await workspaces.update('blog', {
  description: 'Updated description',
  allowed_node_types: ['Page', 'Post', 'Image'],
});

// Delete workspace
await workspaces.delete('blog');
```

### Working with Multiple Branches

```typescript
const client = new RaisinClient('raisin://localhost:8080/sys/default');
await client.connect();
await client.authenticate({ username: 'admin', password: 'password' });

// Work on main branch
const db = client.database('my_repo');
const mainWs = db.workspace('content');
const node = await mainWs.nodes().create({
  type: 'Page',
  path: '/test',
  properties: { title: 'Test' },
});

// Switch to feature branch
client.setBranch('feature/new-design');

// Operations now run on feature branch
const featureWs = db.workspace('content');
const featureNode = await featureWs.nodes().get(node.id);

// Switch back to main
client.setBranch('main');
```

### Error Handling

```typescript
import { RaisinClient } from '@raisindb/client';

try {
  const client = new RaisinClient('raisin://localhost:8080/sys/default');
  await client.connect();

  try {
    await client.authenticate({
      username: 'admin',
      password: 'wrong-password'
    });
  } catch (error) {
    console.error('Authentication failed:', error.message);
    console.error('Error code:', error.code);
  }

  const db = client.database('my_repo');
  const ws = db.workspace('content');

  try {
    const node = await ws.nodes().get('non-existent-id');
    if (!node) {
      console.log('Node not found');
    }
  } catch (error) {
    console.error('Error fetching node:', error);
  }

} catch (error) {
  console.error('Connection error:', error);
}
```

### Custom Token Storage

By default, tokens are stored in memory. You can implement custom storage:

```typescript
import { TokenStorage } from '@raisindb/client';

class LocalStorageTokenStorage implements TokenStorage {
  getAccessToken(): string | null {
    return localStorage.getItem('access_token');
  }

  setAccessToken(token: string): void {
    localStorage.setItem('access_token', token);
  }

  getRefreshToken(): string | null {
    return localStorage.getItem('refresh_token');
  }

  setRefreshToken(token: string): void {
    localStorage.setItem('refresh_token', token);
  }

  clear(): void {
    localStorage.removeItem('access_token');
    localStorage.removeItem('refresh_token');
  }
}

const client = new RaisinClient('raisin://localhost:8080/sys/default', {
  tokenStorage: new LocalStorageTokenStorage(),
});
```

## Framework Integration

The SDK ships dedicated subpath exports — `@raisindb/client/react`,
`@raisindb/client/svelte`, and `@raisindb/client/vue`. None of them make the
framework a dependency: React and Vue use a "bring your own framework"
factory (you pass the module in once), Svelte gets framework-free adapters
that bind naturally to runes.

### React

```tsx
// lib/raisin-react.ts
import React from 'react';
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';
import { createRaisinReact } from '@raisindb/client/react';

export const client = new RaisinClient('ws://localhost:8081/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myapp'),
});

export const {
  RaisinProvider, useAuth, useConnection, useSql, useSubscription,
  useConversation, useConversationList,
} = createRaisinReact(React);

// App.tsx
function Chat() {
  const chat = useConversation({
    database: client.database('myrepo'),
    createOptions: { participant: '/agents/support' },
  });

  return (
    <div>
      {chat.messages.map((m, i) => <p key={i}>{m.content}</p>)}
      {chat.isStreaming && <p className="streaming">{chat.streamingText}</p>}
      <button onClick={() => chat.sendMessage('Hello!')}>Send</button>
    </div>
  );
}

export default function App() {
  return (
    <RaisinProvider client={client} repository="myrepo">
      <Chat />
    </RaisinProvider>
  );
}
```

### Svelte 5

```typescript
// lib/chat.svelte.ts
import { ConversationStore, type ConversationStoreSnapshot } from '@raisindb/client';
import { createConversationAdapter } from '@raisindb/client/svelte';
import { db } from '$lib/raisin';

const adapter = createConversationAdapter({
  database: db,
  createOptions: { participant: '/agents/support' },
});

let snapshot = $state<ConversationStoreSnapshot>(adapter.getSnapshot());
adapter.subscribe((s) => { snapshot = s; });

export const chat = {
  get messages() { return snapshot.messages; },
  get isStreaming() { return snapshot.isStreaming; },
  get streamingText() { return snapshot.streamingText; },
  get plans() { return snapshot.plans; },
  send: adapter.sendMessage,
  approvePlan: adapter.approvePlan,
  destroy: adapter.destroy,
};
```

### Vue 3

```typescript
// lib/raisin-vue.ts
import * as vue from 'vue';
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';
import { createRaisinVue } from '@raisindb/client/vue';

export const client = new RaisinClient('ws://localhost:8081/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myapp'),
});
export const db = client.database('myrepo');

export const {
  useAuth, useConnection, useSql, useSubscription,
  useConversation, useConversationList,
} = createRaisinVue(vue);
```

```vue
<!-- Chat.vue -->
<script setup lang="ts">
import { db, useConversation } from '@/lib/raisin-vue';

const chat = useConversation({
  database: db,
  createOptions: { participant: '/agents/support' },
});
// Cleanup is automatic: the composable destroys its store onUnmounted.
</script>

<template>
  <p v-for="(m, i) in chat.messages.value" :key="i">{{ m.content }}</p>
  <p v-if="chat.isStreaming.value" class="streaming">{{ chat.streamingText.value }}</p>
  <button @click="chat.sendMessage('Hello!')">Send</button>
</template>
```

### Server-Side Rendering (SSR)

For SSR environments (Next.js, Nuxt, SvelteKit server routes), use the HTTP client:

```typescript
import { RaisinClient } from '@raisindb/client';

// Create HTTP-only client for SSR
const client = RaisinClient.forSSR('http://localhost:8080', {
  tenantId: 'default',
});

// Authenticate (admin or JWT)
await client.authenticate({ username: 'admin', password: 'admin' });

// Execute queries
const result = await client.executeSql('myrepo', 'SELECT * FROM nodes LIMIT 10');
```

## API Reference

### RaisinClient

Main client class for connecting to RaisinDB.

#### Constructor

```typescript
new RaisinClient(url: string, options?: ClientOptions)
```

**Options:**
- `connection`: Connection options (auto-reconnect, heartbeat, etc.)
- `tokenStorage`: Custom token storage implementation
- `requestTimeout`: Request timeout in milliseconds (default: 30000)
- `tenantId`: Tenant ID (extracted from URL if not provided)
- `defaultBranch`: Default branch name (default: "main")

#### Methods

**Connection & Admin Auth:**
- `connect(): Promise<void>` - Connect to the server
- `authenticate(credentials: Credentials): Promise<void>` - Authenticate with credentials
- `disconnect(): void` - Disconnect from the server
- `isConnected(): boolean` - Check if connected
- `isAuthenticated(): boolean` - Check if authenticated

**Identity Auth (Email/Password):**
- `loginWithEmail(email, password, repository): Promise<IdentityUser>` - Login with email
- `registerWithEmail(email, password, repository, displayName?): Promise<IdentityUser>` - Register
- `logout(options?): Promise<void>` - Logout (options: `{ disconnect?, reconnect? }`)
- `initSession(repository): Promise<IdentityUser | null>` - Restore session from storage
- `refreshToken(): Promise<IdentityUser | null>` - Refresh access token

**Auth State Listeners:**
- `onAuthStateChange(callback): () => void` - Listen for auth events (`SIGNED_IN`, `SIGNED_OUT`, etc.)
- `onConnectionStateChange(callback): () => void` - Listen for connection state changes
- `onUserChange(callback): () => void` - Listen for user home node changes

**Session Helpers:**
- `getSession(): { user, accessToken } | null` - Get current session (sync)
- `getUser(): IdentityUser | null` - Get current user (alias)
- `getStoredUser(): IdentityUser | null` - Get stored user from localStorage
- `hasStoredToken(): boolean` - Check if token exists in storage
- `getCurrentUser(): CurrentUser | null` - Get full current user with roles/node
- `fetchUserNode(repository): Promise<UserNode | null>` - Fetch user node via SQL

**Database & Branches:**
- `database(name: string): Database` - Get database interface
- `setBranch(branch: string): void` - Set branch for subsequent requests
- `getBranch(): string` - Get current branch

### Database

Database/repository interface.

#### Methods

- `workspace(name: string): WorkspaceClient` - Get workspace client
- `workspaces(): WorkspaceManager` - Get workspace management operations
- `sql(strings, ...values): Promise<SqlResult>` - Execute SQL query with template literals
- `executeSql(query: string, params?: unknown[]): Promise<SqlResult>` - Execute raw SQL

### WorkspaceClient

Workspace operations interface.

#### Methods

- `nodes(): NodeOperations` - Get node operations
- `events(): EventSubscriptions` - Get event subscriptions

### NodeOperations

Node CRUD operations.

#### Methods

- `create(options: NodeCreateOptions): Promise<Node>` - Create a node
- `update(id: string, options: NodeUpdateOptions): Promise<Node>` - Update a node
- `delete(id: string): Promise<boolean>` - Delete a node
- `get(id: string): Promise<Node | null>` - Get a node by ID
- `query(options: NodeQueryOptions): Promise<Node[]>` - Query nodes
- `getByPath(path: string): Promise<Node | null>` - Get a node by path
- `queryByProperty(name: string, value: PropertyValue, limit?: number): Promise<Node[]>` - Query by property
- `queryByType(nodeType: string, limit?: number): Promise<Node[]>` - Query by type
- `getChildren(parentId: string, limit?: number): Promise<Node[]>` - Get children

### EventSubscriptions

Event subscription interface.

#### Methods

- `subscribe(filters: SubscriptionFilters, callback: EventCallback): Promise<Subscription>` - Subscribe to events
- `subscribeToTypes(eventTypes: string[], callback: EventCallback): Promise<Subscription>` - Subscribe to specific types
- `subscribeToPath(path: string, callback: EventCallback): Promise<Subscription>` - Subscribe to path
- `subscribeToNodeType(nodeType: string, callback: EventCallback): Promise<Subscription>` - Subscribe to node type

## Environment Support

### Browser

Works out of the box with native WebSocket support:

```typescript
import { RaisinClient } from '@raisindb/client';

const client = new RaisinClient('raisin://localhost:8080/sys/default');
```

### Node.js

Requires the `ws` package:

```bash
npm install ws
```

```typescript
import { RaisinClient } from '@raisindb/client';

const client = new RaisinClient('raisin://localhost:8080/sys/default');
```

### TypeScript

Full TypeScript support with type definitions included:

```typescript
import { RaisinClient, Node, NodeCreateOptions } from '@raisindb/client';

const options: NodeCreateOptions = {
  type: 'Page',
  path: '/home',
  properties: {
    title: 'Home',
  },
};
```

## Building

```bash
# Install dependencies
npm install

# Build
npm run build

# Watch mode
npm run dev

# Type check
npm run typecheck
```

## Stability options

Options and behaviors that keep chat, inbox, and workflow UIs responsive when
the network or server misbehaves.

### `requestTimeout` (RaisinClient / RaisinHttpClient option)

Default: `30000` ms. Applies to:

- Every WebSocket request (resolved/rejected via the internal request tracker).
- All HTTP auth calls (`loginWithEmail`, `registerWithEmail`, `refreshToken`,
  `initSession` token refresh) and `signAssetUrl` — these abort the underlying
  `fetch` after `requestTimeout` and reject with a `RaisinTimeoutError`
  (`code: 'REQUEST_TIMEOUT'`) instead of hanging forever.

Auth failures from the identity endpoints are thrown as `RaisinAuthError`
instances (real `Error` subclasses) that still carry the `{ code, message }`
fields, so existing destructuring keeps working:

```typescript
try {
  await client.loginWithEmail(email, password, 'my-repo');
} catch (err) {
  if (err instanceof RaisinAuthError) {
    console.log(err.code, err.message, err.status);
  }
}
```

### `sendMessage` inactivity timeout (per-turn SSE)

`db.conversations.sendMessage(path, text, options)` streams the assistant turn
over a single-shot SSE connection. If that stream dies mid-turn (proxy reset,
server restart, dead TCP), the turn now ends with a synthetic `waiting` event
after `inactivityTimeoutMs` of silence (default: `120000` ms, reset on every
byte received) instead of stalling the chat UI forever:

```typescript
for await (const event of db.conversations.sendMessage(path, 'Hi', {
  inactivityTimeoutMs: 60_000, // override; 0 disables
})) {
  // ... a final { type: 'waiting' } event is guaranteed even if the stream dies
}
```

The same option exists on the lower-level `SSEClient` (`inactivityTimeoutMs`).
With reconnection enabled the SSE client silently reconnects on inactivity;
with reconnection disabled the stream ends with a `RaisinTimeoutError`
(`code: 'SSE_INACTIVITY_TIMEOUT'`).

### `streamingTimeoutMs` + `watchdogIntervalMs` (ConversationStore)

`ConversationStore` adds two recovery layers on top of the SSE stream:

- `streamingTimeoutMs` (default: `120000`) — if no SSE event arrives for this
  long while a turn is streaming, the store auto-recovers (reloads messages
  and clears the streaming state).
- `watchdogIntervalMs` (default: `30000`) — while streaming, the store
  periodically asks the backend whether the turn is actually still active
  (`checkTurnHealth`) and recovers if the backend says it finished.

```typescript
const store = new ConversationStore({
  database: db,
  conversationPath,
  streamingTimeoutMs: 90_000,
  watchdogIntervalMs: 15_000,
});
```

### Request queueing during reconnects

Requests issued while the WebSocket is temporarily down are no longer thrown
away. If the connection state is `Connecting`/`Reconnecting` (or
`Disconnected` with auto-reconnect enabled after a previous successful
connection), `RaisinClient` holds the request and flushes it once the
connection is re-established **and** re-authenticated:

- Each queued request still honors its own `requestTimeout` — it rejects if
  the connection doesn't come back in time.
- The queue is bounded (100 requests); overflowing requests reject
  immediately with a "Request queue is full" error.
- If re-authentication fails after reconnect, queued requests reject with a
  `RaisinAuthError`.
- A permanent disconnect (manual `disconnect()`, max reconnect attempts
  reached) cancels everything in the queue.

### `subscription_restore_failed` event

After a reconnect, the client restores event subscriptions automatically and
retries on failure (3 retries with 1s/3s/9s backoff, without duplicating the
subscriptions that already succeeded). If restoration still fails, the client
emits `subscription_restore_failed` so the app can resync its state:

```typescript
client.on('subscription_restore_failed', (error) => {
  // Realtime events may be stale: reload lists and/or re-subscribe manually
  console.warn('Subscriptions not restored after reconnect:', error);
});

client.onReconnected(() => {
  // Fired only after connection + auth + successful subscription restore
});
```

## License

BSL-1.1 - See LICENSE file for details.

## Links

- [RaisinDB Documentation](https://raisindb.com/docs)
- [GitHub Repository](https://github.com/maravilla-labs/raisindb)
- [Issue Tracker](https://github.com/maravilla-labs/raisindb/issues)
