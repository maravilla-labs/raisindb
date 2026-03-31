# Stewardship API Reference

This document provides a complete API reference for working with the RaisinDB Stewardship System.

## Overview

The stewardship system can be accessed through:

1. **REST API** - Standard HTTP endpoints for CRUD operations
2. **SQL Queries** - Direct SQL access to stewardship data
3. **Function Invocation** - Call QuickJS functions for stewardship logic
4. **Graph Queries** - NEIGHBORS function for relationship traversal
5. **RLS Context** - Stewardship-aware permission conditions

---

## REST API Endpoints

### Base URL

```
https://{your-domain}/api/v1/repositories/{repo}/workspaces/{workspace}
```

Default workspace for stewardship: `raisin:access_control`

### Authentication

All requests require authentication via Bearer token:

```
Authorization: Bearer {jwt-token}
```

---

## Node Operations

### Create Message (Relationship Request)

Create a message in the user's outbox to initiate a relationship request.

**Endpoint**: `POST /nodes`

**Request Body**:

```json
{
  "path": "/users/alice/outbox/msg-{uuid}",
  "node_type": "raisin:Message",
  "properties": {
    "message_type": "relationship_request",
    "subject": "Request to be your guardian",
    "body": {
      "relation_type": "GUARDIAN_OF",
      "requestor_name": "Alice Smith",
      "requestor_email": "alice@example.com",
      "message": "I would like to be your legal guardian"
    },
    "recipient_id": "user:bob-jones",
    "sender_id": "user:alice-smith",
    "status": "pending",
    "expires_at": "2025-12-26T00:00:00Z"
  }
}
```

