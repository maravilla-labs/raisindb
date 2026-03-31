# Extending the Stewardship System

This guide shows how to extend the RaisinDB Stewardship System with custom relation types, message types, triggers, and RLS conditions.

## Overview

The stewardship system is designed to be extensible. You can:

1. **Add custom relation types** - Define new relationship types
2. **Add custom message types** - Create new message-based workflows
3. **Create custom triggers** - Automate custom workflows
4. **Extend RLS conditions** - Add stewardship-aware permissions
5. **Create custom functions** - Build stewardship logic

---

## Adding Custom Relation Types

Relation types are defined as `raisin:RelationType` nodes and automatically integrated into the stewardship system.

### Step 1: Create RelationType Node

Create a new node in `/relation-types/`:

```yaml
# Path: /relation-types/mentor-of/.node.yaml
node_type: raisin:RelationType
properties:
  relation_name: "MENTOR_OF"
  title: "Mentor Of"
  description: "Professional mentorship relationship"
  category: "professional"
  inverse_relation_name: "MENTEE_OF"
  bidirectional: false
  implies_stewardship: true    # This relation grants stewardship
  requires_minor: false         # Applies to all ages
  icon: "academic-cap"
  color: "#3b82f6"
```

### Step 2: Update StewardshipConfig

Add the new relation type to the stewardship configuration:

```yaml
# Path: /config/stewardship/.node.yaml
node_type: raisin:StewardshipConfig
properties:
  enabled: true
  stewardship_relation_types:
    - "PARENT_OF"
    - "GUARDIAN_OF"
    - "MENTOR_OF"          # Add new type
  # ... other config
```

### Step 3: Test the Relation Type

```bash
# Create graph relation
POST /api/v1/repositories/{repo}/graph/relations
Content-Type: application/json

{
  "source_node_id": "{mentor-user-id}",
  "target_node_id": "{mentee-user-id}",
  "relation_type": "MENTOR_OF"
}

# Verify stewardship
POST /api/v1/repositories/{repo}/functions/invoke
Content-Type: application/json

{
  "function_path": "/functions/lib/stewardship/is-steward-of",
  "input": {
    "steward_user_path": "/users/internal/mentor",
    "ward_user_path": "/users/internal/mentee"
  }
}

# Should return: { "success": true, "is_steward": true, "relation_type": "MENTOR_OF" }
```

### Advanced: Age-Conditional Relations

For relations that only apply to minors (like `PARENT_OF`):

```yaml
node_type: raisin:RelationType
properties:
  relation_name: "FOSTER_PARENT_OF"
  title: "Foster Parent Of"
  description: "Foster parenting relationship"
  category: "household"
  inverse_relation_name: "FOSTER_CHILD_OF"
  bidirectional: false
  implies_stewardship: true
  requires_minor: true         # Only applies if target is minor
  icon: "heart"
  color: "#ec4899"
```

---

## Adding Custom Message Types

Message types enable workflow automation via triggers.

### Step 1: Define Message Structure

Document your message type:

```javascript
// Message Type: "permission_request"
// Purpose: Request temporary access to a specific resource

{
  message_type: "permission_request",
  subject: "Permission request for {resource}",
  body: {
    permission_type: "read" | "write" | "admin",
    resource_path: "/path/to/resource",
    justification: "Reason for request",
    expires_at: "2025-12-31T00:00:00Z"
  },
  recipient_id: "user:admin",
  sender_id: "user:alice",
  status: "pending"
}
```

### Step 2: Create Handler Function

Create `/functions/custom/handlers/handle-permission-request/index.js`:

