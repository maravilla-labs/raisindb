---
sidebar_position: 1
---

# Row-Level Security (RLS)

RaisinDB provides comprehensive Row-Level Security (RLS) to control which data users can see and modify. Security is enforced at the storage layer, ensuring consistent protection across all access methods (REST API, SQL, WebSocket, pgwire).

## Core Concepts

### Security Model

RaisinDB uses a role-based access control (RBAC) model with these entities:

| Entity | Description |
|--------|-------------|
| **User** | An end-user of your application (stored as `raisin:User` node) |
| **Group** | A collection of users with shared roles (stored as `raisin:Group` node) |
| **Role** | A named set of permissions (stored as `raisin:Role` node) |
| **Permission** | A rule defining access to specific paths/operations |

### Permission Resolution Chain

```
User ─┬─ Direct Roles ──────────────────┐
      │                                  │
      └─ Groups ─► Group Roles ──────────┼──► Effective Roles ──► Permissions
                                         │
                   Role Inheritance ─────┘
```

1. User's direct roles are collected
2. User's groups are resolved, and each group's roles are added
3. For each role, inherited roles are recursively resolved (with cycle detection)
4. All roles are deduplicated
5. Permissions from all effective roles are merged

## Permission Structure

A permission defines what operations a user can perform on which content:

```yaml
path: "content.articles.**"        # Path pattern (required)
operations: [read, update]         # Allowed operations (required)
node_types: ["my:Article"]         # Optional: restrict to specific types
fields: ["title", "body"]          # Optional: whitelist specific fields
except_fields: ["internal_notes"]  # Optional: hide specific fields
conditions:                        # Optional: row-level conditions
  - property_equals:
      key: "author"
      value: "$auth.user_id"
```

### Path Patterns

Permissions use dot-notation path patterns:

| Pattern | Matches |
|---------|---------|
| `content.articles.**` | `/content/articles/` and all descendants |
| `content.articles.*` | Direct children of `/content/articles/` only |
| `departments.*.reports` | `/departments/{any}/reports` |
| `**` | Everything (use with caution) |

### Operations

| Operation | Description |
|-----------|-------------|
| `create` | Create new nodes |
| `read` | View nodes and their properties |
| `update` | Modify existing nodes |
| `delete` | Remove nodes |

### Field-Level Security

Control which properties users can see:

```yaml
# Whitelist: Only show these fields
fields: ["title", "description", "price"]

# Blacklist: Show everything except these fields
except_fields: ["cost_price", "internal_notes", "admin_comments"]
```

## Conditions

Conditions enable dynamic, row-level filtering based on user context or node properties.

### Property Conditions

```yaml
# User can only see their own articles
conditions:
  - property_equals:
      key: "author"
      value: "$auth.user_id"

# User can only see published content
conditions:
  - property_equals:
      key: "status"
      value: "published"

# User can see content in their department
conditions:
  - property_in:
      key: "department"
      values: ["$auth.department", "shared"]
```

### Auth Variables

| Variable | Description |
|----------|-------------|
| `$auth.user_id` | Current user's ID |
| `$auth.email` | Current user's email |

### Comparison Operators

| Condition Type | Description |
|----------------|-------------|
| `property_equals` | Exact match |
| `property_in` | Match any value in list |
| `property_greater_than` | Numeric/string comparison |
| `property_less_than` | Numeric/string comparison |

### Logical Operators

Combine conditions with `all` (AND) or `any` (OR):

```yaml
# Must be owner AND published
conditions:
  - all:
    - property_equals:
        key: "author"
        value: "$auth.user_id"
    - property_equals:
        key: "status"
        value: "published"

# Can see if public OR owner
conditions:
  - any:
    - property_equals:
        key: "visibility"
        value: "public"
    - property_equals:
        key: "author"
        value: "$auth.user_id"
```

### Role & Group Conditions

```yaml
# Only users with 'premium' role can access
conditions:
  - user_has_role: "premium"

# Only users in 'editors' group can access
conditions:
  - user_in_group: "editors"
```

## Setting Up Access Control

### 1. Create Roles

Roles are stored in the `raisin:access_control` workspace:

```json
{
  "node_type": "raisin:Role",
  "name": "content_editor",
  "path": "/roles/content_editor",
  "properties": {
    "role_id": "content_editor",
    "name": "Content Editor",
    "inherits": ["content_viewer"],
    "permissions": [
      {
        "path": "content.**",
        "operations": ["read", "create", "update"],
        "node_types": ["my:Article", "my:Page"]
      }
    ]
  }
}
```

### 2. Create Groups

```json
{
  "node_type": "raisin:Group",
  "name": "marketing_team",
  "path": "/groups/marketing_team",
  "properties": {
    "name": "marketing_team",
    "roles": ["content_editor", "asset_uploader"]
  }
}
```

### 3. Create Users

```json
{
  "node_type": "raisin:User",
  "name": "alice",
  "path": "/users/alice",
  "properties": {
    "email": "alice@example.com",
    "roles": ["content_viewer"],
    "groups": ["marketing_team"]
  }
}
```

## Built-in Roles

| Role | Description |
|------|-------------|
| `system_admin` | Bypasses all RLS checks (full access) |
| `anonymous` | Permissions for unauthenticated users |

## System Context

Some operations bypass RLS entirely:

- **System migrations** - Bootstrap operations use `is_system: true`
- **Admin users with `system_admin` role** - Full access to all data
- **Internal system operations** - Background jobs, indexing

## Admin Impersonation

Admin users with the `can_impersonate` flag can test permissions by viewing content as another user:

```http
GET /api/nodes/content/articles
Authorization: Bearer <admin_token>
X-Raisin-Impersonate: user_123
```

This is useful for:
- Testing permission configurations
- Debugging "why can't user X see this?"
- QA before rolling out permission changes

## Performance

RLS is optimized for production workloads:

- **Permission caching** - Resolved permissions are cached per user (5-minute TTL)
- **Path matching** - Uses prefix-based matching for efficient filtering
- **Batch operations** - Permissions are checked once per request, not per node

## Security Guarantees

RaisinDB's RLS provides:

1. **Single enforcement point** - All data access goes through the storage layer
2. **No bypass via SQL** - SQL queries are filtered at execution time
3. **Consistent across interfaces** - REST, WebSocket, SQL, pgwire all enforce the same rules
4. **Audit trail** - All operations are logged with user context

## Next Steps

- [Permission Examples](./examples.md) - Common permission patterns
- [Condition Reference](./conditions.md) - Complete condition syntax
- [Admin Console](../rest/overview.md) - Manage permissions via UI
