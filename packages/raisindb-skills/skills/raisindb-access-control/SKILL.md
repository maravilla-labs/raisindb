---
name: raisindb-access-control
description: "Roles, permissions, groups, and row-level security for RaisinDB. Configure anonymous access, custom roles, and fine-grained permissions in your package. Use when setting up authorization."
---

# RaisinDB Access Control

## The ACL Model

RaisinDB uses **content-driven security**. Permissions are not code -- they are regular content nodes stored in the built-in `raisin:access_control` workspace. Users, roles, and groups are all nodes with the types `raisin:User`, `raisin:Role`, and `raisin:Group`. Because they are content, you can query them with SQL, ship them in packages, and version them like any other data.

The workspace layout:

```
raisin:access_control/
├── config/
│   └── default                    # raisin:SecurityConfig -- global security settings
├── users/
│   ├── system/
│   │   └── anonymous              # raisin:User -- unauthenticated requests
│   └── internal/                  # System-managed users
├── roles/
│   ├── system_admin               # Full access to everything
│   ├── anonymous                  # Read-only access to public workspaces
│   └── authenticated_user         # Default role for logged-in users
├── groups/                        # User groups with aggregated roles
└── graph-config/                  # Graph algorithm configs
```

Permission resolution: find user -> collect direct roles -> collect group roles -> resolve inheritance (with cycle detection) -> flatten all permissions -> cache for session. The `system_admin` role bypasses all checks.

## Built-in Roles

### system_admin

Full access to all resources in all workspaces. Path pattern `**` with all operations. Assigned to the initial admin user created during setup.

### anonymous

Default role for unauthenticated requests. Typically grants read-only access to specific public workspaces (e.g., `launchpad`). Only active when `anonymous_enabled: true` in SecurityConfig.

### authenticated_user

Default role assigned to every logged-in user. Provides:

- Read/update own user node (via `node.id == auth.local_user_id`)
- Read/update own profile and home folder (via `node.path.startsWith(auth.home)`)
- Read friends' profiles (via `FRIENDS_WITH` graph relation)
- Read `display_name` for all users (public directory)
- Manage own inbox, outbox, sent, and notifications folders

## Permission Objects

Each entry in a role's `permissions` array is an object with these fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `path` | String | yes | Glob pattern matching node paths |
| `operations` | Array | yes | Allowed ops (see below) |
| `workspace` | String | no | Workspace name/pattern. Omit = all workspaces |
| `branch_pattern` | String | no | Branch pattern (glob). Omit = all branches |
| `node_types` | Array | no | Restrict to specific node types |
| `fields` | Array | no | Whitelist: only these fields are accessible |
| `except_fields` | Array | no | Blacklist: these fields are hidden |
| `condition` | String | no | REL expression for row-level security |

### Operations

`create`, `read`, `update`, `delete`, `translate`, `relate`, `unrelate`

### Path Patterns

- `*` -- matches exactly one path segment
- `**` -- matches any number of segments (recursive)
- `/users/*/profile` -- any user's profile node
- `/content/**` -- all content recursively
- `/**` -- everything in the workspace

Example permission:

```yaml
permissions:
  - path: "/**"
    operations: ["read"]
    workspace: "launchpad"
  - path: "/content/**"
    operations: ["create", "read", "update", "delete"]
    workspace: "main"
    node_types: ["myapp:Article", "myapp:Page"]
```

## Field Filtering

Use `fields` (whitelist) or `except_fields` (blacklist) to control which properties are visible. Never use both on the same permission entry.

```yaml
# Public user directory: only expose display_name
- path: "/users/**"
  operations: ["read"]
  node_types: ["raisin:User"]
  fields: ["display_name"]

# Hide internal metadata from non-admins
- path: "/content/**"
  operations: ["read"]
  except_fields: ["internal_notes", "review_score", "moderation_flags"]
```

`fields` returns only listed properties; `except_fields` returns all properties except listed ones.

## REL Conditions (Row-Level Security)

