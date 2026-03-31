# Messaging System Architecture

The RaisinDB messaging system provides a flexible inbox/outbox pattern for inter-user communication, workflow notifications, and relationship management. This document describes the overall architecture, message lifecycle, and processing model.

## Overview

The messaging system is built on three core concepts:

1. **MessageFolder nodes** - Container nodes for messages (inbox, outbox, sent)
2. **Message nodes** - Individual message instances with type-specific payloads
3. **Trigger-based routing** - Event-driven message processing and delivery

## Folder Structure

Each user (raisin:User node) automatically receives three MessageFolder nodes upon creation through the `initial_structure` mechanism:

```yaml
# From crates/raisin-core/global_nodetypes/raisin_user.yaml
initial_structure:
  children:
    - name: inbox
      node_type: raisin:MessageFolder
    - name: outbox
      node_type: raisin:MessageFolder
    - name: sent
      node_type: raisin:MessageFolder
```

### Folder Purposes

```
users/{user_id}/
├── inbox/          # Incoming messages from other users
├── outbox/         # Outgoing messages awaiting processing
└── sent/           # Delivered messages (copies of sent items)
```

- **inbox**: Receives messages delivered from other users' outboxes
- **outbox**: Staging area for new messages. When a message is created here, triggers process and route it
- **sent**: Archive of messages the user has sent (copies created by the router)

### Conversation Threads (Chat)

Chat messages are grouped under conversation nodes to support threading and previews:

```
users/{user_id}/inbox/
└── chats/
    └── conv-<id>/   # raisin:Conversation
        ├── msg-...  # raisin:Message (chat)
        └── msg-...
```

Conversation nodes store participants, last_message, unread_count, and updated_at to support inbox UIs.

## Message Node Type

Messages are represented as `raisin:Message` nodes with the following schema:

```yaml
# From crates/raisin-core/global_nodetypes/raisin_message.yaml
properties:
  - name: message_type
    type: String
    required: true
    description: Type of message (e.g., "relationship_request", "chat")

  - name: subject
    type: String
    required: false
    description: Subject line for display

  - name: body
    type: Object
    required: true
    description: Payload specific to message_type

  - name: recipient_id
    type: String
    required: true
    description: Target user ID

  - name: sender_id
    type: String
    required: true
    description: Source user ID

  - name: status
    type: String
    required: true
    default: "pending"
    description: Status - "pending", "sent", "delivered", "read", "processed"

  - name: related_entity_id
    type: String
    required: false
    description: Related entity ID (e.g., relationship request ID)

  - name: expires_at
    type: Date
    required: false
    description: Expiration date for the message

  - name: metadata
    type: Object
    required: false
    description: Additional extensible metadata
```

## Message Lifecycle

Messages follow a predictable state machine:

```
┌─────────────────────────────────────────────────────────────────┐
│                        Message Lifecycle                         │
└─────────────────────────────────────────────────────────────────┘

1. CREATION
   User creates message in their outbox
   Status: "pending"
   Path: users/{sender_id}/outbox/{message_id}

2. ROUTING (Trigger: on-outbox-create)
   ↓
   Router detects new message in outbox
   Copies message to sender's sent folder
   Updates status to "sent"

3. DELIVERY (Trigger: message-type-specific)
   ↓
   Type-specific handler processes message
   Creates copy in recipient's inbox
   Updates status to "delivered"
   Path: users/{recipient_id}/inbox/{message_id}

4. PROCESSING
   ↓
   Recipient reads message
   Status: "read"
   ↓
   Handler performs actions (e.g., create relationship)
   Status: "processed"
```

### State Transitions

```
pending → sent → delivered → read → processed
   ↓         ↓         ↓
 error     error    error
```

## Message Flow Diagram

```
┌──────────────┐
│ Sender User  │
└──────┬───────┘
       │
       │ 1. Create message
       ↓
┌──────────────────────────────────────────┐
│  Outbox                                   │
│  users/{sender_id}/outbox/{msg_id}       │
│  status: "pending"                        │
└──────────────┬───────────────────────────┘
               │
               │ 2. NodeEvent: Created
               ↓
┌──────────────────────────────────────────┐
│  Trigger: on-outbox-create               │
│  Path filter: users/*/outbox/*           │
│  Function: route-message                 │
└──────────────┬───────────────────────────┘
               │
               ├─→ Copy to Sent Folder
               │   users/{sender_id}/sent/{msg_id}
               │   status: "sent"
               │
               └─→ Update outbox message
                   status: "sent"
                   │
                   │ 3. NodeEvent: Updated (status="sent")
                   ↓
┌──────────────────────────────────────────┐
│  Trigger: process-{message_type}         │
│  Property filter: message_type, status   │
│  Function: handle-{message_type}         │
└──────────────┬───────────────────────────┘
               │
               │ 4. Create in recipient inbox
               ↓
┌──────────────────────────────────────────┐
│  Inbox                                    │
│  users/{recipient_id}/inbox/{msg_id}     │
│  status: "delivered"                      │
└──────────────┬───────────────────────────┘
               │
               │ 5. WebSocket notification
               ↓
┌──────────────────────────────────────────┐
│  Recipient User                          │
│  Receives real-time notification         │
└──────────────────────────────────────────┘
```

## Processing Model

### Asynchronous by Default

