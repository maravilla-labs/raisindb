# Message Processing Triggers

The RaisinDB messaging system uses event-driven triggers to route and process messages asynchronously. This document explains how triggers work, their execution model, and how to configure them.

## Overview

Message processing uses RaisinDB's trigger system, which responds to node events (create, update, delete) and executes functions based on filters and conditions.

### Trigger Architecture

```
┌─────────────────┐
│   Node Event    │  (e.g., Message created in outbox)
└────────┬────────┘
         │
         ↓
┌─────────────────┐
│   Event Bus     │  Distributes events to handlers
└────────┬────────┘
         │
         ↓
┌─────────────────┐
│ Trigger Matcher │  Finds triggers matching event
└────────┬────────┘
         │
         ↓
┌─────────────────┐
│  Job Registry   │  Queues function execution
└────────┬────────┘
         │
         ↓
┌─────────────────┐
│ Function Runner │  Executes handler function
└─────────────────┘
```

## Trigger Definition Structure

Triggers are defined as `raisin:Trigger` nodes with this structure:

```yaml
node_type: raisin:Trigger
properties:
  # Identification
  title: Human-readable trigger title
  name: unique-trigger-name
  description: What this trigger does

  # Trigger type
  trigger_type: node_event  # or: schedule, http

  # When to fire
  config:
    event_kinds:
      - Created
      - Updated

  # Filtering conditions
  filters:
    paths:
      - "users/*/outbox/*"
    node_types:
      - raisin:Message
    property_filters:
      message_type: "relationship_request"
      status: "sent"

  # Execution control
  enabled: true
  priority: 10              # Lower = higher priority
  max_retries: 3            # Retry count on failure

  # Handler function
  function_path: /functions/lib/stewardship/handlers/handle-relationship-request
```

## Message Triggers in Messaging and Stewardship Packages

The Messaging package defines routing and generic message triggers, while Stewardship handles relationship workflows.

### 1. on-outbox-create (Router)

**Purpose:** Routes all outgoing messages from user outboxes

**File:** `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/on-outbox-create/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Message Outbox Router
  name: messaging-outbox-router
  description: Routes messages from user outbox to appropriate handlers and copies to sent folder
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    paths:
      - "users/*/outbox/*"
    node_types:
      - raisin:Message

  priority: 0               # Highest priority - runs first
  max_retries: 3
  function_path: /functions/lib/messaging/handlers/route-message
```

**Trigger Conditions:**
- Fires on: Any node created in a user's outbox
- Path pattern: `users/*/outbox/*` (wildcard matches any user ID)
- Node type: `raisin:Message`

**Handler Actions:**
1. Copy message to sender's `sent` folder
2. Update original message status to "sent"
3. Emit update event (triggers type-specific handlers)

---

### 2. process-chat

**Purpose:** Delivers chat messages and updates conversation metadata

**File:** `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-chat/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Process Chat Messages
  name: messaging-chat
  description: Delivers chat messages and updates conversations
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    node_types:
      - raisin:Message
    property_filters:
      message_type: "chat"
      status: "sent"

  priority: 10
  max_retries: 3
  function_path: /functions/lib/messaging/handlers/handle-chat
```

**Trigger Conditions:**
- Fires on: Message created in the sent folder
- Node type: `raisin:Message`
- Property: `message_type = "chat"`
- Property: `status = "sent"`

**Handler Actions:**
1. Create or update sender and recipient conversations
2. Append chat message to both conversations
3. Create recipient notification

---

### 3. process-task-assignment

**Purpose:** Creates InboxTask and Notification nodes for assignees

**File:** `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-task-assignment/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Process Task Assignment Messages
  name: messaging-task-assignment
  description: Handles task assignment messages by creating InboxTask and notifications
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    node_types:
      - raisin:Message
    property_filters:
      message_type: "task_assignment"
      status: "sent"

  priority: 10
  max_retries: 3
  function_path: /functions/lib/messaging/handlers/handle-task-assignment
```

---

