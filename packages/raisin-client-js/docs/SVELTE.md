# Svelte 5 Integration Guide

The RaisinDB client SDK includes a full Svelte 5 integration: adapter factories for auth, connection, SQL queries, event subscriptions, conversations, and flows, plus a context pattern for sharing the client across your component tree.

Unlike React hooks, Svelte 5 uses runes (`$state`, `$effect`, `$derived`). The SDK provides **plain `.ts` adapter factories** that return `{ subscribe, getSnapshot, ...actions, destroy }` objects. You bind these to `$state` in your own `.svelte.ts` files for full reactivity.

## Quick Start

### 1. Set up `lib/raisin.ts`

```ts
import { RaisinClient, LocalStorageTokenStorage } from '@raisindb/client';

export const client = new RaisinClient('wss://localhost:8443/sys/default/myrepo', {
  tokenStorage: new LocalStorageTokenStorage('myrepo'),
  tenantId: 'default',
  defaultBranch: 'main',
});

export const db = client.database('myrepo');
```

### 2. Set up context in `+layout.svelte`

```svelte
<script>
  import { setContext } from 'svelte';
  import { RAISIN_CONTEXT_KEY, type RaisinContext } from '@raisindb/client';
  import { client } from '$lib/raisin';

  let { children } = $props();

  setContext<RaisinContext>(RAISIN_CONTEXT_KEY, { client, repository: 'myrepo' });
</script>

{@render children()}
```

### 3. Create reactive adapters in `.svelte.ts` files

```typescript
// lib/stores/auth.svelte.ts
import { createAuthAdapter } from '@raisindb/client';
import { client } from '$lib/raisin';

const adapter = createAuthAdapter(client);
let snapshot = $state(adapter.getSnapshot());
adapter.subscribe(s => { snapshot = s; });

export const auth = {
  get user() { return snapshot.user; },
  get isAuthenticated() { return snapshot.isAuthenticated; },
  get isLoading() { return snapshot.isLoading; },
  login: adapter.login,
  register: adapter.register,
  logout: adapter.logout,
  initSession: adapter.initSession,
};
```

### 4. Use in components

```svelte
<script>
  import { auth } from '$lib/stores/auth.svelte';

  const user = $derived(auth.user);
</script>

{#if auth.isLoading}
  <Spinner />
{:else if !user}
  <LoginPage />
{:else}
  <Dashboard {user} />
{/if}
```

---

## Context Provider

Svelte's `setContext`/`getContext` must be called during component initialization. The SDK provides a context key and type — you call the Svelte APIs yourself.

### Setup (`+layout.svelte`)

```svelte
<script>
  import { setContext } from 'svelte';
  import { RAISIN_CONTEXT_KEY, type RaisinContext } from '@raisindb/client';
  import { client } from '$lib/raisin';

  let { children } = $props();
  setContext<RaisinContext>(RAISIN_CONTEXT_KEY, { client, repository: 'myrepo' });
</script>

{@render children()}
```

### Consuming (`+page.svelte` or any child)

```svelte
<script>
  import { getContext } from 'svelte';
  import { RAISIN_CONTEXT_KEY, type RaisinContext } from '@raisindb/client';

  const { client, repository } = getContext<RaisinContext>(RAISIN_CONTEXT_KEY);
  const db = client.database(repository!);
</script>
```

### Exports

| Export | Type | Description |
|--------|------|-------------|
| `RAISIN_CONTEXT_KEY` | `symbol` | Unique symbol for `setContext`/`getContext` |
| `RaisinContext` | `interface` | `{ client: RaisinClient, repository?: string }` |

---

## Authentication (`createAuthAdapter`)

```typescript
import { createAuthAdapter } from '@raisindb/client';

const adapter = createAuthAdapter(client);
```

Subscribes to `client.onAuthStateChange()` and `client.onUserChange()` for reactive updates.

### Snapshot

| Field | Type | Description |
|-------|------|-------------|
| `user` | `IdentityUser \| null` | Current user |
| `isAuthenticated` | `boolean` | Whether a user is logged in |
| `isLoading` | `boolean` | During login/register/logout/initSession |

