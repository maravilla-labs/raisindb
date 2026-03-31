# Access Control Practical Guide

This guide walks through common access control tasks in RaisinDB -- creating users, defining roles, configuring security policies, and testing permissions. It is a hands-on companion to the [Access Control & Authorization](../architecture/access-control.md) architecture chapter, which covers the underlying design in detail.

All access control entities (users, roles, groups) are stored as nodes in the built-in `raisin:access_control` workspace. You manage them via the REST API or the Admin Console, just like any other content in RaisinDB.

## Managing Users

A `raisin:User` node represents a user's presence within the system. Each user lives under `/users` in the `raisin:access_control` workspace and carries role assignments, group memberships, and profile data.

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `email` | String | Yes | User's email (unique) |
| `display_name` | String | Yes | Display name |
| `roles` | Array | No | Direct role assignments |
| `groups` | Array | No | Group memberships |
| `metadata` | Object | No | Custom key-value data |
| `birth_date` | Date | No | For minor status calculation |
| `can_login` | Boolean | No | Whether user can authenticate (default: `true`) |

### Via REST API

Create a user by posting a node to the `raisin:access_control` workspace:

```bash
curl -X POST \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "parent_path": "/users",
    "name": "alice",
    "node_type": "raisin:User",
    "properties": {
      "user_id": "alice",
      "email": "alice@example.com",
      "display_name": "Alice Smith",
      "roles": ["editor", "reviewer"],
      "groups": ["engineering"]
    }
  }'
```

Update an existing user's roles:

```bash
curl -X PUT \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes/users/alice" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "properties": {
      "user_id": "alice",
      "email": "alice@example.com",
      "display_name": "Alice Smith",
      "roles": ["editor", "reviewer", "content-manager"],
      "groups": ["engineering", "content-team"]
    }
  }'
```

List all users:

```bash
curl "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes/users" \
  -H "Authorization: Bearer $TOKEN"
```

### Via Admin Console

The Admin Console provides a **UserEditor** form for managing user accounts:

1. Navigate to the Access Control section and select **Users**.
2. Click **Create User** to open the editor.
3. Fill in the required fields:
   - **user_id** -- A unique text identifier for the user. This field cannot be changed after creation.
   - **email** -- The user's email address.
   - **display_name** -- The name shown in the UI.
4. Assign **roles** using the tag selector. The selector suggests from all existing roles in the system.
5. Assign **groups** using the tag selector. The selector suggests from all existing groups.
6. Click **Save**. The console creates a `raisin:User` node at `/users/{user_id}` in the `raisin:access_control` workspace.

When editing an existing user, the `user_id` field is disabled to prevent accidental identity changes. All other fields remain editable.

## Managing Roles

A `raisin:Role` node defines a named set of permissions. Roles can inherit from other roles, forming a hierarchy. They live under `/roles` in the `raisin:access_control` workspace.

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | String | Yes | Role name (unique) |
| `description` | String | No | Human-readable description |
| `inherits` | Array | No | Role IDs this role inherits from |
| `permissions` | Array | No | Permission grant objects |

### Via REST API

Create a role:

```bash
curl -X POST \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "parent_path": "/roles",
    "name": "content-editor",
    "node_type": "raisin:Role",
    "properties": {
      "role_id": "content-editor",
      "name": "Content Editor",
      "description": "Can manage articles in the content workspace",
      "inherits": ["viewer"],
      "permissions": [
        {
          "workspace": "content",
          "path": "articles/**",
          "operations": ["create", "read", "update", "delete"],
          "node_types": ["blog:Article", "blog:Draft"]
        }
      ]
    }
  }'
```

Update a role to add new permissions:

```bash
curl -X PUT \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes/roles/content-editor" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "properties": {
      "role_id": "content-editor",
      "name": "Content Editor",
      "description": "Can manage articles and media in the content workspace",
      "inherits": ["viewer"],
      "permissions": [
        {
          "workspace": "content",
          "path": "articles/**",
          "operations": ["create", "read", "update", "delete"],
          "node_types": ["blog:Article", "blog:Draft"]
        },
        {
          "path": "media/**",
          "operations": ["read"],
          "fields": ["title", "url", "thumbnail"]
        }
      ]
    }
  }'
```

