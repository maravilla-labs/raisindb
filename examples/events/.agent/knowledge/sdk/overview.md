# RaisinDB JavaScript SDK

## Installation

```bash
npm install @raisindb/client
```

## Connection

The SDK connects via WebSocket. URL format: `ws://host:port/sys/tenant/repository`

```typescript
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';

const client = new RaisinClient('ws://localhost:8081/sys/default/my-repo', {
  tokenStorage: new LocalStorageTokenStorage('my-repo'),  // persist auth across reloads
  tenantId: 'default',
  defaultBranch: 'main',
  connection: {
    autoReconnect: true,
    heartbeatInterval: 30000,
  },
  requestTimeout: 30000,
});
```

For secure connections use `wss://` instead of `ws://`.

## Session Initialization

Call once on app start to restore saved auth tokens and connect:

```typescript
const user = await client.initSession('my-repo');
// Returns the authenticated IdentityUser or null (anonymous)

if (!user && !client.isConnected()) {
  await client.connect();  // connect for anonymous access
}
```

### Connection Gate Pattern

Block queries until the session is ready:

```typescript
let connectionResolve: (() => void) | null = null;
const connectionPromise = new Promise<void>(resolve => { connectionResolve = resolve; });

async function initSession() {
  const user = await client.initSession('my-repo');
  connectionResolve?.();  // open the gate
  return user;
}

async function getDatabase() {
  await connectionPromise;  // wait until session is initialized
  return client.database('my-repo');
}
```

## Authentication

### Admin Mode

Direct admin authentication (for server-side scripts or dev):

```typescript
await client.authenticate({ username: 'admin', password: 'admin' });
```

### Identity Auth (Email/Password)

For end-user authentication with JWT:

```typescript
// Register a new user
const user = await client.registerWithEmail('user@example.com', 'password', 'my-repo', 'Display Name');

// Login
const user = await client.loginWithEmail('user@example.com', 'password', 'my-repo');

// Restore session on page reload (reads from tokenStorage)
const user = await client.initSession('my-repo');

// Logout (clears tokens, reconnects as anonymous)
await client.logout();
```

### Auth State Changes

React to authentication events (Firebase/Supabase-compatible pattern):

```typescript
const unsubscribe = client.onAuthStateChange(({ event, session }) => {
  // event: 'SIGNED_IN' | 'SIGNED_OUT' | 'TOKEN_REFRESHED' | 'SESSION_EXPIRED' | 'USER_UPDATED'
  if (event === 'SIGNED_IN') {
    console.log('Signed in:', session.user?.email);
  }
});
```

### User Home Path

Authenticated users have a home node in `raisin:access_control`:

```typescript
const user = client.getUser();
// user.home = '/raisin:access_control/users/abc123'
// user.email = 'user@example.com'
```

## Core Concepts

### Database and Workspace

All content operations are scoped to a database (repository) and workspace:

```typescript
const db = client.database('my-repo');
const ws = db.workspace('content');
```

- **Database**: Represents a repository. Provides access to SQL, workspaces, node types, flows.
- **Workspace**: A content namespace within a database. Provides node CRUD, events, transactions.

### SQL Queries

```typescript
// Execute parameterized SQL
const result = await db.executeSql(
  "SELECT * FROM 'workspace' WHERE node_type = $1 AND properties ->> 'status' = $2",
  ['myapp:Article', 'published']
);
const rows = result.rows ?? [];
```

### Helper Pattern

```typescript
async function query<T>(sql: string, params?: unknown[]): Promise<T[]> {
  const db = await getDatabase();
  const result = await db.executeSql(sql, params);
  return (result.rows ?? []) as T[];
}

async function queryOne<T>(sql: string, params?: unknown[]): Promise<T | null> {
  const rows = await query<T>(sql, params);
  return rows[0] ?? null;
}
```

### Client State

```typescript
client.isConnected();    // WebSocket connected
client.isReady();        // Connected AND authenticated (or anonymous)
client.getUser();        // Current user info or null
client.getSession();     // { user, accessToken } or null

const unsubscribe = client.onReadyStateChange((ready) => {
  // Use for UI connection indicators
});

const unsubscribe = client.onReconnected(() => {
  // Fired after reconnection + subscription restoration
  // Use to refresh application data
});
```

### SSR / HTTP Client

For server-side rendering without WebSocket:

```typescript
const httpClient = RaisinClient.forSSR('http://localhost:8080', { tenantId: 'default' });
await httpClient.authenticate({ username: 'admin', password: 'admin' });
const result = await httpClient.database('my-repo').executeSql('SELECT * FROM content');
```