Message processing is fully asynchronous and event-driven:

1. **No blocking operations**: Creating a message in the outbox returns immediately
2. **Trigger-based routing**: The event bus notifies triggers of new messages
3. **Job queue execution**: Handlers run as background jobs via JobRegistry
4. **Eventual consistency**: Messages are eventually delivered and processed

### Trigger Architecture

The messaging system uses RaisinDB's trigger system to process messages:

```yaml
# Trigger definition structure
node_type: raisin:Trigger
properties:
  trigger_type: node_event
  config:
    event_kinds: [Created, Updated]
  filters:
    paths: ["users/*/outbox/*"]
    node_types: [raisin:Message]
    property_filters:
      message_type: "relationship_request"
      status: "sent"
  priority: 10
  max_retries: 3
  function_path: /functions/lib/stewardship/handlers/handle-relationship-request
```

### Processing Stages

**Stage 1: Router (Priority 0)**
- Triggers on: Any message created in outbox
- Path filter: `users/*/outbox/*`
- Actions:
  - Copy message to sender's sent folder
  - Update outbox message status to "sent"

**Stage 2: Type Handler (Priority 10)**
- Triggers on: Message status change to "sent"
- Property filter: `message_type` + `status: "sent"`
- Actions:
  - Process message according to type
  - Create message in recipient's inbox
  - Perform type-specific actions (e.g., create relationships)

### Error Handling

Messages support automatic retry on failure:

```yaml
max_retries: 3  # Retry up to 3 times
```

If all retries fail:
1. Message status remains in error state
2. Error details stored in message metadata
3. Manual intervention may be required

### Retry Strategy

- Exponential backoff between retries
- Retries are job-queue based (no immediate retries)
- Failed messages can be manually reprocessed

## Data Paths

### Physical Storage

Messages are stored in the node tree at these locations:

```
/users/{user_id}/inbox/{message_id}
/users/{user_id}/outbox/{message_id}
/users/{user_id}/sent/{message_id}
```

### Querying Messages

Using RaisinSQL, you can query messages:

```sql
-- Get user's inbox messages
SELECT * FROM nodes
WHERE path LIKE 'users/123/inbox/%'
  AND node_type = 'raisin:Message'
  AND status = 'delivered'
ORDER BY created_at DESC;

-- Get messages by type
SELECT * FROM nodes
WHERE node_type = 'raisin:Message'
  AND properties->>'message_type' = 'relationship_request'
  AND properties->>'recipient_id' = '123';
```

### Indexing

The Message node type is fully indexed:

```yaml
indexable: true
index_types: [Property, Fulltext]
```

Key indexed properties:
- `message_type` - Property index for filtering by type
- `recipient_id` - Property index for recipient queries
- `sender_id` - Property index for sender queries
- `status` - Property index for status filtering
- `subject` - Fulltext index for search

## Access Control

Messages respect RaisinDB's Row-Level Security (RLS):

- Users can read their own inbox, outbox, and sent messages
- Users cannot read other users' messages (unless granted via REL)
- System context can access all messages for processing

### Typical RLS Rules

```yaml
# Example permission rule for messages
- operation: Read
  conditions:
    - type: property_match
      property: recipient_id
      value: "{user_id}"
    - type: property_match
      property: sender_id
      value: "{user_id}"
  operator: OR
```

## Real-time Notifications

Messages integrate with RaisinDB's WebSocket event system:

1. Message created in inbox → NodeEvent emitted
2. WsEventHandler receives event
3. RLS check: Can user read this message?
4. If allowed, event forwarded to subscribed WebSocket connections
5. Client receives real-time notification

See [notifications.md](./notifications.md) for details.

## Scalability Considerations

### Performance Characteristics

- **Message creation**: O(1) - Simple node creation
- **Routing**: O(1) - Single copy operation
- **Delivery**: O(1) - Single inbox creation
- **Inbox queries**: O(log n) - Indexed by recipient_id and status

### High-Volume Scenarios

For high message volumes:

1. **Batching**: Messages can be batched for delivery
2. **Partitioning**: Users can be partitioned across repository branches
3. **Archiving**: Old messages can be moved to archive folders
4. **Cleanup**: Expired messages can be automatically deleted

### Message Retention

Configure retention policies using:

```yaml
expires_at: "2025-01-01T00:00:00Z"
```

Expired messages can be cleaned up by scheduled triggers.

## Extension Points

The messaging system is designed for extensibility:

1. **Custom message types**: Define new types with custom schemas
2. **Custom handlers**: Create type-specific processing logic
3. **Custom routing**: Implement complex routing rules
4. **Custom validation**: Add message validation triggers

See [extending.md](./extending.md) for implementation guides.

## Integration with Stewardship

The messaging system is heavily used by the Stewardship package:

- **relationship_request**: Request relationships between users
- **relationship_response**: Accept/reject relationship requests
- **ward_invitation**: Invite new wards to the system
- **stewardship_request**: Request stewardship permissions

These message types enable the delegation and relationship workflows that power RaisinDB's access control system.

## File References

- Message node type: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_message.yaml`
- MessageFolder node type: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_message_folder.yaml`
- User node type: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_user.yaml`
- Router trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/on-outbox-create/.node.yaml`
- WebSocket handler: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-transport-ws/src/event_handler.rs`
