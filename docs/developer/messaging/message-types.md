# Message Types

RaisinDB's messaging system supports multiple message types, each with a specific purpose and body schema. This document describes all built-in message types provided by the Messaging and Stewardship packages.

## Message Type Overview

| Message Type | Purpose | Handler Trigger | Creates |
|--------------|---------|-----------------|---------|
| `relationship_request` | Request a relationship with another user | `process-relationship-request` | Inbox message for recipient |
| `relationship_response` | Accept/reject a relationship request | `process-relationship-response` | REL relationship or rejection notice |
| `ward_invitation` | Invite someone to become a ward | `process-ward-invitation` | New user account + relationship |
| `stewardship_request` | Request stewardship permissions | `process-stewardship-request` | Inbox request + notification |
| `system_notification` | System-generated notifications | `process-system-notification` | Notification only |
| `chat` | Simple chat message | `process-chat` | Conversation message + notification |

## Common Message Structure

All messages share these common properties:

```javascript
{
  // Message envelope
  "message_type": "relationship_request",
  "subject": "Human-readable subject",
  "recipient_id": "user-node-id-123",
  "sender_id": "user-node-id-456",
  "status": "pending",

  // Type-specific payload
  "body": {
    // Type-specific fields
  },

  // Optional fields
  "related_entity_id": "optional-reference-id",
  "expires_at": "2025-12-31T23:59:59Z",
  "metadata": {
    // Additional extensible data
  }
}
```

## Built-in Message Types

### 1. relationship_request

Requests to establish a relationship between two users using RaisinDB's REL (Relationship Expression Language) system.

#### When Used

- User A wants to connect with User B
- System suggests a relationship (e.g., parent-child based on household)
- Automated relationship discovery workflows

#### Body Schema

```typescript
{
  "relationship_type": string,      // Required: REL relation type (e.g., "parent-of", "ward-of")
  "message": string,                // Optional: Personal message from sender
  "auto_accept": boolean,           // Optional: Auto-accept if permitted (default: false)
  "expires_in_days": number,        // Optional: Request expiration (default: 30)
  "metadata": object                // Optional: Additional context
}
```

#### Example

```json
{
  "message_type": "relationship_request",
  "subject": "Relationship Request: Parent-Child",
  "recipient_id": "user-child-123",
  "sender_id": "user-parent-456",
  "status": "pending",
  "body": {
    "relationship_type": "parent-of",
    "message": "Hi! I'd like to connect as your parent to help manage your account.",
    "expires_in_days": 30
  }
}
```

#### Processing Flow

1. Message created in sender's outbox
2. Router copies to sender's sent folder
3. `process-relationship-request` trigger fires
4. Handler creates message in recipient's inbox
5. Recipient receives WebSocket notification
6. Recipient accepts/rejects via UI
7. Response message sent back to original sender

#### Expected Outcomes

**If accepted:**
- REL relationship created: `(parent-456)-[parent-of]->(child-123)`
- Reverse relationship may be created: `(child-123)-[child-of]->(parent-456)`
- Both users receive confirmation messages

**If rejected:**
- Rejection message sent to original sender
- No relationship created
- Request marked as "processed"

#### Related Files

- Trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/.node.yaml`

---

### 2. relationship_response

Accepts or rejects a relationship request.

#### When Used

- User responds to a received relationship_request
- Automated acceptance based on rules
- Bulk approval/rejection workflows

#### Body Schema

```typescript
{
  "original_request_id": string,    // Required: ID of the original request message
  "response": "accept" | "reject",  // Required: Accept or reject
  "relationship_type": string,      // Required: Must match original request
  "message": string,                // Optional: Response message
  "metadata": object                // Optional: Additional context
}
```

#### Example

```json
{
  "message_type": "relationship_response",
  "subject": "Re: Relationship Request Accepted",
  "recipient_id": "user-parent-456",
  "sender_id": "user-child-123",
  "status": "pending",
  "body": {
    "original_request_id": "msg-request-789",
    "response": "accept",
    "relationship_type": "parent-of",
    "message": "Thanks for connecting! Happy to have you as my parent."
  },
  "related_entity_id": "msg-request-789"
}
```

#### Processing Flow

1. Response message created in outbox
2. Router copies to sent folder
3. `process-relationship-response` trigger fires
4. Handler validates original request exists
5. If response is "accept":
   - Creates REL relationship
   - Creates confirmation message in both inboxes
6. If response is "reject":
   - Creates rejection notification in sender's inbox
7. Marks original request as "processed"

#### Expected Outcomes

**If accept:**
- REL relationship created in both directions
- Original requester notified of acceptance
- Both users can now interact based on relationship permissions

**If reject:**
- Original requester notified of rejection
- No relationship created
- Request closed

#### Validation

Handler performs these checks:
- Original request message exists
- Response sender is the original recipient
- Relationship type matches original request
- Request has not expired
- Request has not already been processed

#### Related Files

- Trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-response/.node.yaml`

