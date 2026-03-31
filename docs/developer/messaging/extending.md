# Extending the Messaging System

This guide shows you how to add new message types to RaisinDB's messaging system. We'll walk through creating a complete message type with processing logic, triggers, and tests.

## Overview

Adding a new message type involves:

1. Define the message type schema
2. Create a processing function
3. Create a trigger to invoke the function
4. Test the end-to-end flow
5. Document the message type

## Example: Adding a "task_assignment" Message Type

Let's create a message type for assigning tasks to users.

### Step 1: Define Message Type Schema

Message types are defined by their `message_type` property and `body` schema. Create documentation first:

```markdown
## task_assignment

Assigns a task to a user with due date and priority.

### Body Schema

{
  "task_title": string,         // Required: Task title
  "task_description": string,   // Optional: Detailed description
  "due_date": string,           // Required: ISO 8601 date
  "priority": "low" | "medium" | "high",  // Required
  "related_node_id": string,    // Optional: Related content node
  "metadata": object            // Optional: Additional context
}

### Example

{
  "message_type": "task_assignment",
  "subject": "Task Assignment: Review Q4 Budget",
  "recipient_id": "user-alice",
  "sender_id": "user-manager",
  "body": {
    "task_title": "Review Q4 Budget",
    "task_description": "Please review the Q4 budget proposal and provide feedback.",
    "due_date": "2025-12-25T17:00:00Z",
    "priority": "high",
    "related_node_id": "doc-budget-q4"
  }
}
```

### Step 2: Create Processing Function

Create a function to handle task assignment messages.

**File:** `builtin-packages/tasks/functions/lib/tasks/handle-task-assignment/index.js`

```javascript
/**
 * Handle task_assignment messages
 *
 * Creates an InboxTask node for the recipient and sends confirmation
 * to both sender and recipient.
 */

export default async function handleTaskAssignment(context, event) {
  const { nodeService, logger } = context;
  const message = event.node;

  logger.info(`Processing task assignment: ${message.id}`);

  try {
    // 1. Validate message body
    const { task_title, due_date, priority } = message.properties.body;

    if (!task_title || !due_date || !priority) {
      throw new Error('Missing required fields: task_title, due_date, priority');
    }

    // 2. Create InboxTask node for recipient
    const task = await nodeService.createNode({
      parent_path: `users/${message.properties.recipient_id}/inbox`,
      node_type: 'raisin:InboxTask',
      properties: {
        task_type: 'action',
        title: task_title,
        description: message.properties.body.task_description,
        due_date: due_date,
        priority: getPriorityNumber(priority),
        status: 'pending',
        flow_instance_ref: null,  // Not from a flow
        step_id: 'task_assignment',
        options: [
          { value: 'complete', label: 'Mark Complete', style: 'success' },
          { value: 'defer', label: 'Defer', style: 'default' }
        ],
        metadata: {
          assigned_by: message.properties.sender_id,
          message_id: message.id,
          related_node_id: message.properties.body.related_node_id
        }
      }
    });

    logger.info(`Created InboxTask: ${task.id}`);

    // 3. Create message in recipient's inbox
    await nodeService.createNode({
      parent_path: `users/${message.properties.recipient_id}/inbox`,
      node_type: 'raisin:Message',
      properties: {
        message_type: 'task_assignment',
        subject: message.properties.subject,
        body: message.properties.body,
        recipient_id: message.properties.recipient_id,
        sender_id: message.properties.sender_id,
        status: 'delivered',
        related_entity_id: task.id,  // Link to InboxTask
        metadata: {
          task_id: task.id
        }
      }
    });

    // 4. Update original message status
    await nodeService.updateNode(message.id, {
      status: 'processed',
      metadata: {
        ...message.properties.metadata,
        processed_at: new Date().toISOString(),
        task_id: task.id
      }
    });

    // 5. Send confirmation to sender
    await nodeService.createNode({
      parent_path: `users/${message.properties.sender_id}/inbox`,
      node_type: 'raisin:Message',
      properties: {
        message_type: 'system_notification',
        subject: `Task assigned to ${message.properties.recipient_id}`,
        body: {
          notification_type: 'task_assigned',
          priority: 'low',
          title: 'Task Assigned',
          message: `Your task "${task_title}" has been assigned.`,
          action_url: `/tasks/${task.id}`,
          action_label: 'View Task'
        },
        recipient_id: message.properties.sender_id,
        sender_id: 'system',
        status: 'delivered',
        related_entity_id: task.id
      }
    });

    logger.info(`Task assignment processed successfully`);

  } catch (error) {
    logger.error(`Failed to process task assignment: ${error.message}`);

    // Update message with error
    await nodeService.updateNode(message.id, {
      status: 'error',
      metadata: {
        ...message.properties.metadata,
        error: {
          message: error.message,
          timestamp: new Date().toISOString()
        }
      }
    });

    // Re-throw for retry mechanism
    throw error;
  }
}

function getPriorityNumber(priority) {
  const map = { high: 1, medium: 3, low: 5 };
  return map[priority] || 3;
}
```