**Response**: `201 Created`

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "path": "/users/alice/outbox/msg-123",
  "node_type": "raisin:Message",
  "workspace": "raisin:access_control",
  "properties": { ... },
  "created_at": "2025-12-19T10:30:00Z",
  "updated_at": "2025-12-19T10:30:00Z"
}
```

---

### Create Relationship Response

Create a response message to accept or reject a relationship request.

**Endpoint**: `POST /nodes`

**Request Body**:

```json
{
  "path": "/users/bob/outbox/msg-{uuid}",
  "node_type": "raisin:Message",
  "properties": {
    "message_type": "relationship_response",
    "subject": "Response to guardian request",
    "body": {
      "accepted": true,
      "relation_type": "GUARDIAN_OF",
      "original_request_id": "msg-123"
    },
    "recipient_id": "user:alice-smith",
    "sender_id": "user:bob-jones",
    "status": "pending",
    "related_entity_id": "msg-123"
  }
}
```

**Response**: `201 Created`

The trigger system will automatically create the graph relation `(alice)-[:GUARDIAN_OF]->(bob)` when the message is processed.

---

### Get User's Inbox

Retrieve all messages in a user's inbox.

**Endpoint**: `GET /nodes?path=/users/{username}/inbox/*`

**Response**: `200 OK`

```json
{
  "nodes": [
    {
      "id": "660e8400-e29b-41d4-a716-446655440001",
      "path": "/users/bob/inbox/msg-456",
      "node_type": "raisin:Message",
      "properties": {
        "message_type": "relationship_request",
        "subject": "Request to be your guardian",
        "status": "delivered"
      }
    }
  ],
  "total": 1
}
```

---

### Create RelationType

Define a new relationship type.

**Endpoint**: `POST /nodes`

**Request Body**:

```json
{
  "path": "/relation-types/mentor-of",
  "node_type": "raisin:RelationType",
  "properties": {
    "relation_name": "MENTOR_OF",
    "title": "Mentor Of",
    "description": "Professional mentorship relationship",
    "category": "professional",
    "inverse_relation_name": "MENTEE_OF",
    "bidirectional": false,
    "implies_stewardship": false,
    "icon": "academic-cap",
    "color": "#3b82f6"
  }
}
```

**Response**: `201 Created`

---

### Update StewardshipConfig

Modify repository-wide stewardship settings.

**Endpoint**: `PATCH /nodes?path=/config/stewardship`

**Request Body**:

```json
{
  "properties": {
    "enabled": true,
    "max_stewards_per_ward": 3,
    "invitation_expiry_days": 14
  }
}
```

**Response**: `200 OK`

---

### Create StewardshipOverride

Establish a time-limited stewardship override.

**Endpoint**: `POST /nodes`

**Request Body**:

```json
{
  "path": "/overrides/override-{uuid}",
  "node_type": "raisin:StewardshipOverride",
  "properties": {
    "steward_id": "user:jane-smith",
    "ward_id": "user:alice-smith",
    "delegation_mode": "scoped",
    "scoped_permissions": [
      {
        "permission": "read",
        "scope": "/medical-records"
      }
    ],
    "valid_from": "2025-01-01T00:00:00Z",
    "valid_until": "2025-12-31T23:59:59Z",
    "status": "active",
    "reason": "Temporary guardianship during parent's deployment"
  }
}
```

**Response**: `201 Created`

---

## Function Invocation API

### Invoke Function

Call a QuickJS function.

**Endpoint**: `POST /functions/invoke`

**Request Body**:

```json
{
  "function_path": "/functions/lib/stewardship/get-stewards",
  "input": {
    "ward_user_path": "/users/internal/alice",
    "workspace": "raisin:access_control"
  }
}
```

**Response**: `200 OK`

```json
{
  "success": true,
  "stewards": [
    {
      "user_id": "550e8400-e29b-41d4-a716-446655440000",
      "user_path": "/users/internal/john-smith",
      "email": "john@example.com",
      "display_name": "John Smith",
      "relation_type": "GUARDIAN_OF",
      "relation_title": "Guardian Of"
    }
  ]
}
```

### Available Functions

| Function Path | Description | Input | Output |
|---------------|-------------|-------|--------|
| `/functions/lib/stewardship/get-stewards` | Get all stewards for a ward | `{ ward_user_path }` | `{ success, stewards[], error? }` |
| `/functions/lib/stewardship/get-wards` | Get all wards for a steward | `{ steward_user_path }` | `{ success, wards[], error? }` |
| `/functions/lib/stewardship/is-steward-of` | Check stewardship relationship | `{ steward_user_path, ward_user_path }` | `{ success, is_steward, relation_type?, error? }` |

---

## SQL Queries

### Query Stewardship Relationships via NEIGHBORS

Get all stewards for a user using the NEIGHBORS function:

```sql
-- Get all users who have stewardship relations to ward
SELECT
    e.src_id as steward_id,
    e.edge_label as relation_type,
    u.properties->>'display_name' as steward_name
FROM NEIGHBORS($1, 'IN', NULL) AS e
JOIN nodes u ON u.id = e.src_id
WHERE e.edge_label IN ('PARENT_OF', 'GUARDIAN_OF')
  AND u.node_type = 'raisin:User';

-- Bind parameter: $1 = ward_user_id
```

Get all wards for a steward:

```sql
-- Get all users for whom steward has stewardship relations
SELECT
    e.dst_id as ward_id,
    e.edge_label as relation_type,
    u.properties->>'display_name' as ward_name,
    u.properties->>'birth_date' as birth_date
FROM NEIGHBORS($1, 'OUT', NULL) AS e
JOIN nodes u ON u.id = e.dst_id
WHERE e.edge_label IN ('PARENT_OF', 'GUARDIAN_OF')
  AND u.node_type = 'raisin:User';

-- Bind parameter: $1 = steward_user_id
```

---

### Query RelationTypes

Get all relation types that imply stewardship:

```sql
SELECT
    properties->>'relation_name' as relation_name,
    properties->>'title' as title,
    properties->>'implies_stewardship' as implies_stewardship,
    properties->>'requires_minor' as requires_minor
FROM nodes
WHERE node_type = 'raisin:RelationType'
  AND workspace = 'raisin:access_control'
  AND properties->>'implies_stewardship' = 'true';
```

---

### Query Active Stewardship Overrides

Find active overrides for a ward:

```sql
SELECT *
FROM nodes
WHERE node_type = 'raisin:StewardshipOverride'
  AND workspace = 'raisin:access_control'
  AND properties->>'ward_id' = $1
  AND properties->>'status' = 'active'
  AND (
    properties->>'valid_until' IS NULL
    OR (properties->>'valid_until')::timestamp > NOW()
  );

-- Bind parameter: $1 = 'user:alice'
```

---

### Query Messages

Get pending inbox messages for a user:

```sql
SELECT
    id,
    path,
    properties->>'message_type' as message_type,
    properties->>'subject' as subject,
    properties->>'sender_id' as sender_id,
    properties->>'status' as status,
    created_at
FROM nodes
WHERE node_type = 'raisin:Message'
  AND workspace = 'raisin:access_control'
  AND path LIKE '/users/%/inbox/%'
  AND properties->>'recipient_id' = $1
  AND properties->>'status' = 'pending'
ORDER BY created_at DESC;

-- Bind parameter: $1 = 'user:bob'
```

---

## Graph Operations

### Create Graph Relation

Establish a stewardship relationship directly via graph edge.

**Endpoint**: `POST /graph/relations`

**Request Body**:

```json
{
  "source_node_id": "550e8400-e29b-41d4-a716-446655440000",
  "target_node_id": "660e8400-e29b-41d4-a716-446655440001",
  "relation_type": "GUARDIAN_OF"
}
```

**Response**: `201 Created`

```json
{
  "success": true,
  "relation": {
    "source_id": "550e8400-e29b-41d4-a716-446655440000",
    "target_id": "660e8400-e29b-41d4-a716-446655440001",
    "edge_label": "GUARDIAN_OF"
  }
}
```

---

### Delete Graph Relation

Remove a stewardship relationship.

**Endpoint**: `DELETE /graph/relations`

**Request Body**:

```json
{
  "source_node_id": "550e8400-e29b-41d4-a716-446655440000",
  "target_node_id": "660e8400-e29b-41d4-a716-446655440001",
  "relation_type": "GUARDIAN_OF"
}
```

**Response**: `200 OK`

---

## RLS Integration

### Stewardship Context in AuthContext

When a user acts as a steward, the `AuthContext` includes:

```rust
pub struct AuthContext {
    pub user_id: Option<String>,           // "user:alice"
    pub local_user_id: Option<String>,     // UUID of alice's raisin:User node
    pub acting_as_ward: Option<String>,    // "user:bob" (if acting as steward)
    pub active_stewardship_source: Option<String>, // "GUARDIAN_OF"
    // ... other fields
}
```

### Using Stewardship Context in RLS Conditions

RLS permission conditions can check stewardship context:

```yaml
permissions:
  - resource: "users/**/documents/**"
    action: "read"
    condition: |
      // User can read their own documents OR documents of their ward
      node.properties.owner_id == auth.local_user_id ||
      node.properties.owner_id == auth.acting_as_ward
```

### Example: Ward-Aware Permission

Allow stewards to read their wards' medical records:

```yaml
permissions:
  - resource: "users/**/medical-records/**"
    action: "read"
    condition: |
      // Only if acting as steward via GUARDIAN_OF relation
      auth.acting_as_ward != null &&
      auth.active_stewardship_source == 'GUARDIAN_OF' &&
      node.path.descendantOf('/users/' + auth.acting_as_ward + '/medical-records')
```

### Available Auth Variables in REL Conditions

| Variable | Type | Description | Example |
|----------|------|-------------|---------|
| `auth.user_id` | String | Global user identity | `"user:alice"` |
| `auth.local_user_id` | String | Workspace-specific user node ID | `"550e8400-..."` |
| `auth.email` | String | User's email | `"alice@example.com"` |
| `auth.roles` | Array[String] | User's role IDs | `["editor", "admin"]` |
| `auth.groups` | Array[String] | User's group IDs | `["team-a"]` |
| `auth.acting_as_ward` | String \| null | Ward user ID if acting as steward | `"user:bob"` |
| `auth.active_stewardship_source` | String \| null | Relation type or override ID | `"GUARDIAN_OF"` |

See [REL Documentation](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/REL.md) for full expression syntax.

---

## WebSocket Events

### Subscribe to Message Events

Real-time updates for inbox messages.

**Subscribe Message**:

```json
{
  "type": "subscribe",
  "workspace": "raisin:access_control",
  "path_pattern": "/users/alice/inbox/**",
  "event_types": ["node_created", "node_updated"]
}
```

**Event Notification**:

```json
{
  "type": "event",
  "event_type": "node_created",
  "workspace": "raisin:access_control",
  "node": {
    "id": "770e8400-e29b-41d4-a716-446655440002",
    "path": "/users/alice/inbox/msg-789",
    "node_type": "raisin:Message",
    "properties": {
      "message_type": "relationship_request",
      "subject": "New guardian request",
      "status": "delivered"
    }
  }
}
```

---

## Error Responses

### Standard Error Format

```json
{
  "error": {
    "code": "permission_denied",
    "message": "You do not have permission to access this resource",
    "details": {
      "required_permission": "read",
      "resource_path": "/users/bob/medical-records"
    }
  }
}
```

### Common Error Codes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `permission_denied` | 403 | User lacks required permission |
| `not_found` | 404 | Resource does not exist |
| `invalid_input` | 400 | Request validation failed |
| `conflict` | 409 | Resource already exists or constraint violation |
| `stewardship_limit_exceeded` | 400 | Max stewards/wards limit reached |
| `invalid_relation_type` | 400 | Relation type not recognized |
| `ward_consent_required` | 403 | Ward has not consented to stewardship |

---

## Rate Limits

API requests are subject to rate limiting:

- **Standard tier**: 1000 requests/hour per user
- **Stewardship operations**: 100 relationship changes/hour per user
- **Function invocations**: 500 calls/hour per user

Rate limit headers:

```
X-RateLimit-Limit: 1000
X-RateLimit-Remaining: 742
X-RateLimit-Reset: 1703001600
```

---

## API Versioning

Current API version: `v1`

Version is specified in the URL path:

```
/api/v1/repositories/{repo}/...
```

Breaking changes will result in a new version (`v2`, etc.). Non-breaking changes are added to the current version.

---

## Complete Examples

### Example 1: Establish Guardian Relationship

```bash
# Step 1: User Alice creates relationship request
POST /api/v1/repositories/myrepo/workspaces/raisin:access_control/nodes
Authorization: Bearer {alice-token}
Content-Type: application/json

{
  "path": "/users/alice/outbox/msg-001",
  "node_type": "raisin:Message",
  "properties": {
    "message_type": "relationship_request",
    "subject": "Guardian Request",
    "body": {
      "relation_type": "GUARDIAN_OF",
      "requestor_name": "Alice Smith",
      "message": "I'd like to be your guardian"
    },
    "recipient_id": "user:bob",
    "sender_id": "user:alice",
    "status": "pending"
  }
}

# Step 2: Router trigger fires, copies to sent, creates inbox message for Bob

# Step 3: User Bob retrieves inbox
GET /api/v1/repositories/myrepo/workspaces/raisin:access_control/nodes?path=/users/bob/inbox/*
Authorization: Bearer {bob-token}

# Step 4: Bob creates response (accept)
POST /api/v1/repositories/myrepo/workspaces/raisin:access_control/nodes
Authorization: Bearer {bob-token}
Content-Type: application/json

{
  "path": "/users/bob/outbox/msg-002",
  "node_type": "raisin:Message",
  "properties": {
    "message_type": "relationship_response",
    "body": {
      "accepted": true,
      "original_request_id": "msg-001"
    },
    "recipient_id": "user:alice",
    "sender_id": "user:bob",
    "status": "pending",
    "related_entity_id": "msg-001"
  }
}

# Step 5: Trigger creates graph relation (alice)-[:GUARDIAN_OF]->(bob)

# Step 6: Verify relationship
POST /api/v1/repositories/myrepo/functions/invoke
Authorization: Bearer {alice-token}
Content-Type: application/json

{
  "function_path": "/functions/lib/stewardship/is-steward-of",
  "input": {
    "steward_user_path": "/users/internal/alice",
    "ward_user_path": "/users/internal/bob"
  }
}

# Response:
# {
#   "success": true,
#   "is_steward": true,
#   "relation_type": "GUARDIAN_OF",
#   "relation_title": "Guardian Of"
# }
```

---

### Example 2: Query Stewardship via SQL

```bash
# Execute SQL query via pgwire or SQL endpoint
POST /api/v1/repositories/myrepo/sql/query
Authorization: Bearer {token}
Content-Type: application/json

{
  "query": "SELECT e.dst_id as ward_id, u.properties->>'display_name' as ward_name FROM NEIGHBORS($1, 'OUT', NULL) AS e JOIN nodes u ON u.id = e.dst_id WHERE e.edge_label = 'GUARDIAN_OF' AND u.node_type = 'raisin:User'",
  "params": ["550e8400-e29b-41d4-a716-446655440000"]
}
```

---

## See Also

- [Node Types](./node-types.md) - Stewardship node type definitions
- [Triggers](./triggers.md) - How triggers process messages
- [Functions](./functions.md) - QuickJS function reference
- [Extending Stewardship](./extending.md) - Adding custom functionality
- [ROW_LEVEL_SECURITY.md](/Users/senol/Projects/maravilla-labs/repos/raisindb/docs/ROW_LEVEL_SECURITY.md) - RLS implementation details