---

### 3. ward_invitation

Invites someone to create a ward account (managed user account, typically for minors).

#### When Used

- Parent inviting a child to create an account
- Guardian setting up account for ward
- Organization creating managed user accounts

#### Body Schema

```typescript
{
  "ward_email": string,             // Required: Email for new ward account
  "ward_display_name": string,      // Required: Display name for ward
  "relationship_type": string,      // Required: Relationship (e.g., "parent-of")
  "birth_date": string,             // Optional: ISO date for age verification
  "can_login": boolean,             // Optional: Can ward log in? (default: false)
  "invitation_message": string,     // Optional: Personal message
  "metadata": object                // Optional: Additional ward attributes
}
```

#### Example

```json
{
  "message_type": "ward_invitation",
  "subject": "Ward Invitation: Account Setup",
  "recipient_id": "system",
  "sender_id": "user-parent-456",
  "status": "pending",
  "body": {
    "ward_email": "child@example.com",
    "ward_display_name": "Alex Smith",
    "relationship_type": "parent-of",
    "birth_date": "2015-06-15",
    "can_login": false,
    "invitation_message": "Setting up your account so I can help manage your activities."
  }
}
```

#### Processing Flow

1. Invitation created in sender's outbox
2. Router copies to sent folder
3. `process-ward-invitation` trigger fires
4. Handler performs validation:
   - Email not already in use
   - Sender has permission to create wards
   - Valid relationship type
5. Creates new User node with `can_login: false`
6. Creates initial folder structure (inbox/outbox/sent)
7. Establishes REL relationship between inviter and ward
8. Sends confirmation to inviter's inbox
9. Optionally sends email invitation to ward

#### Expected Outcomes

**Success:**
- New user account created
- REL relationship established (e.g., parent-of/child-of)
- Inviter can now act on behalf of ward
- Ward receives email with setup instructions (if applicable)

**Failure:**
- Error message in inviter's inbox
- No account created
- Common failures: email already exists, invalid permissions

#### Security Considerations

- Ward accounts have `can_login: false` by default
- Only guardians/parents can act on behalf of wards
- Age verification via `birth_date` property
- REL conditions can enforce guardian permissions

#### Related Files

- Trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-ward-invitation/.node.yaml`

---

### 4. stewardship_request

Requests stewardship permissions for temporary or permanent delegation.

#### When Used

- Executive requesting an assistant
- Temporary delegation during vacation
- Role-based permission requests

#### Body Schema

```typescript
{
  "stewardship_type": string,       // Required: Type of stewardship (e.g., "assistant", "temporary")
  "relationship_type": string,      // Required: REL relationship type
  "scope": object,                  // Optional: Scope limitations
  "duration": string,               // Optional: Time limit (ISO 8601 duration)
  "permissions": string[],          // Optional: Specific permissions requested
  "justification": string,          // Optional: Reason for request
  "metadata": object                // Optional: Additional context
}
```

#### Example

```json
{
  "message_type": "stewardship_request",
  "subject": "Stewardship Request: Assistant Access",
  "recipient_id": "user-executive-123",
  "sender_id": "user-assistant-456",
  "status": "pending",
  "body": {
    "stewardship_type": "assistant",
    "relationship_type": "assistant-of",
    "scope": {
      "workspaces": ["calendar", "email"],
      "operations": ["Read", "Create", "Update"]
    },
    "duration": "P90D",
    "justification": "Need access to manage calendar and respond to emails."
  }
}
```

#### Processing Flow

1. Request created in sender's outbox
2. Router copies to sent folder
3. Request delivered to recipient's inbox with a notification
4. Recipient reviews and responds via application workflow
5. Follow-up actions (relationship creation or override) are handled by stewardship policies

#### Expected Outcomes

**If approved:**
- REL relationship created
- StewardshipOverride node created with:
  - Time limit
  - Scope restrictions
  - Permission set
- Sender can now act within specified scope

**If denied:**
- Sender receives rejection notice
- No permissions granted

---

### 5. system_notification

System-generated notifications for users.

#### When Used

- Account changes (password reset, email change)
- Security alerts
- System maintenance notices
- Policy updates

#### Body Schema

```typescript
{
  "notification_type": string,      // Optional: Type of notification
  "title": string,                  // Optional: Notification title
  "body": string,                   // Optional: Notification content
  "link": string,                   // Optional: Call-to-action link
  "priority": number,               // Optional: Priority (1-5)
  "data": object,                   // Optional: Additional data
  "expires_in_seconds": number      // Optional: Auto-dismiss delay
}
```

#### Example

```json
{
  "message_type": "system_notification",
  "subject": "Security Alert: New Login from Unknown Device",
  "recipient_id": "user-123",
  "sender_id": "system",
  "status": "pending",
  "body": {
    "notification_type": "security_alert",
    "title": "New Login Detected",
    "body": "We detected a new login from San Francisco, CA on 2025-12-19.",
    "link": "/settings/security",
    "priority": 4
  }
}
```

#### Processing Flow

1. Message created in sender's outbox
2. Router copies to sent folder
3. Notification node created in recipient's inbox/notifications
4. WebSocket notification sent to user
5. User sees notification in UI and can mark as read

#### Expected Outcomes

- User receives immediate notification
- Message appears in inbox
- User can acknowledge or take action

---

### 6. chat

Simple person-to-person chat messages.

#### When Used

- Direct messaging between users
- Quick questions or responses
- Informal communication

#### Body Schema

```typescript
{
  "message_text": string,           // Required: Chat message content
  "thread_id": string,              // Optional: For threading/replies
  "attachments": object[],          // Optional: File attachments
  "metadata": object                // Optional: Additional context
}
```

#### Example

```json
{
  "message_type": "chat",
  "subject": null,
  "recipient_id": "user-bob-123",
  "sender_id": "user-alice-456",
  "status": "pending",
  "body": {
    "message_text": "Hey Bob, can you review the latest design mockups?",
    "thread_id": "thread-789"
  }
}
```

#### Processing Flow

1. Message created in sender's outbox
2. Router copies to sent folder
3. Conversation nodes created/updated for sender and recipient
4. Message appended to recipient conversation
5. Notification sent to recipient (WebSocket + Notification node)

#### Expected Outcomes

- Real-time delivery to recipient
- Message appears in chat interface with conversation context
- Read receipts may be supported
- Threading preserves conversation context

---

## Message Type Registry

Message types are registered implicitly through trigger definitions. Each trigger filters on:

```yaml
property_filters:
  message_type: "relationship_request"
  status: "sent"
```

To add a new message type, create a trigger with the appropriate `message_type` filter.

## Message Status Values

All message types use these standard status values:

- `pending`: Just created, awaiting router processing
- `sent`: Routed by outbox handler, awaiting type-specific processing
- `delivered`: Successfully delivered to recipient's inbox
- `read`: Recipient has viewed the message
- `processed`: All processing complete (e.g., relationship created)
- `error`: Processing failed (check metadata for error details)

## Validation

Message validation happens at multiple stages:

1. **Schema validation**: Node type schema enforces required fields
2. **Router validation**: Ensures sender_id, recipient_id are valid
3. **Handler validation**: Type-specific business logic validation
4. **RLS validation**: Ensures sender has permission to send to recipient

## Error Handling

When message processing fails:

```json
{
  "status": "error",
  "metadata": {
    "error": {
      "code": "INVALID_RELATIONSHIP_TYPE",
      "message": "Relationship type 'invalid-type' does not exist",
      "timestamp": "2025-12-19T10:30:00Z",
      "retry_count": 3
    }
  }
}
```

Failed messages can be:
- Manually retried by updating status back to "pending"
- Archived for investigation
- Deleted if unrecoverable

## Best Practices

### 1. Use Appropriate Message Types

Choose the right message type for your use case:
- Use `relationship_request` for formal relationship establishment
- Use `chat` for informal communication
- Use `system_notification` for automated alerts

### 2. Set Expiration Dates

For time-sensitive messages:

```json
{
  "expires_at": "2025-01-15T00:00:00Z"
}
```

### 3. Include Context in Metadata

Store additional context for debugging:

```json
{
  "metadata": {
    "source": "mobile_app",
    "version": "1.2.3",
    "user_agent": "RaisinDB Mobile/1.2.3"
  }
}
```

### 4. Use Related Entity ID

Link messages to related entities:

```json
{
  "related_entity_id": "relationship-request-789"
}
```

### 5. Provide User-Friendly Subjects

Always include a descriptive subject:

```json
{
  "subject": "Relationship Request from Alice Smith"
}
```

## File References

- Message node type: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_message.yaml`
- Stewardship package manifest: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/manifest.yaml`
- Relationship request trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/.node.yaml`
- Relationship response trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-response/.node.yaml`
- Ward invitation trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-ward-invitation/.node.yaml`