### Step 3: Create Trigger Definition

Create a trigger to invoke your handler when task assignment messages are sent.

**File:** `builtin-packages/tasks/functions/triggers/process-task-assignment/.node.yaml`

```yaml
node_type: raisin:Trigger
properties:
  title: Process Task Assignment Messages
  name: tasks-task-assignment
  description: Handles task assignment messages by creating InboxTask nodes for recipients
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
  function_path: /functions/tasks/handlers/handle-task-assignment
```

### Step 4: Register in Package Manifest

Add the trigger to your package manifest.

**File:** `builtin-packages/tasks/manifest.yaml`

```yaml
name: raisin-tasks
version: 1.0.0
title: RaisinDB Tasks Package
description: Task management and assignment system
builtin: false

provides:
  triggers:
    - /functions/triggers/process-task-assignment

  # If you created custom node types
  nodetypes:
    - raisin:Task

dependencies:
  - raisin-stewardship  # For message routing
```

### Step 5: Write Tests

Create integration tests for your message type.

**File:** `builtin-packages/tasks/functions/lib/tasks/handle-task-assignment/test.js`

```javascript
import { describe, it, expect, beforeEach } from 'vitest';
import { createTestContext } from '@raisindb/test-utils';

describe('Task Assignment Message Handler', () => {
  let context;

  beforeEach(async () => {
    context = await createTestContext();

    // Create test users
    await context.createUser({ id: 'user-alice', email: 'alice@example.com' });
    await context.createUser({ id: 'user-manager', email: 'manager@example.com' });
  });

  it('should create InboxTask for recipient', async () => {
    // Create task assignment message
    const message = await context.nodeService.createNode({
      parent_path: 'users/user-manager/outbox',
      node_type: 'raisin:Message',
      properties: {
        message_type: 'task_assignment',
        subject: 'Review Q4 Budget',
        recipient_id: 'user-alice',
        sender_id: 'user-manager',
        status: 'pending',
        body: {
          task_title: 'Review Q4 Budget',
          task_description: 'Please review and provide feedback',
          due_date: '2025-12-25T17:00:00Z',
          priority: 'high'
        }
      }
    });

    // Wait for processing
    await context.waitForJobCompletion();

    // Verify InboxTask created
    const tasks = await context.nodeService.listChildren('users/user-alice/inbox', {
      node_type: 'raisin:InboxTask'
    });

    expect(tasks).toHaveLength(1);
    expect(tasks[0].properties.title).toBe('Review Q4 Budget');
    expect(tasks[0].properties.priority).toBe(1);  // high = 1
    expect(tasks[0].properties.status).toBe('pending');

    // Verify message in inbox
    const messages = await context.nodeService.listChildren('users/user-alice/inbox', {
      node_type: 'raisin:Message'
    });

    expect(messages).toHaveLength(1);
    expect(messages[0].properties.message_type).toBe('task_assignment');
    expect(messages[0].properties.status).toBe('delivered');

    // Verify original message marked processed
    const originalMessage = await context.nodeService.getNode(message.id);
    expect(originalMessage.properties.status).toBe('processed');
  });

  it('should send confirmation to sender', async () => {
    await context.nodeService.createNode({
      parent_path: 'users/user-manager/outbox',
      node_type: 'raisin:Message',
      properties: {
        message_type: 'task_assignment',
        subject: 'Review Q4 Budget',
        recipient_id: 'user-alice',
        sender_id: 'user-manager',
        status: 'pending',
        body: {
          task_title: 'Review Q4 Budget',
          due_date: '2025-12-25T17:00:00Z',
          priority: 'high'
        }
      }
    });

    await context.waitForJobCompletion();

    // Verify confirmation in sender's inbox
    const confirmations = await context.nodeService.listChildren('users/user-manager/inbox', {
      node_type: 'raisin:Message',
      properties: {
        message_type: 'system_notification'
      }
    });

    expect(confirmations).toHaveLength(1);
    expect(confirmations[0].properties.body.title).toBe('Task Assigned');
  });

  it('should handle missing required fields', async () => {
    const message = await context.nodeService.createNode({
      parent_path: 'users/user-manager/outbox',
      node_type: 'raisin:Message',
      properties: {
        message_type: 'task_assignment',
        subject: 'Invalid Task',
        recipient_id: 'user-alice',
        sender_id: 'user-manager',
        status: 'pending',
        body: {
          // Missing task_title, due_date, priority
        }
      }
    });

    await context.waitForJobCompletion();

    // Verify error status
    const updatedMessage = await context.nodeService.getNode(message.id);
    expect(updatedMessage.properties.status).toBe('error');
    expect(updatedMessage.properties.metadata.error).toBeDefined();
  });
});
```