### Actions

| Method | Signature | Description |
|--------|-----------|-------------|
| `login` | `(email, password, repository) => Promise<IdentityUser>` | Log in |
| `register` | `(email, password, repository, displayName?) => Promise<IdentityUser>` | Register |
| `logout` | `(options?) => Promise<void>` | Log out |
| `initSession` | `(repository) => Promise<IdentityUser \| null>` | Resume session |
| `destroy` | `() => void` | Unsubscribe and clean up |

### Login Form Example

```svelte
<script>
  import { auth } from '$lib/stores/auth.svelte';

  let email = $state('');
  let password = $state('');
  let error = $state('');

  async function handleSubmit() {
    try {
      await auth.login(email, password, 'myrepo');
    } catch (err) {
      error = err.message;
    }
  }
</script>

<form onsubmit={handleSubmit}>
  <input bind:value={email} />
  <input type="password" bind:value={password} />
  <button disabled={auth.isLoading}>Log in</button>
  {#if error}<p class="error">{error}</p>{/if}
</form>
```

---

## Connection State (`createConnectionAdapter`)

```typescript
import { createConnectionAdapter } from '@raisindb/client';

const adapter = createConnectionAdapter(client);
```

Tracks WebSocket connection state and ready state.

### Snapshot

| Field | Type | Description |
|-------|------|-------------|
| `state` | `ConnectionState` | Current connection state |
| `isConnected` | `boolean` | WebSocket is open |
| `isReady` | `boolean` | Connected AND authenticated (or no stored token) |

### Actions

| Method | Signature | Description |
|--------|-----------|-------------|
| `connect` | `() => Promise<void>` | Open connection |
| `disconnect` | `() => void` | Close connection |
| `destroy` | `() => void` | Unsubscribe and clean up |

### Connection Indicator

```svelte
<script>
  import { connection } from '$lib/stores/connection.svelte';
</script>

<span class={connection.isReady ? 'text-green-500' : 'text-red-500'}>
  {connection.state}
</span>
```

---

## SQL Queries (`createSqlAdapter`)

```typescript
import { createSqlAdapter } from '@raisindb/client';

const adapter = createSqlAdapter<Post>(db, "SELECT * FROM content", [], options?);
```

Takes a `Database` instance (from `client.database(repo)`), not the client itself.

### Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enabled` | `boolean` | `true` | Skip execution when false |
| `refetchOnReconnect` | `boolean` | `true` | Auto-refetch after reconnection |
| `realtime` | `object` | - | Subscribe to events and auto-refetch |
| `realtime.workspace` | `string` | - | Workspace to subscribe to |
| `realtime.eventTypes` | `string[]` | - | Filter event types |
| `realtime.path` | `string` | - | Filter by path pattern |
| `realtime.nodeType` | `string` | - | Filter by node type |

### Snapshot

| Field | Type | Description |
|-------|------|-------------|
| `data` | `T[] \| null` | Query results |
| `isLoading` | `boolean` | Whether query is in-flight |
| `error` | `Error \| null` | Last error |

### Actions

| Method | Signature | Description |
|--------|-----------|-------------|
| `refetch` | `() => Promise<void>` | Re-execute the query |
| `destroy` | `() => void` | Clean up subscriptions |

### Real-time Post List

```typescript
// lib/stores/posts.svelte.ts
import { createSqlAdapter } from '@raisindb/client';
import { db } from '$lib/raisin';

const adapter = createSqlAdapter<{ title: string; path: string }>(
  db,
  "SELECT * FROM content WHERE node_type = 'Post' ORDER BY created_at DESC",
  [],
  { realtime: { workspace: 'content', nodeType: 'Post' } },
);

let snapshot = $state(adapter.getSnapshot());
adapter.subscribe(s => { snapshot = s; });

export const posts = {
  get data() { return snapshot.data; },
  get isLoading() { return snapshot.isLoading; },
  get error() { return snapshot.error; },
  refetch: adapter.refetch,
};
```