### Defining Permissions

Each entry in the `permissions` array is a permission grant. Here is a reference for every field:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `workspace` | String | All workspaces | Workspace pattern (supports globs like `content-*`) |
| `branch_pattern` | String | All branches | Branch pattern (supports globs like `features/*`) |
| `path` | String | -- | Path pattern for content matching (required) |
| `operations` | Array | -- | List of allowed operations (required) |
| `node_types` | Array | All types | Restrict to specific node types |
| `fields` | Array | All fields | Field whitelist -- only these fields are accessible |
| `except_fields` | Array | None | Field blacklist -- all fields except these |
| `condition` | String | None | REL expression that must evaluate to truthy |

**Operations** -- seven operations can be granted:

| Operation | Description |
|-----------|-------------|
| `create` | Create new nodes |
| `read` | View and query nodes |
| `update` | Modify existing nodes |
| `delete` | Remove nodes |
| `translate` | Modify translations on nodes |
| `relate` | Create relationships between nodes |
| `unrelate` | Remove relationships between nodes |

**Path patterns** use glob-style matching:

- `*` matches any characters except `/` (single segment)
- `**` matches any characters including `/` (recursive)
- `?` matches any single character except `/`

Path validation rules (enforced by the Admin Console):
- Must start with `/`
- No triple slashes (`///`)
- Valid characters: letters, numbers, `-`, `_`, `/`, `*`, `?`
- No `***` or longer wildcard sequences

**Example: Read-only access to published articles**

```json
{
  "workspace": "content",
  "path": "articles/published/**",
  "operations": ["read"],
  "node_types": ["blog:Article"]
}
```

**Example: Full access within a team's workspace, excluding sensitive fields**

```json
{
  "workspace": "team-alpha",
  "path": "**",
  "operations": ["create", "read", "update", "delete", "relate", "unrelate"],
  "except_fields": ["internal_notes", "salary"]
}
```

**Example: Conditional access with a REL expression**

```json
{
  "workspace": "content",
  "path": "articles/**",
  "operations": ["update", "delete"],
  "condition": "node.created_by == auth.user_id"
}
```

This permission only applies when the node's creator matches the requesting user, effectively implementing ownership-based access.

### Via Admin Console

The Admin Console provides a **RoleEditor** form and a **PermissionEditor** component for managing roles and their permissions.

**Creating or editing a role:**

1. Navigate to **Roles** under the Access Control section.
2. Click **Create Role** or select an existing role to edit.
3. Fill in the fields:
   - **role_id** -- Unique identifier (disabled when editing an existing role).
   - **name** -- Human-readable role name.
   - **description** -- Optional description (textarea).
4. Under **Inherits**, use the tag selector to pick parent roles. The selector shows all roles in the system, filtered to exclude the current role (preventing self-inheritance).
5. The **Permissions** section shows each permission as a card displaying the path, operations, workspace badge, branch badge, node type count, and condition preview.

**Adding or editing a permission:**

Click **Add Permission** on a role, or click an existing permission card. A dialog opens with two tabs:

**Details tab:**
- **workspace** -- Workspace scope for this permission (text input).
- **branch_pattern** -- Branch scope (text input).
- **path** -- Content path pattern with glob validation.
- **operations** -- Seven checkboxes: create, read, update, delete, translate, relate, unrelate.
- **node_types** -- Tag selector populated from the system's registered node types. Does not allow custom values.
- **fields** -- Whitelist of allowed fields. Tag selector with suggestions loaded from the selected node type's property definitions.
- **except_fields** -- Blacklist. Same tag selector as fields.

**Conditions tab:**
- A ConditionBuilder component for writing REL expressions.
- Shows available variables for reference: `resource.*`, `auth.user_id`, `auth.email`, `auth.roles`, `auth.groups`.
- See the [architecture chapter](../architecture/access-control.md) for the full list of `auth.*` and `node.*` variables available in REL conditions.