REL (Raisin Expression Language) conditions enable per-row access control. A condition is a string expression evaluated at query time using context from the authenticated user and the target node.

### Available Variables

**`auth.*` variables** (from the authenticated user):

| Variable | Description |
|----------|-------------|
| `auth.user_id` | Global identity ID (JWT `sub` claim) |
| `auth.local_user_id` | Workspace-specific `raisin:User` node ID |
| `auth.email` | User's email address |
| `auth.home` | User's home path (`raisin:User` node path) |
| `auth.is_anonymous` | Whether user is unauthenticated |
| `auth.is_system` | Whether this is a system operation |
| `auth.roles` | Array of effective role IDs |
| `auth.groups` | Array of group IDs |

**`node.*` variables** (from the node being accessed):

| Variable | Description |
|----------|-------------|
| `node.id` | Node ID |
| `node.name` | Node name (last path segment) |
| `node.path` | Full hierarchical path |
| `node.node_type` | Node type name |
| `node.created_by` | User ID who created the node |
| `node.updated_by` | User ID who last updated the node |
| `node.owner_id` | Owner user ID |
| `node.workspace` | Workspace name |
| `node.<property>` | Any property from `node.properties` |

### Condition Examples

```yaml
# Owner-only: user can only access nodes they created
condition: "node.created_by == auth.user_id"

# Home directory: access nodes under user's own path
condition: "node.path.startsWith(auth.home)"

# Match on workspace-local user node
condition: "node.id == auth.local_user_id"

# Graph-based: friends can read (requires FRIENDS_WITH relation)
condition: "node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH'"

# Graph-based with depth: friends-of-friends up to 2 hops
condition: "node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 2"

# Property-based: only published content
condition: "node.status == 'published'"

# Combined: owner OR user has editor role
condition: "node.created_by == auth.user_id || auth.roles.contains('editor')"
```

## Gotchas (owner data, create, role assignment)

- **An owner condition cannot gate `create`.** A permission whose `operations`
  include `create` *together with* `condition: "node.created_by == auth.user_id"`
  **denies** creation — at create-check time `created_by` isn't set yet, so the
  condition evaluates false. **Split it**: one entry for `create` with **no**
  condition, a second for `read`/`update`/`delete` with the owner condition.

  ```yaml
  permissions:
    - { path: "/**", operations: [create], workspace: app_data }
    - path: "/**"
      operations: [read, update, delete]
      workspace: app_data
      condition: "node.created_by == auth.user_id"
  ```

- **Logged-in users cannot create arbitrary nodes under their own home.** The
  built-in `authenticated_user` role only covers specific home subfolders
  (inbox/outbox/sent/notifications). For per-user app data, use a **dedicated
  workspace** + a custom role with the owner permission above — don't write
  under the user's home path.

- **Assigning a role to new users.** New accounts receive the repo's
  `default_roles` (a property on the `raisin:RepoAuthConfig` node — there is no
  CLI for it). To grant a custom role on signup, add a **trigger** on
  `raisin:User` Created (filter `paths: ["/users/internal/*"]`,
  `node_types: [raisin:User]`, workspace `raisin:access_control`) whose function
  appends the role to the user's `properties.roles`. See
  `raisindb-functions-triggers`. (After assignment, the user must reconnect/
  re-login for the new role's permissions to take effect — sessions cache the
  resolved permission set.)

## Managing roles at runtime — SQL ACL DDL (and the reinstall trap)

RaisinDB SQL has dedicated **ACL DDL** — prefer it over hand-editing a role node's
`properties.permissions` JSON:

```sql
CREATE ROLE 'editor' ...                                    -- create
ALTER ROLE 'Editor' ADD PERMISSION ALLOW read, update, delete
        ON 'content' PATH '/**' WHERE node.created_by == auth.user_id;
ALTER ROLE 'Editor' DROP PERMISSION 3;                      -- drop the Nth permission (0-based)
ALTER ROLE 'Editor' ADD INHERITS ('authenticated_user');
GRANT ROLE 'editor' TO USER 'user@example.com';             -- assign a role to a user
REVOKE ROLE 'editor' FROM USER 'user@example.com';
SHOW PERMISSIONS FOR ROLE 'Editor';
```