```svelte
<script>
  import { posts } from '$lib/stores/posts.svelte';
</script>

{#if posts.isLoading}
  <Spinner />
{:else if posts.data}
  {#each posts.data as post}
    <PostCard title={post.title} />
  {/each}
{/if}
```

### Parameterized Query

```typescript
const adapter = createSqlAdapter<Post>(
  db,
  "SELECT * FROM content WHERE properties->>'slug'::String = $1",
  [slug],
);
```

---

## Event Subscriptions (`createSubscriptionAdapter`)

```typescript
import { createSubscriptionAdapter } from '@raisindb/client';

const sub = createSubscriptionAdapter(db, filters, callback);
```

Pure side-effect — subscribes to database events and invokes the callback. No snapshot state.

### API

| Method | Signature | Description |
|--------|-----------|-------------|
| `destroy` | `() => void` | Unsubscribe |

### Toast Notifications

```svelte
<script>
  import { onDestroy } from 'svelte';
  import { createSubscriptionAdapter } from '@raisindb/client';
  import { db } from '$lib/raisin';
  import { toast } from '$lib/toast';

  const sub = createSubscriptionAdapter(
    db,
    { workspace: 'content', node_type: 'Post', include_node: true },
    (event) => { toast(`New post: ${event.payload?.name}`); },
  );

  onDestroy(() => sub.destroy());
</script>
```

---

## Conversations (`createConversationAdapter` / `createConversationListAdapter`)

Same API as documented in the main SDK — these adapters wrap `ConversationStore` and `ConversationListStore`.

### AI Chat

```typescript
// lib/stores/chat.svelte.ts
import { createConversationAdapter } from '@raisindb/client';
import { db } from '$lib/raisin';

const adapter = createConversationAdapter({
  database: db,
  conversationPath: '/home/conversations/support-chat',
});

let snapshot = $state(adapter.getSnapshot());
adapter.subscribe(s => { snapshot = s; });

export const chat = {
  get messages() { return snapshot.messages; },
  get isStreaming() { return snapshot.isStreaming; },
  get streamingText() { return snapshot.streamingText; },
  sendMessage: adapter.sendMessage,
  loadMessages: adapter.loadMessages,
  stop: adapter.stop,
  destroy: adapter.destroy,
};
```

```svelte
<script>
  import { chat } from '$lib/stores/chat.svelte';
  import { onMount, onDestroy } from 'svelte';

  let input = $state('');

  onMount(() => chat.loadMessages());
  onDestroy(() => chat.destroy());

  function send() {
    chat.sendMessage(input);
    input = '';
  }
</script>

{#each chat.messages as msg}
  <div>{msg.content}</div>
{/each}
{#if chat.isStreaming}
  <p class="streaming">{chat.streamingText}</p>
{/if}
<input bind:value={input} onkeydown={(e) => e.key === 'Enter' && send()} />
```

---

## Flows (`createFlowAdapter`)

```typescript
import { createFlowAdapter } from '@raisindb/client';

const adapter = createFlowAdapter({ database: db });
```

### Snapshot

| Field | Type | Description |
|-------|------|-------------|
| `events` | `FlowExecutionEvent[]` | Collected events |
| `status` | `FlowStatus` | `'idle' \| 'running' \| 'waiting' \| 'completed' \| 'failed'` |
| `isRunning` | `boolean` | Whether a flow is running |
| `error` | `string \| null` | Error message |
| `output` | `unknown \| null` | Flow output on completion |
| `instanceId` | `string \| null` | Current instance ID |

### Actions

| Method | Signature | Description |
|--------|-----------|-------------|
| `run` | `(flowPath, input?) => Promise<void>` | Start a flow |
| `resume` | `(data) => Promise<void>` | Resume a waiting flow |
| `reset` | `() => void` | Reset state |
| `destroy` | `() => void` | Abort and clean up |

### Flow Runner