```javascript
/**
 * Handles permission request messages
 */
async function handlePermissionRequest(input) {
    const { node, event } = input;
    const { recipient_id, sender_id, body } = node.properties;

    try {
        console.log(`Processing permission request from ${sender_id} to ${recipient_id}`);

        // Validate request
        if (!body.resource_path || !body.permission_type) {
            throw new Error("Invalid request: missing required fields");
        }

        // Check if resource exists
        const resource = await raisin.nodes.get(
            "raisin:access_control",
            body.resource_path
        );
        if (!resource) {
            throw new Error(`Resource not found: ${body.resource_path}`);
        }

        // Create inbox message for recipient
        const recipientPath = recipient_id.replace("user:", "/users/internal/");
        const inboxPath = `${recipientPath}/inbox/req-${Date.now()}`;

        await raisin.nodes.create("raisin:access_control", {
            path: inboxPath,
            node_type: "raisin:Message",
            properties: {
                message_type: "permission_request",
                subject: node.properties.subject,
                body: body,
                recipient_id: recipient_id,
                sender_id: sender_id,
                status: "delivered",
                related_entity_id: node.id
            }
        });

        // Update original message status
        await raisin.nodes.update("raisin:access_control", node.path, {
            properties: {
                status: "delivered"
            }
        });

        console.log(`Permission request delivered to ${recipient_id}`);

        return {
            success: true,
            message: "Permission request delivered"
        };

    } catch (error) {
        console.error(`Failed to process permission request: ${error.message}`);
        return {
            success: false,
            error: error.message
        };
    }
}

module.exports = { handlePermissionRequest };
```

### Step 3: Create Function Definition

Create `/functions/custom/handlers/handle-permission-request/.node.yaml`:

```yaml
node_type: raisin:Function
properties:
  title: Handle Permission Request
  description: Processes permission request messages
  language: JavaScript
  entry_file: index.js:handlePermissionRequest
  execution_mode: Async
  enabled: true
  resource_limits:
    timeout_ms: 10000
    max_memory_bytes: 33554432
```

### Step 4: Create Trigger

Create trigger to invoke the handler:

```yaml
# Path: /functions/custom/triggers/process-permission-request/.node.yaml
node_type: raisin:Trigger
properties:
  title: Process Permission Request Messages
  name: custom-permission-request
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
  function_path: /functions/custom/handlers/handle-permission-request
```

### Step 5: Create Response Handler

For bidirectional workflows, create a response handler:

```javascript
// /functions/custom/handlers/handle-permission-response/index.js

async function handlePermissionResponse(input) {
    const { node } = input;
    const { body, sender_id, related_entity_id } = node.properties;

    try {
        if (body.approved) {
            // Grant temporary permission
            await raisin.nodes.create("raisin:access_control", {
                path: `/permissions/temp-perm-${Date.now()}`,
                node_type: "raisin:Permission",
                properties: {
                    user_id: body.original_sender_id,
                    permission_type: body.permission_type,
                    resource_path: body.resource_path,
                    valid_until: body.expires_at,
                    status: "active"
                }
            });

            console.log(`Permission granted to ${body.original_sender_id}`);
        } else {
            console.log(`Permission denied for ${body.original_sender_id}`);
        }

        // Notify original requester
        // ... create notification message

        return { success: true };

    } catch (error) {
        return { success: false, error: error.message };
    }
}

module.exports = { handlePermissionResponse };
```

### Step 6: Test the Workflow

```bash
# User Alice creates permission request
POST /api/v1/repositories/{repo}/workspaces/raisin:access_control/nodes
Authorization: Bearer {alice-token}

{
  "path": "/users/alice/outbox/req-001",
  "node_type": "raisin:Message",
  "properties": {
    "message_type": "permission_request",
    "subject": "Request access to sensitive docs",
    "body": {
      "permission_type": "read",
      "resource_path": "/documents/sensitive",
      "justification": "Need for project review"
    },
    "recipient_id": "user:admin",
    "sender_id": "user:alice",
    "status": "pending"
  }
}

# Router trigger fires -> sets status to "sent"
# Handler trigger fires -> creates inbox message for admin
```

---

## Creating Custom Triggers

