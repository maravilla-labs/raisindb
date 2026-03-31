# Messaging System Documentation

Welcome to the RaisinDB Messaging System documentation. This system provides an inbox/outbox pattern for inter-user communication, workflow notifications, and relationship management.

## Overview

The messaging system enables:

- **User-to-user messaging**: Direct communication between users
- **Relationship workflows**: Request and accept relationships (parent-child, guardian-ward, etc.)
- **Task assignment**: Assign tasks and track completion
- **System notifications**: Automated alerts and updates
- **Real-time delivery**: WebSocket-based instant notifications

## Documentation Structure

### 1. [Architecture](./architecture.md)

Understand the overall messaging architecture:
- Inbox/outbox/sent folder structure
- Message lifecycle and state transitions
- Trigger-based routing and processing
- Asynchronous execution model
- Error handling and retries
- Access control and RLS integration

**Start here** if you're new to the messaging system.

### 2. [Message Types](./message-types.md)

Learn about built-in message types:
- `relationship_request` - Request relationships between users
- `relationship_response` - Accept/reject relationship requests
- `ward_invitation` - Create ward accounts and establish guardianship
- `stewardship_request` - Request delegation permissions
- `system_notification` - System-generated alerts
- `chat` - Person-to-person messaging

Each message type includes:
- When to use it
- Body schema specification
- Processing flow
- Expected outcomes
- Example messages

### 3. [Triggers](./triggers.md)

Understand the trigger-based processing system:
- How triggers work
- Router trigger (on-outbox-create)
- Message type handlers
- Priority and ordering
- Filter matching (path, node type, properties)
- Job queue integration
- Monitoring and testing

### 4. [Notifications](./notifications.md)

Implement real-time message delivery:
- WebSocket event integration
- RLS filtering for security
- Subscription patterns
- UI notification patterns (toasts, badges, lists)
- Offline support and reconnection
- Push notifications (future)
- Email notifications (future)

### 5. [Extending](./extending.md)

Add custom message types:
- Step-by-step guide with complete example
- Message type schema definition
- Processing function implementation
- Trigger creation and registration
- Testing strategies
- Best practices and patterns
- Troubleshooting

### 6. [Implementation Status](./implementation-status.md)

Track what's implemented vs planned:
- Node types status
- Message type handlers status
- Function bindings status
- External notifications status
- File references for all components

## Quick Start

### Sending a Message

```javascript
// Create message in sender's outbox
const message = await nodeService.createNode({
  parent_path: 'users/alice/outbox',
  node_type: 'raisin:Message',
  properties: {
    message_type: 'chat',
    subject: 'Hello Bob!',
    recipient_id: 'user-bob',
    sender_id: 'user-alice',
    status: 'pending',
    body: {
      message_text: 'Hey, how are you?'
    }
  }
});

// Message is automatically:
// 1. Routed by on-outbox-create trigger
// 2. Copied to alice's sent folder
// 3. Delivered to bob's inbox
// 4. WebSocket notification sent to bob
```

### Subscribing to Messages

```javascript
// Connect to WebSocket
const ws = new WebSocket('ws://localhost:8080/ws');

// Authenticate
ws.send(JSON.stringify({
  type: 'auth',
  token: userToken
}));

// Subscribe to inbox messages
ws.send(JSON.stringify({
  type: 'subscribe',
  subscription_id: 'inbox-messages',
  filters: {
    workspace: 'users',
    path: 'users/user-123/inbox/*',
    event_types: ['node:created'],
    node_type: 'raisin:Message'
  }
}));

// Handle notifications
ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);
  if (msg.type === 'event') {
    showNotification('New message received!');
  }
};
```

### Creating a Custom Message Type

See [extending.md](./extending.md) for a complete walkthrough, but the basic steps are:

1. Define message type schema in documentation
2. Create processing function
3. Create trigger with property filter for your message type
4. Register trigger in package manifest
5. Test end-to-end flow

## Key Concepts

### Inbox/Outbox Pattern

Users have three message folders:
- **inbox**: Incoming messages
- **outbox**: Outgoing messages being processed
- **sent**: Archive of sent messages

Messages flow: `outbox → router → type handler → recipient's inbox`

### Event-Driven Processing

