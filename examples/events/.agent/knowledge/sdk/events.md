# SDK: Real-Time Events

Subscribe to real-time node change events via WebSocket.

## Setup

```typescript
const db = client.database('my-repo');
const events = db.workspace('content').events();
```

## Subscribe to Events

### By path

```typescript
const sub = await events.subscribeToPath('/pages/*', (event) => {
  console.log(event.event_type, event.payload);
});

// Include full node data in the event payload
const sub = await events.subscribeToPath('/pages/*', callback, { includeNode: true });
```

### By node type

```typescript
const sub = await events.subscribeToNodeType('Page', (event) => {
  console.log('Page changed:', event.event_type);
});
```

### By event type

```typescript
const sub = await events.subscribeToTypes(['node:created', 'node:updated'], (event) => {
  console.log(event.event_type, event.payload);
});
```

### Custom filters

```typescript
const sub = await events.subscribe(
  { path: '/pages/*', event_types: ['node:updated'], node_type: 'Page' },
  (event) => { /* ... */ }
);
```

## Event Types

- `node:created` - A node was created
- `node:updated` - A node's properties or content changed
- `node:deleted` - A node was deleted

## Unsubscribe

```typescript
await sub.unsubscribe();

// Check if still active
sub.isActive();
```

## Deduplication

The SDK automatically deduplicates events:
- Multiple subscriptions with identical filters share a single server-side subscription
- Events are deduplicated within a 5-second window to prevent duplicate delivery during reconnection

## Reconnection

After a network disconnection, the SDK automatically:
1. Reconnects the WebSocket
2. Re-authenticates (if using JWT)
3. Restores all active subscriptions

Use `client.onReconnected()` to refresh application data after reconnection:

```typescript
client.onReconnected(() => {
  // Subscriptions are already restored - refresh any cached data
  refreshData();
});
```