Triggers automate workflows based on events.

### Trigger Types

#### 1. Node Event Trigger

Fires when nodes are created, updated, or deleted:

```yaml
node_type: raisin:Trigger
properties:
  title: Notify on Medical Record Update
  name: notify-medical-update
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Updated
  filters:
    paths:
      - "users/*/medical-records/**"
    node_types:
      - raisin:MedicalRecord
  function_path: /functions/custom/notify-medical-update
```

#### 2. Schedule Trigger

Fires on a cron schedule:

```yaml
node_type: raisin:Trigger
properties:
  title: Daily Stewardship Summary
  name: daily-stewardship-summary
  enabled: true
  trigger_type: schedule
  config:
    cron_expression: "0 9 * * *"  # Every day at 9 AM
  function_path: /functions/custom/daily-summary
```

#### 3. HTTP Trigger

Fires on HTTP webhook:

```yaml
node_type: raisin:Trigger
properties:
  title: External Stewardship Request
  name: external-steward-request
  enabled: true
  trigger_type: http
  webhook_id: "abc123xyz"  # Auto-generated
  config:
    methods: ["POST"]
    path_pattern: "/stewardship/external"
    default_sync: true
  function_path: /functions/custom/handle-external-request
```

### Advanced: REL Conditions in Triggers

Use REL expressions for complex filtering:

```yaml
node_type: raisin:Trigger
properties:
  title: High Priority Medical Updates
  name: urgent-medical-notify
  enabled: true
  trigger_type: node_event
  config:
    event_kinds:
      - Updated
  filters:
    node_types:
      - raisin:MedicalRecord
    rel_condition: |
      node.properties.priority >= 8 &&
      node.path.descendantOf('/users') &&
      input.changes.properties.contains('diagnosis')
  function_path: /functions/custom/urgent-notify
```

See [REL Documentation](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/REL.md) for syntax.

---

## Extending RLS Conditions

Row-Level Security can be extended with stewardship-aware conditions.

### Basic Stewardship Condition

Allow stewards to read ward data:

```yaml
# In raisin:Role node
permissions:
  - resource: "users/**/personal-data/**"
    action: "read"
    condition: |
      // User can read their own data OR their ward's data
      node.properties.owner_id == auth.local_user_id ||
      node.properties.owner_id == auth.acting_as_ward
```

### Relation-Specific Conditions

Only allow specific relation types:

```yaml
permissions:
  - resource: "users/**/medical-records/**"
    action: "read"
    condition: |
      // Only GUARDIAN_OF relation grants access to medical records
      auth.acting_as_ward != null &&
      auth.active_stewardship_source == 'GUARDIAN_OF' &&
      node.path.descendantOf('/users/' + auth.acting_as_ward)
```

### Function-Based Conditions

Call functions to check stewardship:

```yaml
permissions:
  - resource: "users/**/documents/**"
    action: "write"
    condition: |
      // Use is-steward-of function to verify relationship
      const wardPath = node.path.parent(2);
      const result = await raisin.functions.invoke(
        "/functions/lib/stewardship/is-steward-of",
        {
          steward_user_path: auth.user_path,
          ward_user_path: wardPath
        }
      );
      return result.success && result.is_steward;
```

### Time-Limited Stewardship

Check stewardship overrides with expiration:

```yaml
permissions:
  - resource: "users/**/temp-access/**"
    action: "read"
    condition: |
      // Check if valid stewardship override exists
      const overrides = await raisin.sql.query(
        `SELECT * FROM nodes
         WHERE node_type = 'raisin:StewardshipOverride'
           AND properties->>'steward_id' = $1
           AND properties->>'ward_id' = $2
           AND properties->>'status' = 'active'
           AND (properties->>'valid_until')::timestamp > NOW()`,
        [auth.user_id, node.properties.owner_id]
      );
      return overrides.length > 0;
```

---

## Creating Custom Stewardship Functions

