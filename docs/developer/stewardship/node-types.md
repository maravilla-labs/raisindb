# Stewardship Node Types

This document describes all node types used in the RaisinDB Stewardship System.

## Overview

The stewardship system defines five core node types:

1. **raisin:StewardshipConfig** - Repository-wide stewardship settings
2. **raisin:RelationType** - Discoverable relationship type definitions
3. **raisin:EntityCircle** - Grouping containers for users
4. **raisin:StewardshipOverride** - Explicit time-limited delegations
5. **raisin:Message** - Inbox/outbox communication for relationship workflows

All node types are defined in `/Users/senol/Projects/maravilla-labs/repos/raisindb/builtin-packages/raisin-stewardship/nodetypes/`.

---

## raisin:StewardshipConfig

Repository-level configuration that controls stewardship behavior.

### Location

Typically stored at `/config/stewardship` in the `raisin:access_control` workspace.

### Properties

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `enabled` | Boolean | `false` | Whether stewardship is enabled for this repository |
| `stewardship_relation_types` | Array[String] | `["PARENT_OF", "GUARDIAN_OF"]` | Which relation types imply stewardship |
| `require_minor_for_parent` | Boolean | `true` | PARENT_OF only grants stewardship if target is minor |
| `allowed_workflows` | Array[String] | `["invitation", "admin_assignment", "steward_creates_ward"]` | Allowed workflows for establishing stewardship |
| `steward_creates_ward_enabled` | Boolean | `false` | Whether stewards can create new ward accounts |
| `max_stewards_per_ward` | Integer | `5` | Maximum number of stewards allowed per ward |
| `max_wards_per_steward` | Integer | `10` | Maximum number of wards allowed per steward |
| `invitation_expiry_days` | Integer | `7` | Days until a stewardship invitation expires |
| `require_ward_consent` | Boolean | `true` | Whether ward consent is required for stewardship |
| `minor_age_threshold` | Integer | `18` | Age below which a user is considered a minor |
| `allow_minor_login` | Boolean | `false` | Whether minors can log in directly |

### Metadata

- **Versionable**: Yes
- **Publishable**: No
- **Auditable**: Yes
- **Indexable**: No

### Example YAML

```yaml
node_type: raisin:StewardshipConfig
properties:
  enabled: true
  stewardship_relation_types:
    - "PARENT_OF"
    - "GUARDIAN_OF"
  require_minor_for_parent: true
  allowed_workflows:
    - "invitation"
    - "admin_assignment"
    - "steward_creates_ward"
  steward_creates_ward_enabled: false
  max_stewards_per_ward: 5
  max_wards_per_steward: 10
  invitation_expiry_days: 7
  require_ward_consent: true
  minor_age_threshold: 18
  allow_minor_login: false
```

### Usage

The stewardship functions (`get-stewards`, `get-wards`, `is-steward-of`) read this configuration to determine:
- Which relation types grant stewardship privileges
- Age calculation for minor status
- Workflow restrictions

---

## raisin:RelationType

Discoverable relationship type definition that maps to graph edge labels.

### Location

Typically stored under `/relation-types/` in the `raisin:access_control` workspace.

### Properties

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `relation_name` | String | Yes | Graph relation type in UPPER_SNAKE_CASE (e.g., "PARENT_OF") |
| `title` | String | Yes | Display name for UI (e.g., "Parent Of") |
| `description` | String | No | Human-readable description of the relationship |
| `category` | String | No | Category for grouping (e.g., "household", "organization", "social") |
| `inverse_relation_name` | String | No | Inverse relation type (e.g., "CHILD_OF" for "PARENT_OF") |
| `bidirectional` | Boolean | No (default: `false`) | If true, relation exists both ways automatically |
| `implies_stewardship` | Boolean | No (default: `false`) | Whether this relation type grants stewardship |
| `requires_minor` | Boolean | No (default: `false`) | Only implies stewardship if target is a minor |
| `icon` | String | No | Icon for UI display (e.g., "users", "heart") |
| `color` | String | No | Color for UI display (e.g., "#10b981") |

### Metadata

- **Versionable**: Yes
- **Publishable**: No
- **Auditable**: Yes
- **Indexable**: Yes (Property, Fulltext)

### Example YAML

