# Access Control & Authorization

RaisinDB implements a content-centric, workspace-scoped authorization system with row-level security (RLS). Rather than bolting access control onto the database as an afterthought, permissions are modeled as first-class data -- stored as nodes in a dedicated workspace, resolved through role inheritance and group membership, and enforced at query time through the RLS filter.

This chapter covers the full design: from the two-tier identity model and permission data structures, through the resolution pipeline, to runtime enforcement via REL conditions and field-level filtering.

## Design Philosophy

Three principles shape RaisinDB's approach to authorization:

**Content-centric.** Permissions are defined in terms of content paths and node types, not API endpoints or database tables. A permission like `content.articles.**` with operations `[read, update]` directly maps to the content hierarchy. This makes policies readable by non-engineers and composable across workspaces.

**Workspace-scoped.** Each workspace is an independent authorization boundary. A user might be an `editor` in the `content` workspace, a `viewer` in `media`, and have no access to `analytics`. This mirrors how organizations actually divide responsibility over content.

**Graph-enhanced.** Because RaisinDB stores users, roles, and groups as nodes in a content graph, the same query and replication infrastructure that serves application data also serves access control data. Role inheritance is a graph traversal. Group membership is a node property. Permission changes replicate across the cluster through the same CRDT mechanisms as any other data.

## Two-Tier Identity Model

RaisinDB separates **global identity** from **workspace-specific user accounts**:

```
┌─────────────────────────────────────┐
│         Global Identity             │
│  (stored in raisin:system)          │
│                                     │
│  identity_id: "id-abc123"           │
│  email: "alice@example.com"         │
│  tenant_id: "acme"                  │
│  linked_providers:                  │
│    - oidc:google (ext: "g-789")     │
│    - local (password)               │
│  local_credentials: { hash: "..." } │
└─────────────┬───────────────────────┘
              │
              │  WorkspaceAccess records
              │  (one per workspace)
              │
    ┌─────────┼──────────┐
    ▼                     ▼
┌──────────────┐   ┌──────────────┐
│ raisin:User  │   │ raisin:User  │
│ (content ws) │   │ (media ws)   │
│              │   │              │
│ roles:       │   │ roles:       │
│  - editor    │   │  - viewer    │
│ groups:      │   │ groups:      │
│  - team-a    │   │  - team-a    │
└──────────────┘   └──────────────┘
```

### Global Identity

An `Identity` (defined in `raisin-models/src/auth/identity.rs`) represents a unique person within a tenant. It is stored in the `raisin:system` workspace and carries:

- `identity_id` -- UUID, globally unique within the tenant
- `email` -- primary identifier, unique within tenant
- `linked_providers` -- external auth providers (OIDC, SAML, etc.)
- `local_credentials` -- optional username/password with bcrypt hashing, lockout tracking

A single identity can have multiple authentication providers linked. For example, a user might log in via Google OIDC or a local password -- both resolve to the same identity.

### Workspace-Specific User

A `raisin:User` node (defined in `raisin_user.yaml`) lives inside the `raisin:access_control` workspace and represents the user's presence within a specific context. It carries workspace-local data:

| Property | Type | Description |
|----------|------|-------------|
| `email` | String | User's email (indexed, unique) |
| `display_name` | String | Display name |
| `roles` | Array\<String\> | Direct role assignments (e.g., `["editor", "reviewer"]`) |
| `groups` | Array\<String\> | Group memberships (e.g., `["team-a", "engineering"]`) |
| `metadata` | Object | Custom key-value data |
| `birth_date` | Date | For minor status calculation |
| `can_login` | Boolean | Whether user can authenticate (default: `true`) |

Each `raisin:User` node is created with an initial structure including `profile`, `inbox`, `outbox`, `sent`, and `notifications` child nodes.

### Why Two Tiers?

This separation serves several purposes:

1. **Authentication vs. authorization** -- The identity layer handles "who are you?" while the user node handles "what can you do here?" These concerns change independently.
2. **Multi-workspace access** -- A user can have different roles in different workspaces without duplicating identity data.
3. **External auth integration** -- The identity links to external providers; the user node is purely internal to RaisinDB.
4. **Replication isolation** -- Workspace-specific user data replicates with the workspace. Global identity data is tenant-level.

