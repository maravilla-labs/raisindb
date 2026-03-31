# Real-time Message Notifications

RaisinDB's messaging system integrates with WebSocket event streaming to provide real-time message delivery notifications. This document explains how messages trigger notifications, how to subscribe to them, and patterns for implementing notification features.

## Overview

When a message is delivered to a user's inbox, the following happens:

1. Message created in recipient's inbox
2. NodeEvent emitted to event bus
3. WsEventHandler receives event
4. RLS (Row-Level Security) check performed
5. Event forwarded to subscribed WebSocket connections
6. Client receives real-time notification

## WebSocket Event System

RaisinDB's WebSocket transport provides event streaming with RLS filtering.

### Connection Establishment

```javascript
// Connect to WebSocket endpoint
const ws = new WebSocket('ws://localhost:8080/ws');

// Authenticate
ws.send(JSON.stringify({
  type: 'auth',
  token: 'jwt-token-here'
}));
```

### Message Event Types

Messages trigger these WebSocket events:

| Event Type | Triggered When | Payload |
|------------|----------------|---------|
| `node:created` | Message created in inbox/outbox/sent | Full message node |
| `node:updated` | Message status changed | Updated message node |
| `node:deleted` | Message deleted | Node ID and path |
| `node:property_changed` | Message property modified | Property name and new value |

## Subscribing to Message Events

### Subscribe to User's Inbox

```javascript
// Subscribe to all events in user's inbox
ws.send(JSON.stringify({
  type: 'subscribe',
  subscription_id: 'inbox-messages',
  filters: {
    workspace: 'users',
    path: 'users/user-123/inbox/*',
    event_types: ['node:created', 'node:updated'],
    node_type: 'raisin:Message'
  }
}));
```

### Subscribe to Specific Message Types

```javascript
// Subscribe only to relationship requests
ws.send(JSON.stringify({
  type: 'subscribe',
  subscription_id: 'relationship-requests',
  filters: {
    workspace: 'users',
    path: 'users/user-123/inbox/*',
    event_types: ['node:created'],
    node_type: 'raisin:Message',
    property_filters: {
      message_type: 'relationship_request'
    }
  }
}));
```

### Subscribe to All Message Folders

