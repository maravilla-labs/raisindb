# Stewardship Triggers

This document describes the trigger system used in the RaisinDB Stewardship System to handle message-based workflows.

## Overview

The stewardship system uses RaisinDB's trigger system to automatically process messages and establish relationships. Triggers listen for specific events (like message creation) and invoke handler functions to process them.

### Key Concepts

- **Triggers**: Event listeners that fire when conditions are met
- **Handlers**: QuickJS functions that execute when triggers fire
- **Message Types**: Different message types route to different handlers
- **Router Trigger**: Central dispatcher that routes messages by type

---

## How Triggers Work in RaisinDB

Triggers in RaisinDB are defined as `raisin:Trigger` nodes with the following structure:

```yaml
node_type: raisin:Trigger
properties:
  title: "Human-readable trigger name"
  name: "unique-trigger-id"
  description: "What this trigger does"
  enabled: true
  trigger_type: "node_event"  # or "schedule", "http"
  config:
    event_kinds: ["Created", "Updated", "Deleted"]
  filters:
    paths: ["users/*/outbox/*"]
    node_types: ["raisin:Message"]
    property_filters:
      message_type: "relationship_request"
      status: "sent"
  priority: 10
  max_retries: 3
  function_path: "/functions/lib/stewardship/handlers/handle-relationship-request"
```

### Trigger Types

| Type | Description | Use Case |
|------|-------------|----------|
| `node_event` | Fires on node creation, update, or deletion | Message processing, workflow automation |
| `schedule` | Fires on cron schedule | Cleanup jobs, periodic checks |
| `http` | Fires on HTTP webhook | External integrations |

### Event Kinds (for node_event triggers)

- `Created` - When a node is created
- `Updated` - When a node is updated
- `Deleted` - When a node is deleted
- `PropertyChanged` - When specific properties change
- `RelationCreated` - When a graph relation is created
- `RelationDeleted` - When a graph relation is deleted

### Filters

Filters narrow when a trigger fires:

```yaml
filters:
  # Only fire for specific workspaces
  workspaces: ["raisin:access_control"]

  # Only fire for nodes matching path patterns
  paths:
    - "users/*/outbox/*"
    - "content/blog/**"

  # Only fire for specific node types
  node_types:
    - "raisin:Message"

  # Only fire when properties match
  property_filters:
    message_type: "relationship_request"
    status: "sent"
```

### REL Conditions (Advanced)

For complex filtering, triggers can use REL (Raisin Expression Language) conditions:

```yaml
filters:
  rel_condition: "input.node.path.descendantOf('/users') && input.properties.priority > 5"
```

See [REL Documentation](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/REL.md) for full syntax.

---

## Stewardship Trigger Architecture

The stewardship system uses a **router pattern** where:

1. A main router trigger listens for all outbox messages
2. The router copies messages to the sender's "sent" folder
3. Specific handlers listen for messages with particular `message_type` values
4. Each handler processes its message type and creates inbox messages for recipients

```
User creates message in outbox
         ↓
    Router Trigger (on-outbox-create)
         ↓
    Copies to /sent folder
         ↓
    Message status → "sent"
         ↓
    Specific Handler Trigger fires
         ↓
    Handler processes message
         ↓
    Creates inbox message for recipient
```

---

## Router Trigger: on-outbox-create

The main entry point for all outbox messages.

### Definition

```yaml
node_type: raisin:Trigger
properties:
  title: Message Outbox Router
  name: stewardship-outbox-router
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
  priority: 0
  max_retries: 3
  function_path: /functions/lib/messaging/handlers/route-message
```

### Behavior

1. Fires when any `raisin:Message` is created in a user's `/outbox/` folder
2. Copies the message to the user's `/sent/` folder
3. Updates message status from `"pending"` to `"sent"`
4. The status change triggers downstream handlers

### Location

`/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/on-outbox-create/.node.yaml`

---

## Message Type Handlers

After the router sets status to "sent", specific handlers process each message type.

### Handler: process-relationship-request

Handles relationship request messages.

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
  priority: 10
  max_retries: 3
  function_path: /functions/lib/stewardship/handlers/handle-relationship-request
```

**What it does**:
1. Receives relationship request message
2. Validates request (checks limits, permissions)
3. Creates inbox message for recipient
4. Updates message status to `"delivered"`

**Location**: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/.node.yaml`

---

### Handler: process-relationship-response

Handles relationship response messages (accept/reject).

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

**What it does**:
1. Receives response (accept/reject) to relationship request
2. If accepted:
   - Creates graph relation (e.g., `(steward)-[:GUARDIAN_OF]->(ward)`)
   - Creates inverse relation if bidirectional
   - Updates original request message status
3. If rejected:
   - Updates request message status to "rejected"
4. Creates inbox message for original requestor

**Location**: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-response/.node.yaml`

---

### Handler: process-ward-invitation

Handles ward invitation messages for creating new ward accounts.

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

**What it does**:
1. Receives ward invitation message
2. Checks if steward has permission to create wards (via `StewardshipConfig.steward_creates_ward_enabled`)
3. Creates new `raisin:User` node for the ward
4. Creates graph relation from steward to ward
5. Sends welcome message to new ward account

**Location**: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-ward-invitation/.node.yaml`

---

## Message Flow Examples

### Example 1: Relationship Request Flow