### Step 6: Run Tests

```bash
# Install dependencies
npm install

# Run tests
npm test builtin-packages/tasks/functions/lib/tasks/handle-task-assignment/test.js
```

## Best Practices

### 1. Validate Message Body

Always validate the message body schema:

```javascript
function validateTaskAssignment(body) {
  const errors = [];

  if (!body.task_title) {
    errors.push('task_title is required');
  }

  if (!body.due_date) {
    errors.push('due_date is required');
  }

  if (!['low', 'medium', 'high'].includes(body.priority)) {
    errors.push('priority must be low, medium, or high');
  }

  if (errors.length > 0) {
    throw new Error(`Validation failed: ${errors.join(', ')}`);
  }
}
```

### 2. Make Handlers Idempotent

Handlers may be retried, so ensure they're safe to run multiple times:

```javascript
export default async function handleMessage(context, event) {
  const message = event.node;

  // Check if already processed
  if (message.properties.status === 'processed') {
    context.logger.info('Message already processed, skipping');
    return;
  }

  // Check if task already exists
  const existingTask = await context.nodeService.findNode({
    path: `users/${message.properties.recipient_id}/inbox/*`,
    node_type: 'raisin:InboxTask',
    properties: {
      'metadata.message_id': message.id
    }
  });

  if (existingTask) {
    context.logger.info('Task already exists, marking message as processed');
    await context.nodeService.updateNode(message.id, {
      status: 'processed'
    });
    return;
  }

  // Proceed with processing...
}
```

### 3. Use Transactions for Atomicity

Wrap multiple operations in a transaction:

```javascript
await context.transaction.run(async (tx) => {
  // All operations in this block are atomic

  const task = await tx.createNode({ /* ... */ });
  const message = await tx.createNode({ /* ... */ });
  await tx.updateNode(originalMessageId, { status: 'processed' });

  // If any operation fails, all are rolled back
});
```

### 4. Provide User-Friendly Error Messages

```javascript
try {
  await processMessage(message);
} catch (error) {
  const userMessage = getUserFriendlyError(error);

  await nodeService.createNode({
    parent_path: `users/${message.properties.sender_id}/inbox`,
    node_type: 'raisin:Message',
    properties: {
      message_type: 'system_notification',
      subject: 'Message Delivery Failed',
      body: {
        notification_type: 'error',
        priority: 'high',
        title: 'Message Delivery Failed',
        message: userMessage,
        metadata: {
          original_message_id: message.id,
          error_code: error.code
        }
      },
      recipient_id: message.properties.sender_id,
      sender_id: 'system',
      status: 'delivered'
    }
  });
}