### Role Inheritance in Practice

Roles can inherit from other roles via the `inherits` property. Permissions are collected from the entire inheritance chain.

A typical hierarchy:

```
system_admin    (full access to everything)
    ^
  admin         (manage users, configure workspaces)
    ^
  editor        (create and edit content)
    ^
  viewer        (read-only access)
```

When you assign a user the `editor` role, they also receive all permissions from `viewer` (and any other roles in `viewer`'s chain). Inheritance is resolved recursively with cycle detection, so you cannot create circular dependencies.

To set up this hierarchy via the API:

```bash
# Create the viewer role
curl -X POST \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "parent_path": "/roles",
    "name": "viewer",
    "node_type": "raisin:Role",
    "properties": {
      "role_id": "viewer",
      "name": "Viewer",
      "permissions": [
        { "path": "**", "operations": ["read"] }
      ]
    }
  }'

# Create the editor role, inheriting from viewer
curl -X POST \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "parent_path": "/roles",
    "name": "editor",
    "node_type": "raisin:Role",
    "properties": {
      "role_id": "editor",
      "name": "Editor",
      "inherits": ["viewer"],
      "permissions": [
        { "path": "articles/**", "operations": ["create", "update", "delete"] }
      ]
    }
  }'
```

The `editor` role now grants `create`, `update`, `delete` on articles, plus the `read` on everything inherited from `viewer`.

## Managing Groups

Groups are a layer of indirection between users and roles. Instead of assigning roles to individual users, you assign roles to a group and then add users to that group. When group roles change, all members are affected.

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | String | Yes | Group name (unique) |
| `description` | String | No | Human-readable description |
| `roles` | Array | No | Roles assigned to all group members |

### Via REST API

Create a group:

```bash
curl -X POST \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "parent_path": "/groups",
    "name": "engineering",
    "node_type": "raisin:Group",
    "properties": {
      "group_id": "engineering",
      "name": "Engineering Team",
      "description": "All engineering staff",
      "roles": ["editor", "developer"]
    }
  }'
```

Then assign a user to the group by including the group in their `groups` array:

```bash
curl -X PUT \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes/users/alice" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "properties": {
      "user_id": "alice",
      "email": "alice@example.com",
      "display_name": "Alice Smith",
      "groups": ["engineering"]
    }
  }'
```

### Via Admin Console

The **GroupEditor** form works similarly to the other editors:

1. Navigate to **Groups** under Access Control.
2. Click **Create Group** or select an existing group.
3. Fill in:
   - **group_id** -- Unique identifier (disabled when editing).
   - **name** -- Human-readable group name.
   - **description** -- Optional description (textarea).
4. Under **Roles**, use the tag selector to assign roles. The selector suggests from all existing roles.
5. Click **Save**. The console creates a `raisin:Group` node at `/groups/{group_id}` in `raisin:access_control`.

## Security Configuration

The `raisin:SecurityConfig` node at `/config/default` in the `raisin:access_control` workspace controls global security behavior.

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `workspace` | String | `*` | Workspace scope (required, unique) |
| `default_policy` | String | `deny` | Default policy when no permission matches (`deny`) |
| `anonymous_enabled` | Boolean | `false` | Whether unauthenticated access is allowed |
| `anonymous_role` | String | -- | Role assigned to anonymous requests |
| `interfaces` | Object | -- | Per-interface overrides |

### Configuring Default Policy

The initial security configuration ships with a deny-all default:

```bash
curl -X PUT \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes/config/default" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "properties": {
      "workspace": "*",
      "default_policy": "deny",
      "anonymous_enabled": false
    }
  }'
```

With `default_policy` set to `deny`, any request that does not match an explicit permission grant is rejected. This is the recommended setting for production.

### Enabling Anonymous Access

To allow unauthenticated users to access certain content, enable anonymous access and assign a role:

```bash
curl -X PUT \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes/config/default" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "properties": {
      "workspace": "*",
      "default_policy": "deny",
      "anonymous_enabled": true,
      "anonymous_role": "anonymous"
    }
  }'
```

The built-in `anonymous` role grants read-only access to the `launchpad` workspace:

```json
{
  "role_id": "anonymous",
  "name": "Anonymous Access",
  "description": "Role for unauthenticated users with read-only access to public workspaces",
  "permissions": [
    { "path": "**", "operations": ["read"], "workspace": "launchpad" }
  ]
}
```

You can customize the `anonymous` role's permissions to grant access to other workspaces or paths as needed.

### Per-Interface Overrides

The `interfaces` property allows different security settings for different transport layers (HTTP, WebSocket, PGWire). For example, you might want stricter anonymous settings on the PGWire interface while allowing anonymous reads over HTTP.

## Testing Permissions with Impersonation

Before deploying permission changes, you should verify they work as expected. RaisinDB supports user impersonation for this purpose.

### Via Admin Console

The Admin Console includes an **ImpersonationSelector** component in the toolbar. It is only visible to administrators whose account has the `can_impersonate` access flag set.

**To impersonate a user:**

1. Click the **"View as..."** button in the toolbar.
2. A dropdown appears with two modes:
   - **Search mode** -- Type a name, email, or user ID. The search queries the `raisin:access_control` workspace for matching `raisin:User` nodes by `display_name`, `email`, `user_id`, or `name`. Search is debounced at 350ms.
   - **Tree browse mode** -- Browse the root nodes of the `raisin:access_control` workspace to find users.
3. Select a user. The toolbar updates to show the impersonated user's name.
4. Navigate the system. All content is now filtered through the impersonated user's permissions.
5. Click **"Exit impersonation"** to return to your own view.

A warning footer displays while impersonation is active: "You are viewing content as another user. Actions are still audited under your account." Impersonation uses the user node's ID (UUID), not the `user_id` property.

### Via REST API

Use the `X-Raisin-Impersonate` header to test permissions programmatically:

```bash
# View content as alice would see it
curl "https://your-server/api/v1/repositories/my-repo/workspaces/content/nodes/articles" \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "X-Raisin-Impersonate: alice-node-uuid"
```

The impersonating user must have the `can_impersonate` access flag. All actions taken while impersonating are audited under the original administrator's account.

## Common Patterns

### Ownership-Based Access

Allow users to edit only content they created:

```json
{
  "path": "articles/**",
  "operations": ["update", "delete"],
  "condition": "node.created_by == auth.user_id"
}
```

This REL condition compares the node's `created_by` field (set automatically on creation) against the authenticated user's identity.

### Team-Based Content Spaces

Give each team a dedicated workspace with full control:

```json
{
  "workspace": "team-alpha",
  "path": "**",
  "operations": ["create", "read", "update", "delete", "relate", "unrelate"]
}
```

Assign this permission to a `team-alpha-member` role, then create a `team-alpha` group with that role. Adding or removing users from the group controls access to the team's workspace.

### Public/Private Content

Combine the built-in `anonymous` role (read access to a public workspace) with authenticated roles for private content:

```json
[
  {
    "workspace": "public-site",
    "path": "**",
    "operations": ["read"]
  }
]
```

Assign this to the `anonymous` role so unauthenticated visitors can read public content. Private workspaces remain inaccessible because the default policy is `deny`.

### Home Directory Pattern

The built-in `authenticated_user` role demonstrates a home directory pattern where each user has access to their own subtree. The relevant permissions use `auth.home` to scope access:

```json
{
  "path": "users/**/inbox/**",
  "operations": ["read", "update", "delete"],
  "workspace": "raisin:access_control",
  "condition": "node.path.startsWith(auth.home)"
}
```

The `auth.home` variable resolves to the user's home path in the repository (e.g., `/users/alice`). This means Alice can manage her own inbox but cannot access Bob's.

The full `authenticated_user` role also covers `outbox`, `sent`, and `notifications` folders with the same pattern:

```json
{
  "path": "users/**/outbox/**",
  "operations": ["create", "read", "update", "delete"],
  "workspace": "raisin:access_control",
  "condition": "node.path.startsWith(auth.home)"
}
```

### Social Graph Permissions

The `authenticated_user` role shows how graph relationships can drive access control. Profile visibility is determined by social connections:

**Own profile -- full access:**

```json
{
  "path": "users/**/profile",
  "operations": ["read", "update"],
  "workspace": "raisin:access_control",
  "condition": "node.path.startsWith(auth.home)"
}
```

**Friends' profiles -- read access:**

```json
{
  "path": "users/**/profile",
  "operations": ["read"],
  "workspace": "raisin:access_control",
  "condition": "node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'"
}
```

**Friends-of-friends -- limited fields only:**

```json
{
  "path": "users/**/profile",
  "operations": ["read"],
  "workspace": "raisin:access_control",
  "fields": ["display_name", "avatar", "bio"],
  "condition": "node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 2"
}
```

This creates a graduated visibility model: you see your own full profile, your direct friends see everything, and friends-of-friends see only `display_name`, `avatar`, and `bio`. Everyone else sees only the `display_name` (granted by a separate blanket permission on `raisin:User` nodes).

## Stewardship Setup

The stewardship system manages parent/guardian relationships over minor or dependent user accounts. It is provided by the built-in `raisin-stewardship` package (which depends on `raisin-relationships`).

### Enabling Stewardship

Stewardship is configured via the `raisin:StewardshipConfig` node. The default configuration has stewardship disabled:

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `enabled` | Boolean | `false` | Master switch for stewardship features |
| `stewardship_relation_types` | Array | `["PARENT_OF", "GUARDIAN_OF"]` | Relation types that grant stewardship |
| `require_minor_for_parent` | Boolean | `true` | PARENT_OF requires ward to be a minor |
| `max_stewards_per_ward` | Number | `5` | Maximum stewards per dependent |
| `max_wards_per_steward` | Number | `10` | Maximum dependents per steward |
| `invitation_expiry_days` | Number | `7` | Days before a stewardship invitation expires |
| `require_ward_consent` | Boolean | `true` | Whether the ward must accept |
| `minor_age_threshold` | Number | `18` | Age below which a user is considered a minor |
| `allow_minor_login` | Boolean | `false` | Whether minors can log in directly |

To enable stewardship:

```bash
curl -X PUT \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes/config/stewardship" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "properties": {
      "enabled": true,
      "stewardship_relation_types": ["PARENT_OF", "GUARDIAN_OF"],
      "minor_age_threshold": 18,
      "allow_minor_login": false
    }
  }'
```

### Creating Entity Circles

Entity circles represent organizational groupings like families, teams, or departments. They are `raisin:EntityCircle` nodes.

| Property | Type | Description |
|----------|------|-------------|
| `name` | String | Circle name (required) |
| `circle_type` | String | One of: `family`, `team`, `org_unit`, `department`, `project`, `custom` |
| `primary_contact_id` | String | User ID of the primary contact |
| `address` | Object | Address with `street`, `city`, `state`, `postal_code`, `country` |
| `metadata` | Object | Custom key-value data |

```bash
curl -X POST \
  "https://your-server/api/v1/repositories/my-repo/workspaces/raisin:access_control/nodes" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $TOKEN" \
  -d '{
    "parent_path": "/circles",
    "name": "smith-family",
    "node_type": "raisin:EntityCircle",
    "properties": {
      "name": "Smith Family",
      "circle_type": "family",
      "primary_contact_id": "alice",
      "address": {
        "city": "Portland",
        "state": "OR",
        "country": "US"
      }
    }
  }'
```

### Establishing Stewardship Relationships

Once stewardship is enabled, create `PARENT_OF` or `GUARDIAN_OF` relationships between user nodes. The stewardship system uses the same relationship infrastructure as the rest of RaisinDB (provided by the `raisin-relationships` dependency).

When a steward is acting on behalf of a ward, the `auth.acting_as_ward` and `auth.active_stewardship_source` variables are populated in REL conditions, enabling permissions that account for delegated access.

## Packages and Access Control

### Builtin Packages

RaisinDB ships with built-in roles that are always available:

**system_admin** -- Full access to everything:

```json
{
  "role_id": "system_admin",
  "name": "System Administrator",
  "description": "Built-in superuser role with full access to all resources",
  "permissions": [
    { "path": "**", "operations": ["create", "read", "update", "delete", "translate", "relate", "unrelate"] }
  ]
}
```

**anonymous** -- Read-only access for unauthenticated users:

```json
{
  "role_id": "anonymous",
  "name": "Anonymous Access",
  "description": "Role for unauthenticated users with read-only access to public workspaces",
  "permissions": [
    { "path": "**", "operations": ["read"], "workspace": "launchpad" }
  ]
}
```

**authenticated_user** -- Default role for all logged-in users, granting access to their own profile, inbox, outbox, sent, and notifications folders. See [Social Graph Permissions](#social-graph-permissions) above for a walkthrough of its permissions.

The `raisin-stewardship` package is also built-in. It depends on `raisin-relationships`, provides the `raisin:StewardshipConfig`, `raisin:EntityCircle`, and `raisin:StewardshipOverride` node types, and installs 12 relation types into the `raisin:access_control` workspace. It also patches the `raisin:access_control` workspace to use `raisin:AclFolder` as the default folder type.

### Custom Auth Content in RAP Packages

You can include custom roles, groups, and security configurations in your own [RAP packages](../guides/packages.md). This is useful for distributing a consistent authorization setup across environments or tenants.

In your package's `content/` directory, place role and group nodes under the `raisin:access_control` workspace:

```
my-auth-package-1.0.0.rap
  manifest.yaml
  content/
    raisin:access_control/
      roles/
        custom-editor/
          node.yaml
        custom-viewer/
          node.yaml
      groups/
        default-team/
          node.yaml
```

Each `node.yaml` defines the node type and properties, just as they would be created via the API. When the package is installed, these nodes are created in the `raisin:access_control` workspace.

## Troubleshooting

**User cannot access content they should have access to:**
1. Check the user's roles and groups. Verify the user node at `/users/{user_id}` has the expected `roles` and `groups` arrays.
2. Inspect role permissions. Look at each role's `permissions` array and verify the `path`, `operations`, `workspace`, and `node_types` fields match the content being accessed.
3. Check inheritance. If the role inherits from other roles, verify those parent roles exist and have the expected permissions.
4. Test with impersonation. Use the Admin Console's impersonation feature to view content as the affected user.

**Permission condition is not matching:**
1. Verify the REL expression syntax. Common mistakes include using `==` with array values (use `CONTAINS` instead) or referencing undefined variables.
2. Check `auth.*` variable values. The `auth.home` variable requires the user to have a home path set. The `auth.local_user_id` is the workspace-specific node ID, not the global identity ID.
3. For graph conditions (`RELATES ... VIA`), verify the relationship exists between the relevant nodes.

**Anonymous access is not working:**
1. Confirm `anonymous_enabled` is `true` in the security config at `/config/default`.
2. Verify `anonymous_role` points to a valid role with appropriate permissions.
3. Check that the `anonymous` role (or whichever role you specified) has permissions for the target workspace and path.

**Field filtering is hiding expected data:**
1. Check whether the matching permission has a `fields` whitelist. If set, only listed fields are returned.
2. Check for `except_fields` blacklists that might exclude the field you need.
3. Remember that whitelist takes precedence over blacklist if both are set on the same permission.

**Role inheritance creates unexpected permissions:**
1. Roles inherit all permissions from their parent chain. Use the Admin Console's role editor to inspect the full `inherits` chain.
2. Check for unintended transitive inheritance -- if role A inherits B which inherits C, role A gets permissions from both B and C.
3. Cycle detection prevents infinite loops, but complex inheritance graphs can still produce surprising results. Keep hierarchies shallow when possible.