Extend the stewardship API with custom logic.

### Example: Get Stewardship History

```javascript
// /functions/custom/get-stewardship-history/index.js

/**
 * Get complete stewardship history for a user
 */
async function getStewardshipHistory(input) {
    const { user_path, workspace = "raisin:access_control" } = input;

    try {
        const user = await raisin.nodes.get(workspace, user_path);
        if (!user) {
            return { success: false, error: "User not found" };
        }

        // Query all stewardship-related messages
        const messages = await raisin.sql.query(
            `SELECT
                id,
                path,
                properties->>'message_type' as message_type,
                properties->>'subject' as subject,
                properties->>'sender_id' as sender_id,
                properties->>'recipient_id' as recipient_id,
                properties->>'status' as status,
                created_at
             FROM nodes
             WHERE node_type = 'raisin:Message'
               AND workspace = $1
               AND (
                 properties->>'sender_id' = $2
                 OR properties->>'recipient_id' = $2
               )
               AND properties->>'message_type' IN ('relationship_request', 'relationship_response')
             ORDER BY created_at DESC`,
            [workspace, user.id]
        );

        // Query current relationships
        const currentStewards = await raisin.functions.invoke(
            "/functions/lib/stewardship/get-stewards",
            { ward_user_path: user_path, workspace }
        );

        const currentWards = await raisin.functions.invoke(
            "/functions/lib/stewardship/get-wards",
            { steward_user_path: user_path, workspace }
        );

        return {
            success: true,
            history: {
                messages: messages,
                current_stewards: currentStewards.stewards || [],
                current_wards: currentWards.wards || []
            }
        };

    } catch (error) {
        return { success: false, error: error.message };
    }
}

module.exports = { getStewardshipHistory };
```

### Example: Batch Stewardship Check

```javascript
// /functions/custom/batch-stewardship-check/index.js

/**
 * Check stewardship for multiple user pairs
 */
async function batchStewardshipCheck(input) {
    const { pairs, workspace = "raisin:access_control" } = input;

    if (!Array.isArray(pairs)) {
        return { success: false, error: "pairs must be an array" };
    }

    try {
        const results = [];

        for (const pair of pairs) {
            const result = await raisin.functions.invoke(
                "/functions/lib/stewardship/is-steward-of",
                {
                    steward_user_path: pair.steward_path,
                    ward_user_path: pair.ward_path,
                    workspace: workspace
                }
            );

            results.push({
                steward_path: pair.steward_path,
                ward_path: pair.ward_path,
                is_steward: result.is_steward || false,
                relation_type: result.relation_type || null
            });
        }

        return {
            success: true,
            results: results
        };

    } catch (error) {
        return { success: false, error: error.message };
    }
}

module.exports = { batchStewardshipCheck };
```

---

## Package Structure for Custom Extensions

Organize custom extensions in a dedicated package:

```
/builtin-packages/myapp-stewardship/
├── manifest.yaml
├── nodetypes/
│   └── custom-relation-type.yaml
├── content/
│   └── raisin:access_control/
│       └── relation-types/
│           └── custom-relation/
│               └── .node.yaml
├── functions/
│   ├── lib/
│   │   └── custom/
│   │       ├── get-history/
│   │       │   ├── .node.yaml
│   │       │   └── index.js
│   │       └── batch-check/
│   │           ├── .node.yaml
│   │           └── index.js
│   └── triggers/
│       └── custom-handler/
│           ├── .node.yaml
│           └── index.js
└── README.md
```

### Package Manifest

```yaml
# manifest.yaml
name: myapp-stewardship
version: 1.0.0
description: Custom stewardship extensions for MyApp
dependencies:
  - raisin-stewardship

nodetypes:
  - path: /nodetypes/custom-relation-type.yaml

content:
  - path: /content/raisin:access_control/relation-types/custom-relation

functions:
  - path: /functions/lib/custom/get-history
    handler: getStewardshipHistory
  - path: /functions/lib/custom/batch-check
    handler: batchStewardshipCheck
  - path: /functions/triggers/custom-handler
    handler: handleCustomMessage
```