```javascript
// Monitor inbox, outbox, and sent folders
const folders = ['inbox', 'outbox', 'sent'];

folders.forEach(folder => {
  ws.send(JSON.stringify({
    type: 'subscribe',
    subscription_id: `messages-${folder}`,
    filters: {
      workspace: 'users',
      path: `users/user-123/${folder}/*`,
      event_types: ['node:created', 'node:updated'],
      node_type: 'raisin:Message'
    }
  }));
});
```

## Receiving Notifications

### Event Message Format

When a message event occurs, clients receive:

```javascript
{
  "type": "event",
  "subscription_id": "inbox-messages",
  "event_type": "node:created",
  "payload": {
    "tenant_id": "default",
    "repository_id": "main-repo",
    "branch": "main",
    "workspace_id": "users",
    "node_id": "msg-abc123",
    "node_type": "raisin:Message",
    "revision": "rev-xyz789",
    "path": "users/user-123/inbox/msg-abc123",
    "kind": "Created",
    "metadata": {}
  }
}
```

### Handling Notifications

```javascript
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.type === 'event') {
    switch (message.event_type) {
      case 'node:created':
        handleNewMessage(message.payload);
        break;

      case 'node:updated':
        handleMessageUpdate(message.payload);
        break;

      case 'node:deleted':
        handleMessageDeleted(message.payload);
        break;
    }
  }
};

function handleNewMessage(payload) {
  // Fetch full message details
  fetch(`/api/nodes/${payload.node_id}`)
    .then(res => res.json())
    .then(message => {
      // Show notification
      showNotification({
        title: 'New Message',
        body: message.properties.subject,
        icon: getMessageTypeIcon(message.properties.message_type),
        onClick: () => openMessage(message.id)
      });

      // Update UI
      addMessageToInbox(message);
    });
}
```

## RLS Filtering

WebSocket events are automatically filtered by RLS permissions.

### How RLS Works for Messages

From `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-transport-ws/src/event_handler.rs`:

```rust
// Fetch node for RLS evaluation
let node_for_rls = storage.nodes().get(...).await?;

// Check if user can read this node
if !rls_filter::can_perform(node, Operation::Read, auth, &scope) {
    // User cannot read - skip event
    continue;
}

// Forward event to connection
conn.send_event(event_message)?;
```

### What This Means

- Users only receive events for messages they have read access to
- Inbox messages: user is the recipient
- Sent messages: user is the sender
- System context bypasses RLS (for admin tools)

### Example Scenarios

**Scenario 1: New inbox message**
- Alice sends message to Bob
- Message created in Bob's inbox
- Event emitted
- RLS check: Can Bob read this message? → Yes (he's the recipient)
- WebSocket event sent to Bob
- Bob sees real-time notification

**Scenario 2: Other user's inbox**
- Alice tries to subscribe to Bob's inbox
- Message created in Bob's inbox
- Event emitted
- RLS check: Can Alice read this message? → No (she's not the recipient)
- Event filtered, Alice receives nothing

**Scenario 3: System admin**
- Admin subscribes to all messages
- Message created anywhere
- Event emitted
- RLS check: Is this system context? → Yes
- Event forwarded (no RLS filtering)

## Notification Patterns

### Pattern 1: Toast Notifications

Show brief notifications for new messages:

```javascript
function showNotification(message) {
  const toast = document.createElement('div');
  toast.className = 'toast notification';
  toast.innerHTML = `
    <div class="notification-icon">
      ${getMessageTypeIcon(message.properties.message_type)}
    </div>
    <div class="notification-content">
      <strong>${message.properties.subject}</strong>
      <p>From: ${message.properties.sender_id}</p>
    </div>
  `;

  document.body.appendChild(toast);

  // Auto-dismiss after 5 seconds
  setTimeout(() => toast.remove(), 5000);

  // Play notification sound
  playNotificationSound();
}
```

### Pattern 2: Badge Counts

Update unread message count in real-time:

```javascript
let unreadCount = 0;

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.event_type === 'node:created') {
    unreadCount++;
    updateBadge(unreadCount);
  }

  if (message.event_type === 'node:updated' &&
      message.payload.status === 'read') {
    unreadCount--;
    updateBadge(unreadCount);
  }
};

function updateBadge(count) {
  const badge = document.getElementById('message-badge');
  badge.textContent = count;
  badge.style.display = count > 0 ? 'block' : 'none';

  // Update document title
  document.title = count > 0
    ? `(${count}) Messages - RaisinDB`
    : 'Messages - RaisinDB';
}
```

### Pattern 3: In-App Message List

Update message list in real-time:

```javascript
class MessageList {
  constructor() {
    this.messages = [];
    this.setupWebSocket();
  }

  setupWebSocket() {
    ws.onmessage = (event) => {
      const msg = JSON.parse(event.data);

      if (msg.event_type === 'node:created') {
        this.addMessage(msg.payload);
      } else if (msg.event_type === 'node:updated') {
        this.updateMessage(msg.payload);
      } else if (msg.event_type === 'node:deleted') {
        this.removeMessage(msg.payload.node_id);
      }
    };
  }

  addMessage(payload) {
    // Fetch full message
    fetch(`/api/nodes/${payload.node_id}`)
      .then(res => res.json())
      .then(message => {
        this.messages.unshift(message);
        this.render();
      });
  }

  updateMessage(payload) {
    const index = this.messages.findIndex(m => m.id === payload.node_id);
    if (index !== -1) {
      // Fetch updated message
      fetch(`/api/nodes/${payload.node_id}`)
        .then(res => res.json())
        .then(message => {
          this.messages[index] = message;
          this.render();
        });
    }
  }

  removeMessage(nodeId) {
    this.messages = this.messages.filter(m => m.id !== nodeId);
    this.render();
  }

  render() {
    // Update UI
  }
}
```

### Pattern 4: Presence Indicators

Show when users are online:

```javascript
// Track online users
const onlineUsers = new Set();

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);

  if (msg.type === 'user_connected') {
    onlineUsers.add(msg.user_id);
    updatePresence(msg.user_id, 'online');
  } else if (msg.type === 'user_disconnected') {
    onlineUsers.delete(msg.user_id);
    updatePresence(msg.user_id, 'offline');
  }
};

function updatePresence(userId, status) {
  const indicator = document.querySelector(`[data-user="${userId}"] .presence`);
  if (indicator) {
    indicator.className = `presence ${status}`;
  }
}
```

## Push Notifications (Future)

While not yet implemented, the messaging system is designed to support push notifications:

### Browser Push API

```javascript
// Request permission
const permission = await Notification.requestPermission();

if (permission === 'granted') {
  // Subscribe to push notifications
  const registration = await navigator.serviceWorker.ready;
  const subscription = await registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: PUBLIC_VAPID_KEY
  });

  // Send subscription to server
  await fetch('/api/push/subscribe', {
    method: 'POST',
    body: JSON.stringify(subscription)
  });
}
```

### Server-side Push (Proposed)

```javascript
// In message handler
async function handleNewMessage(message) {
  // Deliver to inbox
  await createNode({
    parent_path: `users/${message.recipient_id}/inbox`,
    node_type: 'raisin:Message',
    properties: message
  });

  // Send push notification if user is offline
  const isOnline = await checkUserOnline(message.recipient_id);
  if (!isOnline) {
    await sendPushNotification(message.recipient_id, {
      title: 'New Message',
      body: message.subject,
      icon: '/icons/message.png',
      tag: `message-${message.id}`,
      data: {
        url: `/messages/${message.id}`
      }
    });
  }
}
```

## Email Notifications (Future)

Email notifications for messages can be implemented similarly:

### Email Preferences

```javascript
// User notification preferences
{
  email_notifications: {
    enabled: true,
    frequency: 'immediate',  // or 'daily_digest', 'weekly_digest'
    message_types: [
      'relationship_request',
      'ward_invitation'
    ]
  }
}
```

### Email Template

```javascript
async function sendEmailNotification(message) {
  const recipient = await getUser(message.recipient_id);

  if (!recipient.email_notifications?.enabled) {
    return; // User has disabled email notifications
  }

  }

  await sendEmail({
    to: recipient.email,
    subject: `New Message: ${message.subject}`,
    template: 'message-notification',
    data: {
      message,
      view_url: `https://app.example.com/messages/${message.id}`
    }
  });
}
```

## Performance Considerations

### Connection Limits

- Each WebSocket connection consumes server resources
- Recommend: 1 connection per user session
- Multiplex all subscriptions over single connection

### Subscription Limits

- Limit subscriptions per connection (e.g., 50)
- Prefer broader subscriptions over many narrow ones

```javascript
// Bad: Many narrow subscriptions
subscribe({ path: 'users/123/inbox/msg1' });
subscribe({ path: 'users/123/inbox/msg2' });
subscribe({ path: 'users/123/inbox/msg3' });
// ...

// Good: One broad subscription
subscribe({ path: 'users/123/inbox/*' });
```

### Event Volume

For high-volume scenarios:

1. **Batching**: Group events into batches
2. **Throttling**: Limit event rate per subscription
3. **Sampling**: Send only subset of events

```javascript
// Throttle notifications to max 1 per second
let lastNotification = 0;

ws.onmessage = (event) => {
  const now = Date.now();
  if (now - lastNotification < 1000) {
    // Too soon, queue for later
    queueNotification(event);
  } else {
    showNotification(event);
    lastNotification = now;
  }
};
```

## Offline Support

Handle connection interruptions:

```javascript
class ResilientWebSocket {
  constructor(url) {
    this.url = url;
    this.connect();
  }

  connect() {
    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      console.log('Connected');
      this.resubscribe();
    };

    this.ws.onclose = () => {
      console.log('Disconnected, reconnecting...');
      setTimeout(() => this.connect(), 5000);
    };

    this.ws.onerror = (error) => {
      console.error('WebSocket error:', error);
    };
  }

  resubscribe() {
    // Re-establish subscriptions after reconnect
    this.subscriptions.forEach(sub => {
      this.ws.send(JSON.stringify({
        type: 'subscribe',
        ...sub
      }));
    });

    // Fetch missed messages
    this.fetchMissedMessages();
  }

  async fetchMissedMessages() {
    const lastSeen = localStorage.getItem('last_message_timestamp');
    const response = await fetch(`/api/messages?since=${lastSeen}`);
    const messages = await response.json();

    messages.forEach(msg => this.handleMessage(msg));
  }
}
```

## Testing Notifications

### Manual Testing

```javascript
// Create test message
await fetch('/api/nodes', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    parent_path: 'users/bob/outbox',
    node_type: 'raisin:Message',
    properties: {
      message_type: 'chat',
      recipient_id: 'user-alice',
      sender_id: 'user-bob',
      subject: 'Test notification',
      body: { message_text: 'Testing WebSocket notifications' }
    }
  })
});

// Expected: Alice's WebSocket receives event
// Expected: Alice sees notification
```

### Automated Testing

```javascript
describe('Message Notifications', () => {
  it('should receive notification when message created', async () => {
    const ws = new WebSocketClient();
    await ws.connect();
    await ws.authenticate('alice-token');

    // Subscribe to inbox
    await ws.subscribe({
      subscription_id: 'inbox',
      filters: { path: 'users/alice/inbox/*' }
    });

    // Create message
    const messagePromise = ws.waitForEvent('node:created');

    await createMessage({
      recipient_id: 'user-alice',
      message_type: 'chat',
      subject: 'Test'
    });

    // Verify event received
    const event = await messagePromise;
    expect(event.event_type).toBe('node:created');
    expect(event.payload.node_type).toBe('raisin:Message');
  });
});
```

## Security Considerations

### Authentication Required

- All WebSocket connections must be authenticated
- Unauthenticated connections receive no events

### RLS Always Applied

- Events are filtered by Row-Level Security
- Users only receive events they have permission to see

### Subscription Validation

- Server validates all subscription filters
- Invalid filters are rejected
- Path traversal attacks prevented

### Rate Limiting

Consider rate limiting to prevent abuse:

```javascript
// Limit subscription rate
const MAX_SUBSCRIPTIONS_PER_MINUTE = 60;

if (user.subscriptions_this_minute > MAX_SUBSCRIPTIONS_PER_MINUTE) {
  ws.send(JSON.stringify({
    type: 'error',
    code: 'RATE_LIMIT_EXCEEDED',
    message: 'Too many subscriptions'
  }));
  return;
}
```

## Best Practices

### 1. Always Handle Reconnection

Network connections are unreliable. Always implement reconnection logic.

### 2. Fetch Missing Data on Reconnect

After reconnecting, fetch any messages created while offline.

### 3. Use Subscription IDs

Always provide meaningful subscription IDs for debugging:

```javascript
subscribe({
  subscription_id: 'inbox-messages',  // Good
  // Not: subscription_id: 'sub1'      // Bad
});
```

### 4. Unsubscribe When Done

Clean up subscriptions when components unmount:

```javascript
useEffect(() => {
  ws.subscribe({ subscription_id: 'inbox', ... });

  return () => {
    ws.unsubscribe('inbox');  // Clean up
  };
}, []);
```

### 5. Debounce UI Updates

Avoid updating UI on every event:

```javascript
const debouncedUpdate = debounce(() => {
  updateMessageList();
}, 500);

ws.onmessage = () => {
  debouncedUpdate();
};
```

## File References

- WebSocket event handler: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-transport-ws/src/event_handler.rs`
- WebSocket README: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-transport-ws/README.md`
- RLS filter service: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/src/services/rls_filter.rs`