The bridge between the two tiers is the `WorkspaceAccess` record, described in the [Workspace Access Workflows](#workspace-access-workflows) section.

## The raisin:access_control Workspace

Every repository has a built-in workspace called `raisin:access_control`. This workspace stores all authorization entities as nodes:

```
raisin:access_control/
├── users/
│   ├── system/
│   │   └── anonymous          (raisin:User - physical anonymous user)
│   ├── alice                  (raisin:User)
│   │   ├── profile            (raisin:Profile)
│   │   ├── inbox              (raisin:MessageFolder)
│   │   ├── outbox             (raisin:MessageFolder)
│   │   ├── sent               (raisin:MessageFolder)
│   │   └── notifications      (raisin:Folder)
│   └── bob                    (raisin:User)
│       └── ...
├── roles/
│   ├── system_admin           (raisin:Role)
│   ├── editor                 (raisin:Role)
│   ├── viewer                 (raisin:Role)
│   └── anonymous              (raisin:Role)
└── groups/
    ├── engineering             (raisin:Group)
    └── content-team            (raisin:Group)
```

### Node Types

**raisin:User** -- User accounts with roles, groups, and profile data. Each user node is the authoritative source for what that user can do in this workspace.

**raisin:Role** -- Role definitions containing permissions and inheritance chains. Properties:

| Property | Type | Description |
|----------|------|-------------|
| `name` | String | Role identifier (unique, indexed) |
| `description` | String | Human-readable description |
| `inherits` | Array\<String\> | Role IDs this role inherits from |
| `permissions` | Array\<Object\> | Permission grant objects |

Roles are versionable and publishable, meaning permission changes can go through a draft/publish workflow.

**raisin:Group** -- Named groups of users with assigned roles. Properties:

| Property | Type | Description |
|----------|------|-------------|
| `name` | String | Group name (unique, indexed) |
| `description` | String | Human-readable description |
| `roles` | Array\<String\> | Roles assigned to all group members |

**raisin:AclFolder** -- Organizational folder that extends `raisin:Folder`. Can contain Users, Roles, Groups, and nested AclFolders. Used to organize the access control hierarchy.

**raisin:WorkspaceAccess** -- Records linking global identities to workspace-specific users. Tracks access status, who granted access, and when.

## Roles & Permissions

### Role Definition

A role is a named collection of permission grants. Here is what a role node's `permissions` array looks like as stored data:

```json
{
  "name": "content-editor",
  "description": "Can manage articles in the content workspace",
  "inherits": ["viewer"],
  "permissions": [
    {
      "workspace": "content",
      "path": "articles/**",
      "operations": ["create", "read", "update", "delete"],
      "node_types": ["blog:Article", "blog:Draft"],
      "except_fields": ["internal_notes"],
      "condition": "node.created_by == auth.user_id"
    },
    {
      "path": "media/**",
      "operations": ["read"],
      "fields": ["title", "url", "thumbnail"]
    }
  ]
}
```

### Permission Structure

Each permission grant (defined in `raisin-models/src/permissions/permission.rs`) contains:

| Field | Type | Description |
|-------|------|-------------|
| `workspace` | Option\<String\> | Workspace pattern (glob). `None` = all workspaces |
| `branch_pattern` | Option\<String\> | Branch pattern (glob). `None` = all branches |
| `path` | String | Path pattern for content matching |
| `node_types` | Option\<Vec\<String\>\> | Restrict to specific node types. `None` = all types |
| `operations` | Vec\<Operation\> | Allowed operations |
| `fields` | Option\<Vec\<String\>\> | Field whitelist (only these fields accessible) |
| `except_fields` | Option\<Vec\<String\>\> | Field blacklist (all fields except these) |
| `condition` | Option\<String\> | REL expression that must evaluate to truthy |

### Operations

Seven operations can be granted:

| Operation | Description |
|-----------|-------------|
| `create` | Create new nodes |
| `read` | View/query nodes |
| `update` | Modify existing nodes |
| `delete` | Remove nodes |
| `translate` | Modify translations on nodes |
| `relate` | Create relationships between nodes |
| `unrelate` | Remove relationships between nodes |

### Scope Matching

Workspace and branch patterns use glob-style matching (compiled via the `glob` crate):

- `content` -- exact match
- `content-*` -- matches `content-us`, `content-eu`
- `*` or empty -- matches all workspaces/branches
- `?` -- matches a single character
- `features/*` -- matches `features/auth`, `features/login`

Patterns are compiled once into `ScopeMatcher` instances and cached on the `Permission` struct via `OnceLock` for repeated evaluation.

### Path Matching

Path patterns use a regex-based glob system (defined in `raisin-models/src/permissions/path_matcher.rs`):

- `*` -- matches any characters except `/` (single segment)
- `**` -- matches any characters including `/` (recursive)
- `?` -- matches any single character except `/`

Examples:

```
/articles/*          matches /articles/news, NOT /articles/news/2024
/articles/**         matches /articles, /articles/news, /articles/a/b/c
/users/*/profile     matches /users/alice/profile, NOT /users/a/b/profile
/**/blog/**          matches /blog, /foo/blog/post, /a/b/blog/x/y
/00**                matches /00, /001, /00abc, /00/child
```

Path matchers are pre-compiled into regex patterns and cached on each `Permission` for efficiency. When multiple permissions match the same node, the most specific pattern wins, scored by a specificity algorithm:

- Exact segment: 100 points
- Single wildcard `*`: 10 points
- Prefix with `**`: 5 points
- Recursive `**`: 1 point
- Length bonus: `segments * 5`

### Role Inheritance

Roles can inherit from other roles via the `inherits` property:

```
system_admin
    ↑ inherits
  admin
    ↑ inherits
  editor
    ↑ inherits
  viewer
```

Inheritance is resolved recursively with cycle detection (implemented in `PermissionService::resolve_role_inheritance`). The algorithm uses a visited set to prevent infinite loops:

1. Start with the user's directly assigned roles
2. For each role, check its `inherits` array
3. Add inherited roles to the processing queue
4. Skip roles already visited (cycle detection)
5. Continue until the queue is empty

All permissions from all roles in the inheritance chain are collected into a flat list.

## Groups

Groups provide a layer of indirection between users and roles. Instead of assigning roles to individual users, you assign roles to groups and users to groups:

```
User: alice
  groups: ["engineering", "content-team"]

Group: engineering
  roles: ["developer", "viewer"]

Group: content-team
  roles: ["editor"]

Effective roles for alice:
  direct: []
  from groups: ["developer", "viewer", "editor"]
```

Groups are looked up by their `name` property (not node ID) via `find_by_property` on the storage layer.

**When to use groups vs. direct roles:**

- Use **groups** when multiple users share the same role set and you want to change permissions for all of them at once
- Use **direct roles** for individual exceptions or temporary elevated access

## Permission Resolution

The `PermissionService` (in `raisin-core/src/services/permission_service/mod.rs`) resolves a user's effective permissions through this pipeline:

```
┌─────────────────┐
│   raisin:User    │
│   node lookup    │
│ (by email,       │
│  identity_id,    │
│  or node_id)     │
└────────┬────────┘
         │
         ▼
┌─────────────────┐     ┌─────────────────┐
│  Direct Roles   │     │     Groups      │
│  from user.roles│     │  from user.groups│
└────────┬────────┘     └────────┬────────┘
         │                       │
         │              ┌────────▼────────┐
         │              │  Group Roles    │
         │              │  from each      │
         │              │  group.roles    │
         │              └────────┬────────┘
         │                       │
         └───────────┬───────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  Deduplicate roles    │
         │  (HashSet)            │
         └───────────┬───────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  Resolve inheritance  │
         │  (recursive, with     │
         │   cycle detection)    │
         └───────────┬───────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  Collect permissions  │
         │  from all effective   │
         │  roles                │
         └───────────┬───────────┘
                     │
                     ▼
         ┌───────────────────────┐
         │  ResolvedPermissions  │
         │  {                    │
         │    user_id,           │
         │    email,             │
         │    direct_roles,      │
         │    group_roles,       │
         │    effective_roles,   │
         │    groups,            │
         │    permissions: [...],│
         │    is_system_admin,   │
         │    resolved_at        │
         │  }                    │
         └───────────────────────┘
```

The result is a `ResolvedPermissions` struct that contains:

- `user_id` -- the workspace-specific user node ID
- `email` -- for auth variable resolution in REL conditions
- `direct_roles` -- roles assigned directly to the user
- `group_roles` -- roles inherited through group membership
- `effective_roles` -- all roles after inheritance resolution (deduplicated)
- `groups` -- group IDs the user belongs to
- `permissions` -- flat list of all permission grants from all effective roles
- `is_system_admin` -- `true` if `system_admin` is among effective roles (bypasses all checks)
- `resolved_at` -- timestamp for cache TTL validation

### Special Cases

**System admin**: If any effective role is `system_admin`, the user gets a single `Permission::full_access("**")` and `is_system_admin` is set to `true`. All subsequent permission checks short-circuit.

**Anonymous access**: Resolved by finding the physical `raisin:User` node with `user_id: "anonymous"` at `/users/system/anonymous`. This goes through the normal resolution pipeline (roles, groups, inheritance). Falls back to extracting permissions from just the `anonymous` role if no physical user node exists.

**Deny all**: An empty permissions list with user_id `$deny`. Used when anonymous access is disabled and no auth token is provided.

## Row-Level Security (RLS)

Permissions are enforced at query time by the RLS filter (in `raisin-core/src/services/rls_filter/`). Every node read from storage passes through this filter before being returned to the caller.

### Filter Pipeline

```
Node from storage
        │
        ▼
┌───────────────────┐
│ System context?   │──── yes ──→ Return node (bypass)
└───────────┬───────┘
            │ no
            ▼
┌───────────────────┐
│ Permissions       │──── none ──→ Deny (return None)
│ resolved?         │
└───────────┬───────┘
            │ yes
            ▼
┌───────────────────┐
│ is_system_admin?  │──── yes ──→ Return node (bypass)
└───────────┬───────┘
            │ no
            ▼
┌───────────────────┐
│ Find matching     │──── none ──→ Deny (return None)
│ permission        │
│ (scope + path +   │
│  operation +      │
│  node_type)       │
└───────────┬───────┘
            │ found
            ▼
┌───────────────────┐
│ Evaluate REL      │──── false ──→ Deny (return None)
│ condition         │
│ (if present)      │
└───────────┬───────┘
            │ true
            ▼
┌───────────────────┐
│ Apply field       │
│ filtering         │
│ (whitelist or     │
│  blacklist)       │
└───────────┬───────┘
            │
            ▼
    Return filtered node
```

### Permission Matching

The `find_matching_permission` function (in `rls_filter/matching.rs`) finds the best permission for a given node and operation. It evaluates permissions in this order:

1. **Scope match** -- workspace and branch patterns (fail-fast, O(1) per permission)
2. **Path match** -- node's path against the permission's path pattern
3. **Operation match** -- the permission must include the requested operation
4. **Node type match** -- if `node_types` is set, the node's type must be listed
5. **Specificity** -- among all matching permissions, the most specific path pattern wins

### Field Filtering

After a matching permission is found, field-level filtering is applied:

- **Whitelist** (`fields`): Only the listed fields are retained in the node's properties. All other properties are stripped.
- **Blacklist** (`except_fields`): All fields are retained except the listed ones.
- Whitelist takes precedence over blacklist if both are somehow set.

### Write Operations

The `can_perform` function checks whether a user can perform a specific operation (create, update, delete, etc.) on a node. It follows the same matching logic as read filtering but for the requested operation.

The `can_create_at_path` function handles the special case of node creation, where no node exists yet. It checks permissions against the target path and node type.

## REL Conditions

REL (Raisin Expression Language) conditions enable dynamic, runtime access control. A condition is a string expression attached to a permission that must evaluate to truthy for the permission to apply.

### Available Variables

When a REL condition is evaluated, two objects are available in the expression context:

**`auth.*` variables:**

| Variable | Type | Description |
|----------|------|-------------|
| `auth.user_id` | String\|Null | Global identity ID (from JWT `sub`) |
| `auth.local_user_id` | String\|Null | Workspace-specific user node ID |
| `auth.email` | String\|Null | User's email address |
| `auth.is_anonymous` | Boolean | Whether this is an anonymous request |
| `auth.is_system` | Boolean | Whether this is a system operation |
| `auth.roles` | Array\<String\> | Effective role IDs |
| `auth.groups` | Array\<String\> | Group IDs |
| `auth.acting_as_ward` | String\|Null | Ward's user ID if acting as steward |
| `auth.active_stewardship_source` | String\|Null | Stewardship relation type |
| `auth.home` | String\|Null | User's home path in the repository |

**`node.*` variables:**

| Variable | Type | Description |
|----------|------|-------------|
| `node.id` | String | Node UUID |
| `node.name` | String | Node name |
| `node.path` | String | Full path in the hierarchy |
| `node.node_type` | String | Node type (e.g., `blog:Article`) |
| `node.created_by` | String\|Null | Identity ID of the creator |
| `node.updated_by` | String\|Null | Identity ID of the last updater |
| `node.owner_id` | String\|Null | Node owner identity ID |
| `node.workspace` | String\|Null | Workspace the node belongs to |
| `node.<property>` | Any | Any node property is available by key |

Node properties are automatically converted to REL values: strings, numbers, booleans, arrays, objects, and null are all supported. Composite types (Geometry, Element) map to null.

### Fail-Closed Evaluation

If a REL condition fails to parse or evaluate (e.g., referencing a non-existent variable), the result is `false` -- access is denied. This is a deliberate security choice:

```rust
match raisin_rel::eval(expr, &ctx) {
    Ok(value) => value.is_truthy(),
    Err(_) => false, // Fail-closed: deny on error
}
```

### Practical Examples

**Ownership-based access** -- users can only edit their own content:

```
node.created_by == auth.user_id
```

**Role-based condition** -- only editors can see draft content:

```
auth.roles.contains('editor')
```

**Group-based condition** -- only engineering team members:

```
auth.groups.contains('engineering')
```

**Property-based condition** -- only published articles:

```
node.status == 'published'
```

**Home directory access** -- users can access content under their home path:

```
node.path.startsWith(auth.home)
```

**Combined conditions** -- ownership OR admin:

```
node.created_by == auth.user_id || auth.roles.contains('admin')
```

### Structured Conditions (RoleCondition)

In addition to REL string expressions, RaisinDB supports structured conditions (defined in `raisin-models/src/permissions/condition.rs`) for programmatic use:

- `PropertyEquals` -- `author == $auth.user_id`
- `PropertyIn` -- `status IN ['draft', 'review']`
- `PropertyGreaterThan` / `PropertyLessThan` -- numeric comparisons
- `UserHasRole` -- check if user has a specific role
- `UserInGroup` -- check if user is in a specific group
- `All` -- AND composition of sub-conditions
- `Any` -- OR composition of sub-conditions

Condition values can be literals or auth variable references (`$auth.user_id`, `$auth.email`).

## Workspace Access Workflows

The `WorkspaceAccess` record (defined in `raisin-models/src/auth/access.rs`) bridges global identities to workspace-specific users. It tracks the lifecycle of workspace access through a state machine:

```
                  ┌─────────┐
    Request  ────►│ Pending │
                  └────┬────┘
                       │
              ┌────────┼────────┐
              ▼                 ▼
        ┌──────────┐     ┌──────────┐
        │  Active  │     │  Denied  │
        └────┬─────┘     └──────────┘
             │
             ▼
        ┌──────────┐
        │ Revoked  │
        └──────────┘

                  ┌─────────┐
    Invite  ────►│ Invited │
                  └────┬────┘
                       │
              ┌────────┼────────┐
              ▼                 ▼
        ┌──────────┐     ┌──────────┐
        │  Active  │     │ Declined │
        └──────────┘     └──────────┘

    Direct grant ────► Active

    Suspend ────► Suspended
```

### Access Statuses

| Status | Description | `allows_access()` | `is_final()` |
|--------|-------------|:------------------:|:------------:|
| `Active` | User can access the workspace | yes | no |
| `Pending` | Access requested, awaiting approval | no | no |
| `Invited` | User invited, hasn't accepted | no | no |
| `Denied` | Request was denied | no | yes |
| `Revoked` | Access was revoked after being active | no | yes |
| `Declined` | User declined the invitation | no | yes |
| `Suspended` | Access temporarily suspended | no | no |

### Request Flow

1. User calls the request endpoint with their identity
2. A `WorkspaceAccess` record is created with status `Pending`
3. An admin reviews and calls `approve()` or `deny()`
4. On approval: a `raisin:User` node is created in `raisin:access_control`, the `user_node_id` is set, status becomes `Active`

### Invitation Flow

1. Admin creates an invitation with target identity and initial roles
2. A `WorkspaceAccess` record is created with status `Invited`
3. User calls `accept_invitation()` or `decline_invitation()`
4. On acceptance: a `raisin:User` node is created, status becomes `Active`

### Direct Grant

For programmatic access (e.g., during setup), `WorkspaceAccess::new_active()` creates a record that is immediately `Active` with a pre-created user node.

### WorkspaceAccess Record

Each record tracks:

- `identity_id` -- global identity
- `tenant_id`, `repo_id` -- scope
- `user_node_id` -- link to the workspace-specific `raisin:User` node
- `status` -- current access status
- `granted_at`, `granted_by` -- when and who granted access
- `requested_at` -- when the request was made
- `roles` -- roles assigned in this workspace
- `notes` -- reason for access decisions

## Stewardship

RaisinDB supports a parent/guardian model where one user can act on behalf of another (the "ward"). This is relevant for families, legal guardianships, and delegated administration -- any situation where one person must manage another's account and data. The stewardship system is implemented as a built-in package (`raisin-stewardship`, version 1.0.0, `builtin: true`) that depends on the `raisin-relationships` package.

### Relation Types

Stewardship builds on a general-purpose relationship layer. The `raisin:RelationType` node type (from `relation-type.yaml`) defines how two entities can be related:

| Property | Type | Required | Description |
|----------|------|:--------:|-------------|
| `relation_name` | String | yes (unique) | Machine identifier (e.g., `PARENT_OF`) |
| `title` | String | yes | Human-readable label |
| `description` | String | no | Longer explanation of the relationship |
| `category` | String | no | Grouping category (`household`, `legal`, `organization`) |
| `inverse_relation_name` | String | no | The inverse relation (e.g., `PARENT_OF` inverses to `CHILD_OF`) |
| `bidirectional` | Boolean | no | If `true`, the relation applies in both directions (default: `false`) |
| `implies_stewardship` | Boolean | no | If `true`, the source entity can act as steward for the target (default: `false`) |
| `requires_minor` | Boolean | no | If `true`, the target must be a minor for the relation to be valid (default: `false`) |
| `enables_messaging` | Boolean | no | If `true`, enables direct messaging between related entities (default: `false`) |
| `icon` | String | no | Display icon identifier |
| `color` | String | no | Display color |

The following relation types are pre-configured:

| Relation | Category | Inverse | Bidirectional | Implies Stewardship | Requires Minor |
|----------|----------|---------|:-------------:|:-------------------:|:--------------:|
| `PARENT_OF` | household | `CHILD_OF` | no | yes | yes |
| `GUARDIAN_OF` | legal | `WARD_OF` | no | yes | no |
| `SPOUSE_OF` | household | -- | yes | no | no |
| `SIBLING_OF` | household | -- | yes | no | no |
| `GRANDPARENT_OF` | household | `GRANDCHILD_OF` | no | no | no |
| `MANAGER_OF` | organization | `REPORTS_TO` | no | no | no |
| `ASSISTANT_OF` | organization | `HAS_ASSISTANT` | no | no | no |

Only relation types with `implies_stewardship: true` grant the ability to act on behalf of another user. `PARENT_OF` additionally requires the target to be a minor (determined by `birth_date` and the configured age threshold), while `GUARDIAN_OF` does not -- a legal guardian can manage an adult ward's account.

### Entity Circles

The `raisin:EntityCircle` node type groups related entities together (e.g., a family, a team):

| Property | Type | Required | Description |
|----------|------|:--------:|-------------|
| `name` | String | yes | Circle name (e.g., "Smith Family") |
| `circle_type` | String | no | One of: `family`, `team`, `org_unit`, `department`, `project`, `custom` |
| `primary_contact_id` | String | no | User ID of the primary contact for this circle |
| `address` | Object | no | Shared address for the circle |
| `metadata` | Object | no | Custom key-value data |

Members are linked to a circle via the `MEMBER_OF` relation. Circles are organizational -- they do not grant permissions directly, but they provide a convenient way to visualize and manage groups of related users.

### Stewardship Configuration

The `raisin:StewardshipConfig` node type controls how stewardship behaves within a deployment:

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `enabled` | Boolean | `false` | Master switch for stewardship features |
| `stewardship_relation_types` | Array | `["PARENT_OF", "GUARDIAN_OF"]` | Which relation types grant stewardship |
| `require_minor_for_parent` | Boolean | `true` | Whether `PARENT_OF` requires the ward to be a minor |
| `allowed_workflows` | Array | `["invitation", "admin_assignment", "steward_creates_ward"]` | How stewardship can be established |
| `steward_creates_ward_enabled` | Boolean | `false` | Whether stewards can create ward accounts directly |
| `max_stewards_per_ward` | Number | `5` | Maximum stewards for a single ward |
| `max_wards_per_steward` | Number | `10` | Maximum wards for a single steward |
| `invitation_expiry_days` | Number | `7` | Days before a stewardship invitation expires |
| `require_ward_consent` | Boolean | `true` | Whether the ward must accept the stewardship |
| `minor_age_threshold` | Number | `18` | Age below which a user is considered a minor |
| `allow_minor_login` | Boolean | `false` | Whether minors can authenticate directly |

The default configuration (from `.node.yaml`) ships with stewardship disabled. When enabled, only `PARENT_OF` and `GUARDIAN_OF` relations grant stewardship by default, and all three establishment workflows are available.

### Stewardship Overrides

For situations that do not fit neatly into relation-based stewardship, the `raisin:StewardshipOverride` node type provides explicit, time-bounded delegation:

| Property | Type | Required | Description |
|----------|------|:--------:|-------------|
| `steward_id` | String | yes | User ID of the steward |
| `ward_id` | String | yes | User ID of the ward |
| `delegation_mode` | String | yes | `"full"` (all permissions) or `"scoped"` (limited) |
| `scoped_permissions` | Array | no | Permission grants when `delegation_mode` is `"scoped"` |
| `valid_from` | Date | no | Start of the delegation period |
| `valid_until` | Date | no | End of the delegation period |
| `status` | String | yes | One of: `pending`, `active`, `expired`, `revoked` (default: `pending`) |
| `reason` | String | no | Reason for the override |

Overrides enable scenarios like temporary guardianship (e.g., a caretaker for a week) or scoped delegation (e.g., a colleague can manage only a specific content area).

### Package Functions and Triggers

The `raisin-stewardship` package provides these functions for querying stewardship relationships:

- `is-steward-of` -- checks whether one user is a steward of another
- `get-stewards` -- returns all stewards for a given ward
- `get-wards` -- returns all wards for a given steward
- Handlers for processing invitations and requests

Two triggers automate stewardship lifecycle events:

- `process-ward-invitation` -- handles the invitation workflow when a steward invites a ward
- `process-stewardship-request` -- handles the request workflow when a ward (or their representative) requests a steward

### AuthContext and Audit Logging

The `AuthContext` carries stewardship state at runtime:

```rust
pub acting_as_ward: Option<String>,           // Ward's user ID
pub active_stewardship_source: Option<String>, // Relation type or override ID
```

When a steward acts on behalf of a ward:

- `auth.user_id` remains the steward's identity
- `auth.acting_as_ward` is set to the ward's user ID
- `auth.active_stewardship_source` identifies the relation type (e.g., `"PARENT_OF"`) or override ID that grants the stewardship
- Audit logging captures both: `"{steward_id}:acting_as:{ward_id}"`
- REL conditions can check `auth.acting_as_ward` to apply stewardship-specific rules

The `raisin:User` node type includes `birth_date` for minor status calculation and `can_login` to control whether guardian-created accounts can authenticate directly.

## Impersonation

Admin users can impersonate regular users to test permissions without switching accounts:

```rust
let ctx = AuthContext::impersonated("target_user", "admin_user");
```

When impersonation is active:

- The impersonated user's permissions are resolved and enforced
- `auth.impersonated_by` is set to the admin's identity
- Audit logging captures both identities: `"{admin_id}:impersonating:{target_id}"`
- The `TokenType::Impersonation` variant in the JWT tracks the admin and target

Impersonation tokens are a distinct JWT token type, ensuring they can be identified and audited separately from regular access tokens.

## Caching Strategy

### Lean JWT Design

RaisinDB uses intentionally small JWTs. The `AuthClaims` (in `raisin-models/src/auth/claims.rs`) contain only:

- `sub` -- identity ID
- `email` -- user's email
- `tenant_id` -- tenant scope
- `sid` -- session ID
- `auth_strategy` -- how the user authenticated
- `auth_time` -- when authentication happened (for sudo mode)
- `global_flags` -- tenant-wide flags (is_tenant_admin, email_verified)
- `home` -- user's home path (for fast path-based access without DB lookup)
- Standard JWT fields (exp, iat, jti, etc.)

Workspace-specific permissions are **not** stored in the JWT. This avoids:

- Token size bloat (>4KB with many workspaces)
- Stale permissions until token refresh
- HTTP header size limit issues

### Hot LRU Cache

Instead, permissions are resolved on first access and cached in an LRU cache (in `raisin-auth/src/cache/permission_cache.rs`):

- **Cache key**: `(session_id, workspace_id)` tuple
- **TTL**: 5 minutes (configurable)
- **Eviction**: LRU when capacity is exceeded
- **Thread safety**: `tokio::sync::RwLock` for concurrent read access

The cache implements a cache-aside pattern:

1. Check cache for `(session_id, workspace_id)`
2. If found and not expired, return cached permissions
3. If miss or expired, call the resolver function (queries `raisin:access_control`)
4. Store result in cache, return

### Cache Invalidation

The cache provides targeted invalidation:

- `invalidate_session(session_id)` -- on logout, removes all entries for a session
- `invalidate_workspace(workspace_id)` -- on permission changes, removes all entries for a workspace
- `clear()` -- full cache flush

Permission changes (role updates, group membership changes) trigger workspace-level invalidation via the EventBus, ensuring cached permissions stay consistent with a maximum staleness of the TTL window.

### Capacity Planning

The cache capacity should be set based on:

```
capacity = expected_active_sessions * avg_workspaces_per_user * 1.5
```

For example, 1000 active sessions with an average of 3 workspaces each: `capacity = 1000 * 3 * 1.5 = 4500`.

## Graph-Accelerated ACL

Because RaisinDB stores authorization data as nodes and relations in a content graph, it can leverage graph algorithms to precompute access paths. The `raisin:GraphAlgorithmConfig` node type configures which algorithms run and how they are scoped.

### GraphAlgorithmConfig

| Property | Type | Required | Description |
|----------|------|:--------:|-------------|
| `algorithm` | String | yes | Algorithm name: `pagerank`, `louvain`, `connected_components`, `betweenness_centrality`, `triangle_count`, or `relates_cache` |
| `enabled` | Boolean | yes | Whether this algorithm is active (default: `true`) |
| `target` | Object | yes | Branch/revision targeting. `target.mode` is one of `"branch"`, `"all_branches"`, `"revision"`, or `"branch_pattern"` |
| `scope` | Object | yes | Node scoping: which paths, node types, workspaces, and relation types to include |
| `config` | Object | no | Algorithm-specific parameters (e.g., `max_depth` and `cache_scope` for `relates_cache`) |
| `refresh` | Object | no | Refresh trigger configuration: `ttl_seconds`, `on_branch_change`, `on_relation_change`, `cron` |

Several algorithms are available for different graph analytics use cases, but the one directly relevant to access control is `relates_cache`.

### The relates_cache Algorithm

REL conditions often use `RELATES` expressions to check whether the current user is connected to the target node through a chain of relations (e.g., "user is a member of a group that owns this folder"). Evaluating these paths at query time requires graph traversals that can become expensive as the graph grows.

The `relates_cache` algorithm precomputes these relation paths and stores the results, turning a runtime graph traversal into a cache lookup. This is especially valuable for:

- Group membership chains (user -> group -> parent group -> role)
- Stewardship paths (steward -> relation -> ward)
- Organizational hierarchies (user -> department -> division -> org)

The `config` object for `relates_cache` accepts:

- `max_depth` -- maximum traversal depth for relation paths
- `cache_scope` -- which portion of the graph to cache

The `refresh` object controls when the cache is rebuilt:

- `ttl_seconds` -- time-based expiration
- `on_branch_change` -- rebuild when the target branch changes
- `on_relation_change` -- rebuild when relations in scope are modified
- `cron` -- rebuild on a cron schedule

```json
{
  "node_type": "raisin:GraphAlgorithmConfig",
  "name": "acl-relates-cache",
  "properties": {
    "algorithm": "relates_cache",
    "enabled": true,
    "target": {
      "mode": "branch",
      "branch": "main"
    },
    "scope": {
      "workspaces": ["raisin:access_control"],
      "relation_types": ["MEMBER_OF", "PARENT_OF", "GUARDIAN_OF"],
      "node_types": ["raisin:User", "raisin:Group", "raisin:Role"]
    },
    "config": {
      "max_depth": 5,
      "cache_scope": "workspace"
    },
    "refresh": {
      "ttl_seconds": 300,
      "on_relation_change": true,
      "on_branch_change": false
    }
  }
}
```

This configuration precomputes relation paths within the `raisin:access_control` workspace on the `main` branch, covering membership, parental, and guardian relations up to 5 hops deep. The cache refreshes every 5 minutes or immediately when a relevant relation changes.

The precomputed cache integrates with the RLS filter: when a REL condition references a `RELATES` expression, the filter checks the cache first. If the cache has a precomputed answer, the graph traversal is skipped entirely. If the cache misses (e.g., for a relation type not in scope), the filter falls back to a live traversal.

## Access Settings

Each workspace can configure its access control policies through `AccessSettings` (in `raisin-models/src/auth/access.rs`):

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `allow_access_requests` | bool | `true` | Whether users can request access |
| `require_approval` | bool | `true` | Whether requests need admin approval |
| `allow_invitations` | bool | `true` | Whether admins can send invitations |
| `default_roles` | Vec\<String\> | `["viewer"]` | Roles assigned on auto-approve |
| `max_pending_requests` | u32 | 100 | Maximum pending requests per workspace |
| `invitation_expiry_days` | u32 | 7 | Days before invitations expire |

When `require_approval` is `false`, access requests are auto-approved with the `default_roles`.

## Security Configuration

While `AccessSettings` controls workspace access workflows (requests, invitations, approvals), the `raisin:SecurityConfig` node type governs the security posture of the authorization system itself -- default policies, anonymous access, and per-interface overrides.

### raisin:SecurityConfig

| Property | Type | Required | Default | Description |
|----------|------|:--------:|---------|-------------|
| `workspace` | String (unique) | yes | -- | Workspace pattern this config applies to (`*` for global default) |
| `default_policy` | String | yes | `"deny"` | Default access policy when no permission matches: `"deny"` or `"allow"` |
| `anonymous_enabled` | Boolean | no | `false` | Whether anonymous (unauthenticated) access is allowed |
| `anonymous_role` | String | no | -- | Role ID to use for anonymous users |
| `interfaces` | Object | no | -- | Per-interface configuration overrides (`rest`, `pgwire`, `websocket`) |

The node type is versionable, publishable, and auditable -- security policy changes go through the same draft/publish workflow as other content and leave an audit trail.

### Resolution Order

When the authorization system needs to determine the security configuration for a given workspace, it resolves settings in this order:

```
1. Specific workspace match    (workspace = "content")
2. Global default              (workspace = "*")
3. Hardcoded deny              (built-in fallback)
```

The most specific match wins. If a `raisin:SecurityConfig` node exists with `workspace` set to the exact workspace name, that configuration is used. Otherwise, the global default (`workspace = "*"`) applies. If no configuration node exists at all, the system falls back to a hardcoded deny-all policy -- ensuring that a misconfiguration never results in open access.

### Initial State

The default `raisin:SecurityConfig` node ships in the `raisin:access_control` workspace at `/config/default`:

- `workspace`: `"*"` (applies to all workspaces)
- `default_policy`: `"deny"`
- `anonymous_enabled`: `false`

This means that out of the box, unauthenticated requests are rejected and any request that does not match an explicit permission is denied.

### Per-Interface Overrides

The `interfaces` object allows different security settings for each transport layer. This is useful when, for example, the REST API should allow anonymous read access for a public website, but the PGWire interface (used by internal analytics tools) should require authentication:

```json
{
  "node_type": "raisin:SecurityConfig",
  "name": "content-security",
  "properties": {
    "workspace": "content",
    "default_policy": "deny",
    "anonymous_enabled": true,
    "anonymous_role": "anonymous",
    "interfaces": {
      "rest": {
        "anonymous_enabled": true,
        "anonymous_role": "anonymous"
      },
      "pgwire": {
        "anonymous_enabled": false
      },
      "websocket": {
        "anonymous_enabled": false
      }
    }
  }
}
```

In this example, the `content` workspace allows anonymous access through the REST API (using the `anonymous` role), but requires authentication for PGWire and WebSocket connections. Interface-level settings override the top-level values for that specific transport.

## Authentication Flow

Here is the end-to-end flow from an HTTP request to a permission-checked response:

```
1. HTTP Request arrives
   └─ Authorization: Bearer <JWT>
   └─ X-Raisin-Workspace: content

2. JWT Validation (raisin-auth)
   └─ Verify signature, check expiration
   └─ Extract AuthClaims { sub, sid, tenant_id, ... }

3. AuthContext Construction
   └─ AuthContext::for_user(claims.sub)
   └─ .with_email(claims.email)

4. Permission Resolution (cached)
   └─ Cache lookup: (session_id, workspace_id)
   └─ On miss: PermissionService::resolve_for_identity_id()
      └─ Find raisin:User by identity_id in raisin:access_control
      └─ Extract direct roles and groups
      └─ Resolve group roles
      └─ Resolve role inheritance
      └─ Collect all permissions
      └─ Build ResolvedPermissions
   └─ Cache result with TTL

5. AuthContext enrichment
   └─ .with_permissions(resolved)
   └─ Syncs roles, groups, email, local_user_id

6. Query Execution
   └─ Storage reads nodes

7. RLS Filtering
   └─ For each node: filter_node(node, auth, scope)
      └─ Find matching permission (scope + path + operation + type)
      └─ Evaluate REL condition (if present)
      └─ Apply field filtering
   └─ Return only accessible, filtered nodes

8. Response
```

For write operations, `can_perform()` or `can_create_at_path()` is called before the storage write, preventing unauthorized modifications.

## Further Reading

- [Authentication](./authentication.md) -- authentication strategies, JWT tokens, session management
- [SQL Access Control Extensions](./sql-access-control.md) -- managing roles, groups, users, and permissions via SQL
- [Access Control Practical Guide](../guides/access-control-guide.md) -- hands-on examples for setting up roles, groups, and permissions
- [Multi-Tenancy](./multi-tenancy.md) -- how tenants provide isolation boundaries
- [Workspaces](./workspaces.md) -- workspace model and workspace-scoped data