Hard-won caveats:

- **`ALTER ROLE 'X'` keys off the role's NODE PATH SEGMENT, not its `role_id`.**
  A role created from `content/.../roles/member/.node.yaml` with `name: Member`
  lives at `/roles/Member` — so it's `ALTER ROLE 'Member'` (capitalized), even
  though its `role_id` is `member`. Wrong casing → `Role 'x' not found`.
  `GRANT ROLE` uses the `role_id`.
- **ACL DDL requires an authenticated `system_admin` session.** A server API key
  / SSR client is rejected with `Forbidden: Authentication required for access
  control operations`. Run these in the admin console's SQL editor (or as a
  server-side function, which executes in system context).
- **`deploy --install` MERGES role permissions** (appends, does not replace). So
  a role shipped in a package accumulates duplicate/stale permission entries on
  every reinstall and drifts/corrupts. **Keep roles you'll evolve OUT of the
  package** (don't ship them, or `.bak` them after first creation) and manage
  them with `ALTER ROLE` instead — otherwise the next `deploy --install`
  re-corrupts your fix.

### The default `viewer` over-read foot-gun

The default `viewer` role often grants `read` with **no workspace** (omitted =
**all** workspaces) and `path: /**` — i.e. read EVERYTHING, including
owner-scoped data in other workspaces. Since `viewer` is in `default_roles`,
this silently breaks owner isolation: a per-user `members` workspace gated by
`created_by == auth.user_id` still leaks because `viewer` unions an unconditioned
read on top. **Scope `viewer` to the public content workspace only:**

```sql
ALTER ROLE 'Viewer' DROP PERMISSION 0;
ALTER ROLE 'Viewer' ADD PERMISSION ALLOW read ON 'content' PATH '/**';
```

When isolation "works in a quick test but leaks later", check every role the
user holds (especially `viewer`) for an unconditioned cross-workspace read —
ACL grants are a UNION, so the most permissive matching grant wins.

> **Identity note:** `auth.user_id` equals a node's `created_by` and is the
> user's global identity id (verified). So `condition: node.created_by ==
> auth.user_id` is the correct owner check, and `auth.user_id` is the value a
> graph edge endpoint must equal for a `node RELATES auth.user_id VIA '...'`
> condition to match.

## Creating Custom Roles in Package YAML

Roles are shipped as part of a package under `content/_raisin__access_control/roles/{role-name}/.node.yaml`. The path prefix `_raisin__access_control` is the encoded form of the `raisin:access_control` workspace (colons become double underscores).

### Viewer Role Example

From the launchpad-next package (`content/_raisin__access_control/roles/Viewer/.node.yaml`):

```yaml
node_type: raisin:Role
properties:
  role_id: viewer
  name: Viewer
  description: Default role for authenticated users with full access to their home folder.
  permissions:
    # Full access to user's own home folder and all children
    # No node_types filter = allows ALL node types (including AI types)
    - path: "/users/**"
      operations: ["create", "read", "update", "delete"]
      workspace: "raisin:access_control"
    # Read access to launchpad workspace (public pages)
    - path: "/**"
      operations: ["read"]
      workspace: "launchpad"
    # Read access to functions workspace (agents, functions)
    - path: "/**"
      operations: ["read"]
      workspace: "functions"
```

### Custom Content Editor Role

```yaml
# content/_raisin__access_control/roles/content-editor/.node.yaml
node_type: raisin:Role
properties:
  role_id: content_editor
  name: Content Editor
  description: Can create and edit content in the main workspace
  inherits:
    - authenticated_user
  permissions:
    - path: "/**"
      operations: ["create", "read", "update"]
      workspace: "main"
      node_types: ["myapp:Article", "myapp:Page"]
    - path: "/**"
      operations: ["read"]
      workspace: "media"
```

Key points:
- `role_id` is the identifier used when assigning roles to users.
- `inherits` lists role IDs whose permissions are merged into this role.
- The folder name (e.g., `content-editor`) becomes the node's path segment; `role_id` is what the system uses internally.

## Groups

Groups aggregate roles for team-based assignment. Instead of assigning five roles to every user on a team, create a group with those roles and add users to the group.

### Group Node Type (`raisin:Group`)

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | String | yes | Unique group name |
| `description` | String | no | Human-readable description |
| `roles` | Array | no | Role IDs assigned to all group members |

### Package YAML and SQL

```yaml
# content/_raisin__access_control/groups/editorial-team/.node.yaml
node_type: raisin:Group
properties:
  name: editorial-team
  description: Editorial team members
  roles:
    - content_editor
    - media_viewer
```

Assign a user to a group via SQL:

```sql
UPDATE "raisin:access_control"
SET properties = properties || '{"groups": ["editorial-team"]}'::jsonb
WHERE node_type = 'raisin:User'
  AND properties->>'email'::String = 'user@example.com'
```

## Anonymous Access

Anonymous access lets unauthenticated users interact with your application (e.g., viewing public pages).

### Step 1: Enable in SecurityConfig

The `raisin:SecurityConfig` node at `/config/default` in `raisin:access_control` controls global settings:

```yaml
# config/default -- raisin:SecurityConfig
workspace: "*"
default_policy: "deny"
anonymous_enabled: true       # Set to true to allow unauthenticated access
```

When `anonymous_enabled` is `false` (the default), all unauthenticated requests are rejected before permission checks run.

### Step 2: Configure the Anonymous Role

The built-in `anonymous` role defines what unauthenticated users can do. Give it read access to public workspaces:

```yaml
# content/_raisin__access_control/roles/anonymous/.node.yaml
node_type: raisin:Role
properties:
  role_id: anonymous
  name: Anonymous
  description: Read-only access for unauthenticated users
  permissions:
    - path: "/**"
      operations: ["read"]
      workspace: "launchpad"
```

The system user at `/users/system/anonymous` is automatically used for unauthenticated requests when anonymous access is enabled.

## workspace_patches in manifest.yaml

When your package needs to store custom node types in the `raisin:access_control` workspace (e.g., messages, conversations, AI nodes), declare them in `workspace_patches` in your `manifest.yaml`:

```yaml
# manifest.yaml
name: my-app
version: 1.0.0

workspace_patches:
  "raisin:access_control":
    allowed_node_types:
      add:
        - raisin:Folder
        - raisin:Message
        - raisin:Conversation
        - raisin:AIConversation
        - raisin:AIMessage
```

Without this patch, creating nodes of unlisted types in that workspace fails validation. You can also patch your own workspaces:

```yaml
workspace_patches:
  launchpad:
    allowed_node_types:
      add:
        - launchpad:Page
        - raisin:Folder
        - raisin:Asset
```

## Validation

**MANDATORY** — run after every YAML change in `package/`:

    npm run validate

This checks role YAML validity, required permission fields (`path`, `operations`), referenced node types, workspace patches, and folder structure. Fix all errors before proceeding.

## Quick Reference

| Task | How |
|------|-----|
| Add a custom role | Create `content/_raisin__access_control/roles/{name}/.node.yaml` |
| Add a group | Create `content/_raisin__access_control/groups/{name}/.node.yaml` |
| Enable anonymous access | Set `anonymous_enabled: true` in SecurityConfig + configure anonymous role |
| Restrict to node types | Add `node_types: [...]` to permission entry |
| Owner-only access | Add `condition: "node.created_by == auth.user_id"` |
| Hide fields | Use `except_fields: [...]` on the permission entry |
| Expose only certain fields | Use `fields: [...]` on the permission entry |
| Allow custom types in AC workspace | Add `workspace_patches` to `manifest.yaml` |
| Validate package | Run `npm run validate` |