```yaml
node_type: raisin:RelationType
properties:
  relation_name: "GUARDIAN_OF"
  title: "Guardian Of"
  description: "Legal guardian relationship"
  category: "legal"
  inverse_relation_name: "WARD_OF"
  bidirectional: false
  implies_stewardship: true
  requires_minor: false
  icon: "shield"
  color: "#8b5cf6"
```

### Relationship to Graph Relations

The `relation_name` property corresponds directly to edge labels in the graph database. When a graph relation is created (e.g., `CREATE (user1)-[:PARENT_OF]->(user2)`), the system can look up the RelationType node to:

- Display the human-friendly `title` in UIs
- Determine if it grants stewardship via `implies_stewardship`
- Apply age-based filtering via `requires_minor`

---

## raisin:EntityCircle

Generic grouping container for users (families, teams, organizational units).

### Location

Can be stored anywhere in the workspace, typically under `/circles/` or `/families/`.

### Properties

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | String | Yes | Circle name (e.g., "Smith Family", "Engineering Team") |
| `circle_type` | String | No | Type of circle (e.g., "family", "team", "org_unit") |
| `primary_contact_id` | String | No | User ID of the primary contact for this circle |
| `address` | Object | No | Shared address information for the circle |
| `metadata` | Object | No | Extensible properties for additional data |

### Metadata

- **Versionable**: Yes
- **Publishable**: No
- **Auditable**: Yes
- **Indexable**: Yes (Property, Fulltext)

### Example YAML

```yaml
node_type: raisin:EntityCircle
properties:
  name: "Smith Family"
  circle_type: "family"
  primary_contact_id: "user:john-smith"
  address:
    street: "123 Main St"
    city: "Springfield"
    state: "IL"
    zip: "62701"
  metadata:
    created_date: "2025-01-15"
    family_size: 4
```

### Member Relationships

Users are linked to EntityCircles via graph relations. Common patterns:

```cypher
// Link user to family
(user)-[:MEMBER_OF]->(circle)

// Link steward to family
(user)-[:STEWARD_OF]->(circle)

// Link primary contact
(user)-[:PRIMARY_CONTACT_OF]->(circle)
```

---

## raisin:StewardshipOverride

Explicit stewardship override for time-limited or scoped delegations.

### Location

Can be stored anywhere, typically under `/overrides/` or attached to user nodes.

### Properties

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `steward_id` | String | Yes | User ID of the steward |
| `ward_id` | String | Yes | User ID of the ward |
| `delegation_mode` | String | Yes | "full" or "scoped" delegation |
| `scoped_permissions` | Array[Object] | No | Specific permissions granted (for scoped delegation) |
| `valid_from` | Date | No | Start date for the stewardship |
| `valid_until` | Date | No | End date for the stewardship (null = indefinite) |
| `status` | String | Yes (default: "pending") | Status - "pending", "active", "expired", "revoked" |
| `reason` | String | No | Reason for the stewardship override |

### Metadata

- **Versionable**: Yes
- **Publishable**: No
- **Auditable**: Yes
- **Indexable**: Yes (Property)

### Example YAML

```yaml
node_type: raisin:StewardshipOverride
properties:
  steward_id: "user:jane-smith"
  ward_id: "user:alice-smith"
  delegation_mode: "scoped"
  scoped_permissions:
    - permission: "read"
      scope: "/medical-records"
    - permission: "update"
      scope: "/education"
  valid_from: "2025-01-01T00:00:00Z"
  valid_until: "2025-12-31T23:59:59Z"
  status: "active"
  reason: "Temporary guardianship during parent's deployment"
```

### Use Cases

1. **Temporary Guardianship**: Time-limited delegation when primary steward is unavailable
2. **Scoped Access**: Grant specific permissions without full stewardship
3. **Emergency Access**: Quick delegation in urgent situations
4. **Administrative Overrides**: Admin-assigned stewardship outside normal workflows

---

## raisin:Message

Message node for inbox/outbox communication patterns (relationship requests, notifications, chat).

### Location

- Outbox: `/users/{username}/outbox/{message-id}`
- Inbox: `/users/{username}/inbox/{message-id}`
- Sent: `/users/{username}/sent/{message-id}`