---

## Testing Custom Extensions

### Unit Tests for Functions

```javascript
// tests/stewardship/custom-functions.test.js

describe("Custom Stewardship Functions", () => {
    test("getStewardshipHistory returns complete history", async () => {
        const result = await invokeFunction(
            "/functions/custom/get-stewardship-history",
            { user_path: "/users/internal/alice" }
        );

        expect(result.success).toBe(true);
        expect(result.history).toHaveProperty("messages");
        expect(result.history).toHaveProperty("current_stewards");
        expect(result.history).toHaveProperty("current_wards");
    });

    test("batchStewardshipCheck handles multiple pairs", async () => {
        const result = await invokeFunction(
            "/functions/custom/batch-stewardship-check",
            {
                pairs: [
                    { steward_path: "/users/internal/alice", ward_path: "/users/internal/bob" },
                    { steward_path: "/users/internal/alice", ward_path: "/users/internal/charlie" }
                ]
            }
        );

        expect(result.success).toBe(true);
        expect(result.results).toHaveLength(2);
    });
});
```

### Integration Tests

```bash
# Test custom relation type
curl -X POST http://localhost:8080/api/v1/repositories/test/graph/relations \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "source_node_id": "{mentor-id}",
    "target_node_id": "{mentee-id}",
    "relation_type": "MENTOR_OF"
  }'

# Verify stewardship
curl -X POST http://localhost:8080/api/v1/repositories/test/functions/invoke \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "function_path": "/functions/lib/stewardship/is-steward-of",
    "input": {
      "steward_user_path": "/users/internal/mentor",
      "ward_user_path": "/users/internal/mentee"
    }
  }'
```

---

## Best Practices

### 1. Design Principles

- **Keep It Simple**: Start with basic extensions, add complexity as needed
- **Follow Conventions**: Use existing patterns (message types, triggers, functions)
- **Document Everything**: Clear comments and documentation
- **Version Your Extensions**: Track changes to custom types and workflows

### 2. Security

- **Validate Input**: Always validate function inputs
- **Check Permissions**: Verify user has permission before granting access
- **Audit Actions**: Log all stewardship changes
- **Limit Scope**: Don't grant more access than necessary

### 3. Performance

- **Cache Configuration**: Cache frequently accessed config nodes
- **Batch Operations**: Process multiple items in batches
- **Optimize Queries**: Use SQL efficiently, avoid N+1 queries
- **Set Timeouts**: Configure appropriate function timeouts

### 4. Error Handling

- **Return Structured Errors**: Use `{ success, error }` pattern
- **Log Errors**: Use `console.error()` for debugging
- **Handle Edge Cases**: Test with invalid inputs
- **Graceful Degradation**: Don't break the system on errors

### 5. Testing

- **Test All Paths**: Happy path, error cases, edge cases
- **Integration Tests**: Test end-to-end workflows
- **Load Testing**: Verify performance under load
- **Security Testing**: Test permission boundaries

---

## Migration Guide

### Migrating from Built-in to Custom Relation Types

If you need to customize a built-in relation type:

1. **Create new custom type**: Don't modify built-in types
2. **Migrate existing relations**: Copy graph edges to new type
3. **Update configuration**: Add new type to StewardshipConfig
4. **Update permissions**: Adjust RLS conditions
5. **Deprecate old type**: Mark old type as deprecated

---

## See Also

- [Node Types](./node-types.md) - Stewardship node type definitions
- [Triggers](./triggers.md) - How triggers work
- [Functions](./functions.md) - QuickJS function reference
- [API Reference](./api-reference.md) - REST API endpoints
- [REL Documentation](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/REL.md) - Expression language syntax