The system is fully asynchronous:
- Creating a message returns immediately
- Triggers fire based on node events
- Handlers run as background jobs
- Eventual consistency model

### Message Lifecycle

```
pending → sent → delivered → read → processed
```

Status changes trigger different handlers at each stage.

### RLS Security

All messages respect Row-Level Security:
- Users can only read their own messages
- WebSocket events are filtered by permissions
- System context bypasses RLS for processing

## Common Use Cases

### 1. Relationship Requests

```javascript
await createNode({
  parent_path: 'users/parent/outbox',
  node_type: 'raisin:Message',
  properties: {
    message_type: 'relationship_request',
    recipient_id: 'user-child',
    sender_id: 'user-parent',
    body: {
      relationship_type: 'parent-of',
      message: 'Hi! I would like to connect as your parent.'
    }
  }
});
```

### 2. Task Assignment

```javascript
await createNode({
  parent_path: 'users/manager/outbox',
  node_type: 'raisin:Message',
  properties: {
    message_type: 'task_assignment',
    recipient_id: 'user-employee',
    sender_id: 'user-manager',
    body: {
      task_title: 'Review Budget',
      due_date: '2025-12-31T23:59:59Z',
      priority: 'high'
    }
  }
});
```

### 3. System Notifications

```javascript
await createNode({
  parent_path: 'users/alice/inbox',  // Create directly in inbox
  node_type: 'raisin:Message',
  properties: {
    message_type: 'system_notification',
    recipient_id: 'user-alice',
    sender_id: 'system',
    status: 'delivered',
    body: {
      notification_type: 'security_alert',
      title: 'New Login Detected',
      body: 'A new device logged into your account.',
      priority: 4
    }
  }
});
```

## Related Documentation

- [Row-Level Security (RLS)](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/ROW_LEVEL_SECURITY.md)
- [REL (Relationship Expression Language)](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/REL.md)
- [Event-Driven Architecture](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/EVENT_DRIVEN_ARCHITECTURE.md)
- [WebSocket Transport](/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-transport-ws/README.md)

## File References

### Core Node Types
- Message: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_message.yaml`
- MessageFolder: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_message_folder.yaml`
- Conversation: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/nodetypes/conversation.yaml`
- User: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_user.yaml`
- Trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_trigger.yaml`

### Messaging Package
- Package manifest: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/manifest.yaml`
- Router trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/on-outbox-create/.node.yaml`
- Chat handler trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-chat/.node.yaml`
- Task assignment trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-task-assignment/.node.yaml`
- System notification trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-system-notification/.node.yaml`

### Stewardship Package
- Package manifest: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/manifest.yaml`
- Relationship request handler: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/.node.yaml`
- Relationship response handler: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-response/.node.yaml`
- Ward invitation handler: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-ward-invitation/.node.yaml`

### WebSocket Implementation
- Event handler: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-transport-ws/src/event_handler.rs`

## Getting Help

- Check the [Troubleshooting](#troubleshooting) section in each guide
- Review example code in the [extending.md](./extending.md) guide
- Examine existing message type implementations in the messaging and stewardship packages
- Check server logs for error details

## Contributing

When adding new message types or improving the system:

1. Follow the patterns established in existing message types
2. Write comprehensive tests
3. Document your message type using the template in [extending.md](./extending.md)
4. Ensure proper error handling and retry logic
5. Test with WebSocket subscriptions for real-time delivery

## Troubleshooting

### Message Not Delivered

1. Check message status: `SELECT * FROM nodes WHERE id = 'message-id'`
2. Check job queue: `SELECT * FROM jobs WHERE status = 'failed'`
3. Check logs: `tail -f logs/raisindb.log | grep message-id`
4. Verify triggers are enabled

### WebSocket Not Receiving Events

1. Check authentication: Ensure valid JWT token
2. Check subscription filters: Verify path/type filters match
3. Check RLS permissions: User must have read access
4. Check connection status: Monitor for disconnects

### Trigger Not Firing

1. Verify trigger enabled: `properties->>'enabled' = true`
2. Check filter conditions: Path, node type, property filters
3. Verify function path exists
4. Check priority (lower = higher priority)

---

**Last Updated:** 2025-01-18
**Version:** 1.0.0
**Package:** raisin-messaging + raisin-stewardship