### Properties

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `message_type` | String | Yes | Type of message (see Message Types below) |
| `subject` | String | No | Subject line for display |
| `body` | Object | Yes | Payload specific to message_type |
| `recipient_id` | String | Yes | Target user ID |
| `sender_id` | String | Yes | Source user ID |
| `status` | String | Yes (default: "pending") | "pending", "sent", "delivered", "read", "processed" |
| `related_entity_id` | String | No | Related entity ID (e.g., relationship request ID) |
| `expires_at` | Date | No | Expiration date for the message |
| `metadata` | Object | No | Additional extensible metadata |

### Metadata

- **Versionable**: No
- **Publishable**: No
- **Auditable**: Yes
- **Indexable**: Yes (Property, Fulltext)

### Message Types

The `message_type` property determines how the message is processed:

| Message Type | Description | Handler Trigger |
|--------------|-------------|-----------------|
| `relationship_request` | Request to establish a relationship | `process-relationship-request` |
| `relationship_response` | Response (accept/reject) to relationship request | `process-relationship-response` |
| `ward_invitation` | Invitation to create a ward account | `process-ward-invitation` |
| `system_notification` | System-generated notification | N/A |
| `chat` | Direct user-to-user message | N/A |

### Example: Relationship Request

```yaml
node_type: raisin:Message
properties:
  message_type: "relationship_request"
  subject: "John Smith wants to be your guardian"
  body:
    relation_type: "GUARDIAN_OF"
    requestor_name: "John Smith"
    requestor_email: "john@example.com"
    message: "I would like to be your legal guardian"
  recipient_id: "user:alice-jones"
  sender_id: "user:john-smith"
  status: "pending"
  expires_at: "2025-12-26T00:00:00Z"
```

### Example: Relationship Response

```yaml
node_type: raisin:Message
properties:
  message_type: "relationship_response"
  subject: "Alice accepted your guardian request"
  body:
    accepted: true
    relation_type: "GUARDIAN_OF"
    original_request_id: "msg:12345"
  recipient_id: "user:john-smith"
  sender_id: "user:alice-jones"
  status: "sent"
  related_entity_id: "msg:12345"
```

### Example: Ward Invitation

```yaml
node_type: raisin:Message
properties:
  message_type: "ward_invitation"
  subject: "You are invited to create an account"
  body:
    relation_type: "PARENT_OF"
    steward_name: "Jane Smith"
    steward_email: "jane@example.com"
    ward_email: "child@example.com"
    ward_display_name: "Alex Smith"
    invitation_code: "INV-ABC123"
  recipient_id: "user:pending-ward"
  sender_id: "user:jane-smith"
  status: "pending"
  expires_at: "2025-12-26T00:00:00Z"
```

---

## Working with Node Types

### Creating Nodes via API

```bash
# Create a RelationType
POST /api/v1/repositories/{repo}/workspaces/raisin:access_control/nodes
Content-Type: application/json

{
  "path": "/relation-types/mentor-of",
  "node_type": "raisin:RelationType",
  "properties": {
    "relation_name": "MENTOR_OF",
    "title": "Mentor Of",
    "description": "Professional mentorship relationship",
    "category": "professional",
    "inverse_relation_name": "MENTEE_OF",
    "implies_stewardship": false,
    "icon": "academic-cap",
    "color": "#3b82f6"
  }
}
```

### Querying Nodes via SQL

```sql
-- Find all relation types that imply stewardship
SELECT * FROM nodes
WHERE node_type = 'raisin:RelationType'
  AND properties->>'implies_stewardship' = 'true';

-- Find all active stewardship overrides for a ward
SELECT * FROM nodes
WHERE node_type = 'raisin:StewardshipOverride'
  AND properties->>'ward_id' = 'user:alice'
  AND properties->>'status' = 'active'
  AND (properties->>'valid_until' IS NULL
       OR properties->>'valid_until'::timestamp > NOW());

-- Find pending messages for a user
SELECT * FROM nodes
WHERE node_type = 'raisin:Message'
  AND properties->>'recipient_id' = 'user:alice'
  AND properties->>'status' = 'pending'
ORDER BY created_at DESC;
```

---

## See Also

- [Stewardship Triggers](./triggers.md) - How triggers process messages
- [Stewardship Functions](./functions.md) - QuickJS functions for stewardship queries
- [API Reference](./api-reference.md) - REST API endpoints
- [Extending Stewardship](./extending.md) - Adding custom types and workflows