### 4. process-system-notification

**Purpose:** Creates Notification nodes for recipients

**File:** `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-system-notification/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Process System Notification Messages
  name: messaging-system-notification
  description: Handles system notification messages by creating Notification nodes
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    node_types:
      - raisin:Message
    property_filters:
      message_type: "system_notification"
      status: "sent"

  priority: 10
  max_retries: 3
  function_path: /functions/lib/messaging/handlers/handle-system-notification
```

---

### 5. process-relationship-request

**Purpose:** Processes relationship requests by delivering to recipient

**File:** `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Process Relationship Request Messages
  name: stewardship-relationship-request
  description: Handles relationship request messages by creating inbox messages for recipients
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    node_types:
      - raisin:Message
    property_filters:
      message_type: "relationship_request"
      status: "sent"

  priority: 10              # Lower priority - runs after router
  max_retries: 3
  function_path: /functions/lib/stewardship/handlers/handle-relationship-request
```

**Trigger Conditions:**
- Fires on: Message created OR updated
- Node type: `raisin:Message`
- Property: `message_type = "relationship_request"`
- Property: `status = "sent"`

**Handler Actions:**
1. Validate request (relationship type exists, recipient exists)
2. Create message copy in recipient's inbox
3. Update message status to "delivered"
4. Send WebSocket notification to recipient

---

### 3. process-relationship-response

**Purpose:** Processes relationship acceptance/rejection

**File:** `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-response/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Process Relationship Response Messages
  name: stewardship-relationship-response
  description: Handles relationship response messages by creating or rejecting relationships
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    node_types:
      - raisin:Message
    property_filters:
      message_type: "relationship_response"
      status: "sent"

  priority: 10
  max_retries: 3
  function_path: /functions/lib/stewardship/handlers/handle-relationship-response
```

**Trigger Conditions:**
- Same as relationship-request but for `message_type = "relationship_response"`

**Handler Actions:**
1. Validate original request exists
2. If response is "accept":
   - Create REL relationships (forward and reverse)
   - Notify both users of established relationship
3. If response is "reject":
   - Notify original sender of rejection
4. Mark original request as "processed"

---

### 4. process-ward-invitation

**Purpose:** Creates new ward accounts and establishes relationships

**File:** `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-ward-invitation/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Process Ward Invitation Messages
  name: stewardship-ward-invitation
  description: Handles ward invitation messages by creating new ward accounts and establishing relationships
  enabled: true
  trigger_type: node_event

  config:
    event_kinds:
      - Created

  filters:
    node_types:
      - raisin:Message
    property_filters:
      message_type: "ward_invitation"
      status: "sent"

  priority: 10
  max_retries: 3
  function_path: /functions/lib/stewardship/handlers/handle-ward-invitation
```

**Trigger Conditions:**
- Same pattern as other message type handlers

**Handler Actions:**
1. Validate ward email not already in use
2. Create new User node with:
   - `can_login: false` (ward accounts are managed)
   - Email and display name from invitation
   - Birth date for age verification
3. Create initial folder structure (inbox/outbox/sent)
4. Establish REL relationships:
   - `(inviter)-[parent-of]->(ward)`
   - `(ward)-[child-of]->(inviter)`
5. Notify inviter of success/failure

---

## Trigger Execution Model

### Priority and Ordering

Triggers are executed in priority order (lower number = higher priority):

```
Priority 0:  Router (on-outbox-create)
    ↓
Priority 10: Type-specific handlers
    ↓
Priority 20: Custom handlers (if any)
```

**Why this matters:**
1. Router **must** run first to set status to "sent"
2. Type handlers filter on `status = "sent"`, so they run after router
3. Custom handlers can use even lower priorities

### Execution Flow

```
1. Node Event Emitted
   ↓
2. All Matching Triggers Identified
   ↓
3. Triggers Sorted by Priority
   ↓
4. For Each Trigger (in priority order):
   a. Check if enabled
   b. Evaluate filters
   c. If match: Queue job in JobRegistry
   ↓
5. Job Queue Processes Jobs
   ↓
6. Handler Function Executes
   ↓
7. On Failure: Retry with backoff (up to max_retries)
```

