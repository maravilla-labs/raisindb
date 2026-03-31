export function accessControlKnowledge(): string {
  return `# RaisinDB Access Control Reference

## Overview

RaisinDB uses a content-driven access control system. Users, Roles, and Groups are
stored as regular content nodes in the \`raisin:access_control\` workspace. Permissions
are defined declaratively in roles and evaluated at query time (Row-Level Security).

## The raisin:access_control Workspace

This is a built-in global workspace that stores all access control entities:

\`\`\`
raisin:access_control/
├── config/
│   └── default                    # raisin:SecurityConfig -- default policy
├── users/
│   ├── system/
│   │   └── anonymous              # raisin:User -- unauthenticated requests
│   └── internal/                  # System-managed users
├── roles/
│   ├── system_admin               # Full access to everything
│   ├── anonymous                  # Read-only access to public workspaces
│   └── authenticated_user         # Default role for logged-in users
├── groups/                        # User groups with aggregated roles
├── relation-types/                # Graph relation type definitions
├── circles/                       # Entity circles
└── graph-config/                  # Graph algorithm configs
\`\`\`

## Built-in Node Types

### raisin:User

User account with authentication and authorization information.

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| email | String | yes | Unique email address |
| display_name | String | yes | Display name |
| roles | Array | no | Direct role IDs (array of strings) |
| groups | Array | no | Group IDs the user belongs to |
| metadata | Object | no | Arbitrary metadata |
| birth_date | Date | no | For minor status / stewardship |
| can_login | Boolean | no | Whether user can authenticate (default true) |

Each user node automatically gets child folders:
- \`profile\` (raisin:Profile) -- user profile data
- \`inbox\` (raisin:MessageFolder) -- incoming messages
- \`outbox\` (raisin:MessageFolder) -- draft outgoing messages
- \`sent\` (raisin:MessageFolder) -- sent messages
- \`notifications\` (raisin:Folder) -- notification items

### raisin:Role

Role definition with permissions and optional inheritance.

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| name | String | yes | Unique role name |
| description | String | no | Human-readable description |
| inherits | Array | no | Role IDs this role inherits from |
| permissions | Array | no | Array of permission objects |

### raisin:Group

User group that aggregates roles for team-based assignment.

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| name | String | yes | Unique group name |
| description | String | no | Human-readable description |
| roles | Array | no | Role IDs assigned to all group members |

## Permission Object Structure

Each entry in a role's \`permissions\` array is an object with these fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| path | String | yes | Glob pattern (e.g. \`**\`, \`users/**\`, \`content/*\`) |
| operations | Array | yes | Allowed ops: create, read, update, delete, translate, relate, unrelate |
| workspace | String | no | Workspace pattern (glob). Omit for all workspaces |
| branch_pattern | String | no | Branch pattern (glob). Omit for all branches |
| node_types | Array | no | Restrict to specific node types |
| fields | Array | no | Whitelist: only these fields are accessible |
| except_fields | Array | no | Blacklist: these fields are NOT accessible |
| condition | String | no | REL expression that must evaluate to true |

### Path Patterns

- \`*\` -- matches one path segment
- \`**\` -- matches any number of segments (recursive)
- \`users/*/profile\` -- any user's profile
- \`content/**\` -- all content recursively

### Field Filtering

Use \`fields\` (whitelist) or \`except_fields\` (blacklist) to control which
properties are visible:

\`\`\`yaml
# Only expose display_name for public user listings
- path: "users/**"
  operations: ["read"]
  node_types: ["raisin:User"]
  fields: ["display_name"]

# Hide internal_notes from non-admins
- path: "content/**"
  operations: ["read"]
  except_fields: ["internal_notes", "review_score"]
\`\`\`

## REL Conditions (Row-Level Security)

REL (Raisin Expression Language) conditions enable dynamic, per-row access control.
A condition is a string expression that is evaluated at query time with context
variables from the authenticated user and the target node.

### Available Variables

**auth.* variables** (from the authenticated user):
- \`auth.user_id\` -- global identity ID (JWT sub claim)
- \`auth.local_user_id\` -- workspace-specific raisin:User node ID
- \`auth.email\` -- user's email
- \`auth.home\` -- user's home path (raisin:User node path)
- \`auth.is_anonymous\` -- whether user is unauthenticated
- \`auth.is_system\` -- whether this is a system operation
- \`auth.roles\` -- array of effective role IDs
- \`auth.groups\` -- array of group IDs

**node.* variables** (from the node being accessed):
- \`node.id\` -- node ID
- \`node.name\` -- node name (last path segment)
- \`node.path\` -- full hierarchical path
- \`node.node_type\` -- node type name
- \`node.created_by\` -- user ID who created the node
- \`node.updated_by\` -- user ID who last updated the node
- \`node.owner_id\` -- owner user ID
- \`node.workspace\` -- workspace name
- \`node.<property>\` -- any property from node.properties

### Condition Examples

\`\`\`yaml
# Owner-only access: user can only access nodes they created
condition: "node.created_by == auth.user_id"

# Home directory: user can access nodes under their home path
condition: "node.path.startsWith(auth.home)"

# Same local user: match on workspace-local user node ID
condition: "node.id == auth.local_user_id"

# Graph-based: friends can read (requires FRIENDS_WITH relation)
condition: "node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'"

# Graph-based with depth: friends-of-friends up to 2 hops
condition: "node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 2"

# Property-based: only published content
condition: "node.status == 'published'"

# Combined: owner OR editor role
condition: "node.created_by == auth.user_id || auth.roles.contains('editor')"
\`\`\`

## Permission Resolution

When a request comes in, permissions are resolved in this order:

1. **Find user** -- look up raisin:User node by identity ID or email
2. **Collect direct roles** -- from user's \`roles\` property
3. **Collect group roles** -- for each group in user's \`groups\`, get that group's \`roles\`
4. **Resolve inheritance** -- for each role, recursively follow \`inherits\` (with cycle detection)
5. **Flatten permissions** -- collect all permission objects from all effective roles
6. **Cache result** -- resolved permissions are cached for the session

The \`system_admin\` role is special: it bypasses all permission checks.

## Built-in Roles

### system_admin
Full access to all resources in all workspaces. Has \`**\` path pattern with all
operations. Assigned to the initial admin user.

### anonymous
Default role for unauthenticated requests. Typically read-only access to specific
public workspaces (e.g., \`launchpad\`).

### authenticated_user
Default role for all logged-in users. Provides:
- Read/update own user node (via \`node.id == auth.local_user_id\`)
- Read/update own profile (via \`node.path.startsWith(auth.home)\`)
- Read friends' profiles (via FRIENDS_WITH graph relation)
- Read limited fields for friends-of-friends (display_name, avatar, bio)
- Read display_name for all users (public)
- Manage own inbox, outbox, sent, notifications

## Workspace Access Workflows

### Request/Approve Flow
1. User requests access to a workspace (status: \`pending\`)
2. Admin reviews and approves or denies
3. On approval: a raisin:User node is created, status becomes \`active\`

### Invite/Accept Flow
1. Admin invites a user (status: \`invited\`)
2. User accepts or declines the invitation
3. On accept: a raisin:User node is created, status becomes \`active\`

### Access Statuses
- \`active\` -- user can access the workspace
- \`pending\` -- awaiting admin approval
- \`invited\` -- awaiting user acceptance
- \`denied\` -- request was denied
- \`revoked\` -- access was revoked after being granted
- \`declined\` -- user declined an invitation
- \`suspended\` -- temporarily suspended

## Stewardship (Parent/Guardian)

Stewardship allows a guardian user to act on behalf of a ward user. This is used
for parental controls, delegated administration, and similar scenarios.

- \`acting_as_ward\` on AuthContext indicates stewardship is active
- \`active_stewardship_source\` tracks which relation type or override authorized it
- Audit logs record actions as \`guardian_id:acting_as:ward_id\`

## SecurityConfig

The \`raisin:SecurityConfig\` node at \`/config/default\` controls global security settings:

\`\`\`yaml
workspace: "*"              # Which workspaces this config applies to
default_policy: "deny"      # Default deny -- only explicitly permitted operations allowed
anonymous_enabled: false    # Whether anonymous access is enabled
\`\`\`

## Creating Custom Roles

### Via YAML (in a package)

Create a role node in \`content/raisin:access_control/roles/\`:

\`\`\`yaml
# content/raisin:access_control/roles/content-editor/.node.yaml
node_type: raisin:Role
properties:
  role_id: "content_editor"
  name: "Content Editor"
  description: "Can create and edit content in the main workspace"
  inherits:
    - "authenticated_user"
  permissions:
    - path: "**"
      operations: ["create", "read", "update"]
      workspace: "main"
      node_types: ["myapp:Article", "myapp:Page"]
    - path: "**"
      operations: ["read"]
      workspace: "media"
\`\`\`

### Via SQL

\`\`\`sql
INSERT INTO "raisin:access_control" (path, node_type, properties)
VALUES ('/roles/content-editor', 'raisin:Role', '{
  "role_id": "content_editor",
  "name": "Content Editor",
  "description": "Can create and edit content",
  "inherits": ["authenticated_user"],
  "permissions": [
    {
      "path": "**",
      "operations": ["create", "read", "update"],
      "workspace": "main"
    }
  ]
}'::jsonb)
\`\`\`

## Assigning Roles to Users

\`\`\`sql
-- Add a role to a user
UPDATE "raisin:access_control"
SET properties = properties || '{"roles": ["content_editor", "authenticated_user"]}'::jsonb
WHERE node_type = 'raisin:User' AND properties ->> 'email' = 'user@example.com'

-- Add a user to a group
UPDATE "raisin:access_control"
SET properties = properties || '{"groups": ["editorial-team"]}'::jsonb
WHERE node_type = 'raisin:User' AND properties ->> 'email' = 'user@example.com'
\`\`\`

## Creating Groups

\`\`\`sql
INSERT INTO "raisin:access_control" (path, node_type, properties)
VALUES ('/groups/editorial-team', 'raisin:Group', '{
  "name": "editorial-team",
  "description": "Editorial team members",
  "roles": ["content_editor", "media_viewer"]
}'::jsonb)
\`\`\`
`;
}