```typescript
// lib/stores/order-flow.svelte.ts
import { createFlowAdapter } from '@raisindb/client';
import { db } from '$lib/raisin';

const adapter = createFlowAdapter({ database: db });
let snapshot = $state(adapter.getSnapshot());
adapter.subscribe(s => { snapshot = s; });

export const orderFlow = {
  get events() { return snapshot.events; },
  get status() { return snapshot.status; },
  get isRunning() { return snapshot.isRunning; },
  get error() { return snapshot.error; },
  get output() { return snapshot.output; },
  run: adapter.run,
  resume: adapter.resume,
  reset: adapter.reset,
};
```

```svelte
<script>
  import { orderFlow } from '$lib/stores/order-flow.svelte';
</script>

<button
  onclick={() => orderFlow.run('/flows/process-order', { orderId: '123' })}
  disabled={orderFlow.isRunning}
>
  Process Order
</button>
<p>Status: {orderFlow.status}</p>
{#if orderFlow.status === 'waiting'}
  <button onclick={() => orderFlow.resume({ approved: true })}>Approve</button>
{/if}
{#if orderFlow.error}
  <p class="error">{orderFlow.error}</p>
{/if}
```

---

## Common Patterns

### Auth Gate

```svelte
<!-- +layout.svelte -->
<script>
  import { auth } from '$lib/stores/auth.svelte';
  import { connection } from '$lib/stores/connection.svelte';
  import { onMount } from 'svelte';

  let { children } = $props();

  onMount(() => { auth.initSession('myrepo'); });
</script>

{#if auth.isLoading || !connection.isReady}
  <Spinner />
{:else if !auth.user}
  <LoginPage />
{:else}
  {@render children()}
{/if}
```

### Real-time Dashboard

```typescript
// lib/stores/dashboard.svelte.ts
import { createSqlAdapter, createSubscriptionAdapter } from '@raisindb/client';
import { db } from '$lib/raisin';

const statsAdapter = createSqlAdapter<{ total: number; node_type: string }>(
  db,
  "SELECT COUNT(*) as total, node_type FROM content GROUP BY node_type",
  [],
  { realtime: { workspace: 'content' } },
);

let snapshot = $state(statsAdapter.getSnapshot());
statsAdapter.subscribe(s => { snapshot = s; });

export const dashboard = {
  get stats() { return snapshot.data; },
  get isLoading() { return snapshot.isLoading; },
  destroy: statsAdapter.destroy,
};
```

### Conditional Queries with `$effect`

For queries that depend on reactive state (e.g., auth user), create adapters inside `$effect`:

```svelte
<script>
  import { createSqlAdapter } from '@raisindb/client';
  import { auth } from '$lib/stores/auth.svelte';
  import { db } from '$lib/raisin';

  let profile = $state(null);

  $effect(() => {
    if (!auth.user?.home) return;

    const adapter = createSqlAdapter(
      db,
      "SELECT * FROM 'raisin:access_control' WHERE path = $1",
      [auth.user.home],
    );
    adapter.subscribe(s => { profile = s.data?.[0] ?? null; });

    return () => adapter.destroy();
  });
</script>

{#if profile}
  <ProfileCard {profile} />
{/if}
```

### `createSubscriber` Tip

Svelte 5's `createSubscriber` (from `svelte/reactivity`) can bridge external subscriptions to runes as an alternative to the `subscribe` + `$state` pattern:

```typescript
// lib/stores/auth.svelte.ts
import { createSubscriber } from 'svelte/reactivity';
import { createAuthAdapter } from '@raisindb/client';
import { client } from '$lib/raisin';

const adapter = createAuthAdapter(client);
const subscribe = createSubscriber((update) => {
  return adapter.subscribe(() => update());
});

export const auth = {
  get user() { subscribe(); return adapter.getSnapshot().user; },
  get isAuthenticated() { subscribe(); return adapter.getSnapshot().isAuthenticated; },
  get isLoading() { subscribe(); return adapter.getSnapshot().isLoading; },
  login: adapter.login,
  register: adapter.register,
  logout: adapter.logout,
  initSession: adapter.initSession,
};
```

This approach avoids the intermediate `$state` variable — each getter reads directly from the adapter's snapshot, and `createSubscriber` ensures Svelte re-renders when the snapshot changes.