### Asynchronous Execution

All triggers execute asynchronously:

- Event emission returns immediately
- Triggers queued as background jobs
- No blocking on handler completion
- Eventual consistency model

## Filter Matching

### Path Filters

Path filters use glob-style patterns:

```yaml
paths:
  - "users/*/outbox/*"      # Matches any user's outbox
  - "users/123/inbox/*"     # Specific user's inbox
  - "**/messages/*"         # Any message anywhere
```

**Wildcards:**
- `*` matches any single path segment
- `**` matches any number of segments
- Exact strings match literally

### Node Type Filters

```yaml
node_types:
  - raisin:Message
  - raisin:User
```

Events only trigger if the node's type matches one of these.

### Property Filters

```yaml
property_filters:
  message_type: "relationship_request"
  status: "sent"
  recipient_id: "user-123"
```

**Matching Logic:**
- All property filters must match (AND logic)
- Values are exact string matches
- Nested properties not currently supported

### Workspace Filters

```yaml
filters:
  workspaces:
    - "users"
    - "calendar"
```

Only triggers for events in specified workspaces.

## REL Conditions (Advanced)

Triggers can include REL-based conditions for access control:

```yaml
filters:
  rel_conditions:
    - expression: "(user)-[steward-of]->(ward)"
      user_id: "{sender_id}"
      resource_id: "{recipient_id}"
```

**Use Cases:**
- Only allow messages between related users
- Enforce guardian permissions for ward invitations
- Require specific relationship types for certain messages

**Note:** REL conditions are evaluated at trigger execution time, not at event matching time.

## Error Handling and Retries

### Retry Configuration

```yaml
max_retries: 3              # Retry up to 3 times
```

### Retry Behavior

```
Attempt 1: Immediate
   ↓ (failure)
Attempt 2: After 10s
   ↓ (failure)
Attempt 3: After 1min
   ↓ (failure)
Attempt 4: After 10min
   ↓ (failure)
Final: Mark as failed
```

### Error States

Failed messages end up with:

```json
{
  "status": "error",
  "metadata": {
    "error": {
      "message": "Recipient user not found",
      "code": "RECIPIENT_NOT_FOUND",
      "retry_count": 3,
      "last_attempt": "2025-12-19T10:30:00Z"
    }
  }
}
```

### Manual Retry

To retry a failed message:

```javascript
// Reset status to pending
await updateNode(messageId, {
  status: "pending",
  metadata: {
    ...existingMetadata,
    error: null  // Clear error
  }
});
```

## Job Queue Integration

Message triggers use RaisinDB's unified job queue system:

```javascript
// Inside trigger handler
await JobRegistry.register_job({
  job_type: 'process_message',
  payload: {
    message_id: message.id,
    message_type: message.properties.message_type
  },
  retry_count: 0,
  max_retries: 3
});

await JobDataStore.put(job);
```

**Benefits:**
- Persistence: Jobs survive server restarts
- Monitoring: Job status can be queried
- Throttling: Rate limiting can be applied
- Distributed: Jobs can run on any cluster node

## Monitoring Trigger Execution

### Via Logs

Trigger execution is logged:

```
[INFO] Trigger matched: stewardship-outbox-router for node user-123/outbox/msg-456
[INFO] Queued job: process_message (msg-456)
[INFO] Executing handler: route-message
[INFO] Handler completed successfully
```

### Via Job Queue

Query job status:

```sql
SELECT * FROM jobs
WHERE job_type = 'process_message'
  AND status = 'failed'
ORDER BY created_at DESC;
```

### Via WebSocket Events

Subscribe to trigger execution events:

```javascript
ws.subscribe({
  workspace: 'system',
  event_types: ['trigger:executed', 'trigger:failed']
});
```

## Testing Triggers

### Unit Testing