function getUserFriendlyError(error) {
  const messages = {
    'RECIPIENT_NOT_FOUND': 'The recipient user does not exist.',
    'INVALID_DUE_DATE': 'The due date is invalid or in the past.',
    'PERMISSION_DENIED': 'You do not have permission to assign tasks to this user.'
  };

  return messages[error.code] || 'An unexpected error occurred. Please try again.';
}
```

### 5. Use Related Entity IDs

Link messages to related entities for traceability:

```javascript
await nodeService.createNode({
  properties: {
    message_type: 'task_assignment',
    related_entity_id: taskId,  // Link to created task
    metadata: {
      task_id: taskId,
      workflow_id: workflowId,
      source: 'task_system'
    }
  }
});
```

### 6. Log Generously

Add detailed logging for debugging:

```javascript
logger.info(`Processing ${message.properties.message_type} message`, {
  message_id: message.id,
  sender_id: message.properties.sender_id,
  recipient_id: message.properties.recipient_id
});

logger.debug('Message body', { body: message.properties.body });

logger.info(`Created task ${task.id} for user ${recipientId}`);

logger.error('Failed to process message', {
  message_id: message.id,
  error: error.message,
  stack: error.stack
});
```

## Advanced Patterns

### Pattern 1: Multi-Step Workflows

Create message chains for complex workflows:

```javascript
// Step 1: Send task assignment
await createMessage({ message_type: 'task_assignment', ... });

// Step 2: Handler creates InboxTask
// Step 3: User completes task

// Step 4: Task completion trigger sends confirmation
await createMessage({
  message_type: 'task_completion',
  recipient_id: originalSenderId,
  body: {
    task_id: taskId,
    completed_by: userId,
    completed_at: new Date().toISOString()
  }
});
```

### Pattern 2: Conditional Processing

Route messages differently based on content:

```javascript
export default async function handleMessage(context, event) {
  const message = event.node;
  const { priority } = message.properties.body;

  if (priority === 'urgent') {
    // Send SMS/push notification
    await sendUrgentNotification(message);
  }

  if (message.properties.body.requires_approval) {
    // Create approval workflow
    await createApprovalFlow(message);
  } else {
    // Process immediately
    await processTask(message);
  }
}
```

### Pattern 3: Bulk Operations

Handle messages that affect multiple users:

```javascript
export default async function handleBroadcast(context, event) {
  const message = event.node;
  const { recipient_group_id } = message.properties.body;

  // Get all users in group
  const users = await context.nodeService.getUsersInGroup(recipient_group_id);

  // Create individual messages for each user
  for (const user of users) {
    await context.nodeService.createNode({
      parent_path: `users/${user.id}/inbox`,
      node_type: 'raisin:Message',
      properties: {
        message_type: 'group_notification',
        recipient_id: user.id,
        sender_id: message.properties.sender_id,
        body: message.properties.body,
        status: 'delivered',
        related_entity_id: message.id  // Link to original broadcast
      }
    });
  }

  // Mark broadcast as processed
  await context.nodeService.updateNode(message.id, {
    status: 'processed',
    metadata: {
      delivered_count: users.length,
      delivered_at: new Date().toISOString()
    }
  });
}
```

### Pattern 4: External Integration

Integrate with external services:

```javascript
export default async function handleExternalTask(context, event) {
  const message = event.node;

  // 1. Create internal task
  const task = await createInboxTask(context, message);

  // 2. Create task in external system (e.g., Jira, Asana)
  const externalTask = await fetch('https://api.external.com/tasks', {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${process.env.EXTERNAL_API_KEY}` },
    body: JSON.stringify({
      title: message.properties.body.task_title,
      description: message.properties.body.task_description,
      assignee: message.properties.recipient_id,
      due_date: message.properties.body.due_date
    })
  }).then(res => res.json());

  // 3. Link internal and external tasks
  await context.nodeService.updateNode(task.id, {
    metadata: {
      ...task.properties.metadata,
      external_task_id: externalTask.id,
      external_task_url: externalTask.url
    }
  });

  // 4. Set up webhook to sync status changes
  await registerWebhook({
    url: `https://your-instance.com/webhooks/external-task/${externalTask.id}`,
    events: ['task.updated', 'task.completed']
  });
}
```

## Testing Strategies

### Unit Tests

Test individual functions:

```javascript
describe('getPriorityNumber', () => {
  it('should convert high to 1', () => {
    expect(getPriorityNumber('high')).toBe(1);
  });

  it('should default to 3 for unknown priority', () => {
    expect(getPriorityNumber('unknown')).toBe(3);
  });
});
```

### Integration Tests

Test full message flow:

```javascript
it('should process task assignment end-to-end', async () => {
  // 1. Create message in outbox
  const message = await createMessage({ /* ... */ });

  // 2. Wait for async processing
  await waitForJobCompletion();

  // 3. Verify outcomes
  expect(await getInboxTask(recipientId)).toBeDefined();
  expect(await getInboxMessage(recipientId)).toBeDefined();
  expect(message.status).toBe('processed');
});
```

### Manual Testing

Test via API or UI:

```bash
# Create message via REST API
curl -X POST http://localhost:8080/api/nodes \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer ${TOKEN}" \
  -d '{
    "parent_path": "users/user-123/outbox",
    "node_type": "raisin:Message",
    "properties": {
      "message_type": "task_assignment",
      "subject": "Test Task",
      "recipient_id": "user-456",
      "sender_id": "user-123",
      "body": {
        "task_title": "Test Task",
        "due_date": "2025-12-31T23:59:59Z",
        "priority": "high"
      }
    }
  }'