```
1. User A creates message in /users/alice/outbox/msg-001
   - message_type: "relationship_request"
   - status: "pending"
   - recipient_id: "user:bob"
   - body: { relation_type: "GUARDIAN_OF", ... }

2. Router Trigger (on-outbox-create) fires
   - Copies to /users/alice/sent/msg-001
   - Updates status: "sent"

3. Handler Trigger (process-relationship-request) fires
   - Creates /users/bob/inbox/msg-001
   - Bob now has request in inbox

4. User B responds by creating message in /users/bob/outbox/msg-002
   - message_type: "relationship_response"
   - status: "pending"
   - body: { accepted: true, original_request_id: "msg-001" }

5. Router Trigger fires for Bob's response
   - Copies to /users/bob/sent/msg-002
   - Updates status: "sent"

6. Handler Trigger (process-relationship-response) fires
   - Creates graph relation: (alice)-[:GUARDIAN_OF]->(bob)
   - Creates inverse: (bob)-[:WARD_OF]->(alice)
   - Updates original request message status: "processed"
   - Creates inbox message for Alice: "Bob accepted your request"
```

### Example 2: Ward Invitation Flow

```
1. Steward creates ward invitation in /users/steward/outbox/msg-003
   - message_type: "ward_invitation"
   - status: "pending"
   - body: {
       ward_email: "child@example.com",
       ward_display_name: "Alex Smith",
       relation_type: "PARENT_OF"
     }

2. Router Trigger fires
   - Copies to /users/steward/sent/msg-003
   - Updates status: "sent"

3. Handler Trigger (process-ward-invitation) fires
   - Creates new user node: /users/internal/alex-smith
   - Creates graph relation: (steward)-[:PARENT_OF]->(alex)
   - Sends email to child@example.com with signup link
   - Creates inbox message for new ward with welcome info
```

---

## Adding Custom Message Types

To add a new message type:

### 1. Define the Message Type

Document the message type structure:

```javascript
// Message Type: "permission_request"
{
  message_type: "permission_request",
  subject: "Request for access",
  body: {
    permission_type: "read",
    resource_path: "/documents/sensitive",
    justification: "Need access for project review"
  },
  recipient_id: "user:admin",
  sender_id: "user:alice",
  status: "pending"
}
```

### 2. Create Handler Function

Create a QuickJS function at `/functions/lib/stewardship/handlers/handle-permission-request/index.js`:

```javascript
async function handlePermissionRequest(input) {
    const { node, event } = input;
    const { recipient_id, sender_id, body } = node.properties;

    // Validate request
    // Check permissions
    // Process request
    // Create inbox message for recipient

    return { success: true };
}

module.exports = { handlePermissionRequest };
```

### 3. Create Trigger Definition

Create trigger node at `/functions/lib/stewardship/handlers/handle-permission-request/.node.yaml`:

```yaml
node_type: raisin:Trigger
properties:
  title: Process Permission Request Messages
  name: stewardship-permission-request
  description: Handles permission request messages
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Created
  filters:
    node_types:
      - raisin:Message
    property_filters:
      message_type: "permission_request"
      status: "sent"
  priority: 10
  max_retries: 3
  function_path: /functions/lib/stewardship/handlers/handle-permission-request
```

### 4. Register in Package Manifest

Add to `/builtin-packages/raisin-stewardship/manifest.yaml`:

```yaml
functions:
  - path: /functions/lib/stewardship/handlers/handle-permission-request
    handler: handlePermissionRequest
```

---

## Trigger Priority and Execution Order

Triggers are executed in priority order (lower number = higher priority):

| Priority | Trigger | Purpose |
|----------|---------|---------|
| 0 | Router (on-outbox-create) | Must run first to copy and update status |
| 10 | Message handlers | Process specific message types after routing |
| 100+ | Custom handlers | User-defined handlers run last |

---

## Error Handling and Retries

Triggers support automatic retries via `max_retries` property:

```yaml
max_retries: 3  # Retry up to 3 times on failure
```

**Retry behavior**:
- Exponential backoff: 1s, 2s, 4s, 8s, etc.
- Errors are logged to trigger execution history
- After max retries, trigger is marked as failed
- Failed triggers can be manually retried via admin console

---

## Debugging Triggers

### View Trigger Execution History

```sql
SELECT * FROM trigger_executions
WHERE trigger_id = 'stewardship-relationship-request'
ORDER BY executed_at DESC
LIMIT 10;
```

### Check Trigger Status

```sql
SELECT * FROM nodes
WHERE node_type = 'raisin:Trigger'
  AND properties->>'enabled' = 'true';
```

### Test Trigger Manually

Use the function invocation endpoint:

```bash
POST /api/v1/repositories/{repo}/functions/invoke
Content-Type: application/json

{
  "function_path": "/functions/lib/stewardship/handlers/handle-relationship-request",
  "input": {
    "node": {
      "id": "msg-123",
      "properties": {
        "message_type": "relationship_request",
        "recipient_id": "user:bob",
        "sender_id": "user:alice",
        "body": { "relation_type": "GUARDIAN_OF" }
      }
    },
    "event": {
      "kind": "Created"
    }
  }
}
```

---

## Best Practices

1. **Use Router Pattern**: Centralize message routing logic
2. **Set Priorities Carefully**: Ensure execution order is correct
3. **Handle Errors Gracefully**: Always return structured error responses
4. **Validate Input**: Check message structure before processing
5. **Idempotency**: Handlers should be safe to retry
6. **Log Extensively**: Use `console.log()` for debugging
7. **Test Thoroughly**: Test each message type in isolation

---

## See Also

- [Node Types](./node-types.md) - Stewardship node type definitions
- [Functions](./functions.md) - QuickJS functions for stewardship queries
- [API Reference](./api-reference.md) - REST API endpoints
- [REL Documentation](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/REL.md) - Expression language syntax