Test trigger matching logic:

```javascript
const trigger = {
  filters: {
    paths: ['users/*/outbox/*'],
    node_types: ['raisin:Message'],
    property_filters: {
      message_type: 'relationship_request'
    }
  }
};

const event = {
  path: 'users/123/outbox/msg-456',
  node_type: 'raisin:Message',
  properties: {
    message_type: 'relationship_request'
  }
};

assert(matchesTrigger(event, trigger));
```

### Integration Testing

Test full message flow:

```javascript
// 1. Create message in outbox
const message = await createNode({
  parent_path: 'users/alice/outbox',
  node_type: 'raisin:Message',
  properties: {
    message_type: 'relationship_request',
    recipient_id: 'user-bob',
    sender_id: 'user-alice',
    body: { relationship_type: 'parent-of' }
  }
});

// 2. Wait for async processing
await waitForJobCompletion();

// 3. Verify message in recipient inbox
const inbox = await listChildren('users/bob/inbox');
assert(inbox.some(m => m.properties.sender_id === 'user-alice'));

// 4. Verify copy in sender's sent folder
const sent = await listChildren('users/alice/sent');
assert(sent.some(m => m.id === message.id));
```

## Best Practices

### 1. Use Appropriate Priorities

- **0-9**: System-critical triggers (routers, validators)
- **10-19**: Business logic triggers (message handlers)
- **20-29**: Auditing and logging triggers
- **30+**: Low-priority background tasks

### 2. Keep Handlers Idempotent

Handlers may be retried, so ensure they're safe to run multiple times:

```javascript
async function handleRelationshipRequest(message) {
  // Check if already processed
  if (message.status === 'processed') {
    return; // Already done, skip
  }

  // Process message...
}
```

### 3. Use Specific Filters

Avoid overly broad filters:

```yaml
# Bad: Triggers on every message
filters:
  node_types:
    - raisin:Message

# Good: Specific type and status
filters:
  node_types:
    - raisin:Message
  property_filters:
    message_type: "relationship_request"
    status: "sent"
```

### 4. Handle Failures Gracefully

```javascript
try {
  await processMessage(message);
} catch (error) {
  // Log error details
  console.error('Message processing failed:', error);

  // Update message with error info
  await updateNode(message.id, {
    status: 'error',
    metadata: {
      error: {
        message: error.message,
        code: error.code,
        timestamp: new Date().toISOString()
      }
    }
  });

  // Don't throw - let retry mechanism handle it
  throw error;
}
```

### 5. Use Related Entity IDs

Link messages to related entities for traceability:

```javascript
await createNode({
  properties: {
    message_type: 'relationship_response',
    related_entity_id: originalRequestId,  // Link to original request
    // ...
  }
});
```

## Advanced Patterns

### Message Chains

Create message workflows:

```
Request → Response → Confirmation
```

Each message type handler can create the next message in the chain.

### Conditional Routing

Use property filters to route different message types differently:

```yaml
# Handler 1: High priority requests
property_filters:
  message_type: "relationship_request"
  body.priority: "high"
priority: 5

# Handler 2: Normal requests
property_filters:
  message_type: "relationship_request"
priority: 10
```

### Broadcast Messages

Send to multiple recipients by creating multiple messages:

```javascript
for (const recipientId of recipientIds) {
  await createNode({
    parent_path: `users/${senderId}/outbox`,
    node_type: 'raisin:Message',
    properties: {
      recipient_id: recipientId,
      // ...
    }
  });
}
```

Each message triggers independently, ensuring individual delivery tracking.

## File References

- Trigger node type: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_trigger.yaml`
- Router trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/on-outbox-create/.node.yaml`
- Chat trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-chat/.node.yaml`
- Task assignment trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-task-assignment/.node.yaml`
- System notification trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/process-system-notification/.node.yaml`
- Relationship request trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/.node.yaml`
- Relationship response trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-response/.node.yaml`
- Ward invitation trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-ward-invitation/.node.yaml`