# Verify task created
curl http://localhost:8080/api/nodes/users/user-456/inbox \
  -H "Authorization: Bearer ${TOKEN}"
```

## Deployment

### 1. Package Your Extension

```yaml
# builtin-packages/tasks/manifest.yaml
name: raisin-tasks
version: 1.0.0
builtin: false  # Set to true for core packages

provides:
  triggers:
    - /functions/triggers/process-task-assignment
  nodetypes:
    - raisin:Task
```

### 2. Install Package

```bash
# Copy package to builtin-packages/
cp -r tasks builtin-packages/

# Restart server to load package
cargo run
```

### 3. Verify Installation

```sql
-- Check if trigger is registered
SELECT * FROM nodes
WHERE node_type = 'raisin:Trigger'
  AND properties->>'name' = 'tasks-task-assignment';
```

## Troubleshooting

### Message Not Processing

**Check trigger enabled:**
```sql
SELECT * FROM nodes
WHERE node_type = 'raisin:Trigger'
  AND properties->>'name' = 'tasks-task-assignment';
```

**Check job queue:**
```sql
SELECT * FROM jobs
WHERE job_type = 'trigger_execution'
  AND status = 'failed'
ORDER BY created_at DESC;
```

**Check logs:**
```bash
tail -f logs/raisindb.log | grep task-assignment
```

### Handler Errors

**View error details:**
```javascript
const message = await getNode(messageId);
console.log(message.properties.metadata.error);
```

**Retry failed message:**
```javascript
await updateNode(messageId, {
  status: 'pending',
  metadata: {
    ...message.properties.metadata,
    error: null
  }
});
```

## Documentation Template

When documenting your message type, use this template:

```markdown
## {message_type}

Brief description of what this message type does.

### When Used

- Use case 1
- Use case 2
- Use case 3

### Body Schema

\`\`\`typescript
{
  "field1": string,      // Required: Description
  "field2": number,      // Optional: Description
  "field3": object       // Optional: Description
}
\`\`\`

### Example

\`\`\`json
{
  "message_type": "your_type",
  "subject": "Example Subject",
  "recipient_id": "user-123",
  "sender_id": "user-456",
  "body": {
    "field1": "value",
    "field2": 42
  }
}
\`\`\`

### Processing Flow

1. Step 1
2. Step 2
3. Step 3

### Expected Outcomes

**Success:**
- Outcome 1
- Outcome 2

**Failure:**
- Error scenario 1
- Error scenario 2

### Related Files

- Handler: `path/to/handler.js`
- Trigger: `path/to/trigger.yaml`
- Tests: `path/to/test.js`
```

## File References

- Message node type: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_message.yaml`
- Trigger node type: `/Users/senol/Projects/maravilla-labs/repos/raisindb/crates/raisin-core/global_nodetypes/raisin_trigger.yaml`
- Example router trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-messaging/content/functions/triggers/on-outbox-create/.node.yaml`
- Example handler trigger: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/content/functions/triggers/process-relationship-request/.node.yaml`
- Stewardship package manifest: `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/manifest.yaml`
