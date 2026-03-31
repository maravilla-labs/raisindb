# SQL Access Control Extensions

RaisinDB manages access control through nodes in the `raisin:access_control` workspace -- roles, groups, users, and security policies are all first-class content. The REST API and Admin Console already provide full CRUD for these entities. This chapter introduces SQL extensions that bring the same capabilities to the SQL interface, so administrators can manage access control from `psql`, the SQL query page in the Admin Console, or any PostgreSQL-compatible client.

## Design Philosophy

Three principles guided the design of these extensions:

**Consistent with existing patterns.** RaisinDB already extends SQL with `CREATE NODETYPE`, `CREATE BRANCH`, `RELATE`, and other domain-specific statements. Access control DDL follows the same conventions: keyword-driven syntax, string-quoted identifiers, optional clauses in any order, and results returned as row sets.

**Declarative over procedural.** Administrators declare the desired state (`CREATE ROLE 'editor' WITH PERMISSIONS (...)`) rather than issuing a sequence of low-level node mutations. The engine translates this into the correct node operations in `raisin:access_control`, including validation, deduplication, and cache invalidation.

**SQL as a first-class management interface.** Every access control operation available through the REST API or Admin Console should be expressible in SQL. This enables infrastructure-as-code workflows where security policies are version-controlled SQL scripts, applied through migrations or CI/CD pipelines.

## Statement Overview

| Category | Statements |
|----------|------------|
| **Roles** | `CREATE ROLE`, `ALTER ROLE`, `DROP ROLE`, `SHOW ROLES`, `DESCRIBE ROLE` |
| **Groups** | `CREATE GROUP`, `ALTER GROUP`, `DROP GROUP`, `SHOW GROUPS`, `DESCRIBE GROUP` |
| **Users** | `CREATE USER`, `ALTER USER`, `DROP USER`, `SHOW USERS`, `DESCRIBE USER` |
| **Grants** | `GRANT`, `REVOKE` |
| **Security Policy** | `ALTER SECURITY CONFIG`, `SHOW SECURITY CONFIG` |
| **Inspection** | `SHOW PERMISSIONS FOR`, `SHOW EFFECTIVE ROLES FOR` |

All statements operate on the `raisin:access_control` workspace. They require appropriate permissions -- typically the `system_admin` role or a custom role with write access to `raisin:access_control`.

## Role Management

### CREATE ROLE

```sql
CREATE ROLE 'editor'
  DESCRIPTION 'Can manage articles in the content workspace'
  INHERITS ('viewer')
  PERMISSIONS (
    ALLOW read, update ON 'content' PATH '/articles/**'
      NODE TYPES ('blog:Article', 'blog:Draft')
      EXCEPT FIELDS (internal_notes)
      WHERE node.created_by == auth.user_id,

    ALLOW read ON '*' PATH '/media/**'
      FIELDS (title, url, thumbnail)
  );
```

The `CREATE ROLE` statement creates a `raisin:Role` node under `/roles` in the `raisin:access_control` workspace.

#### Syntax

```
CREATE ROLE 'role_id'
  [DESCRIPTION 'text']
  [INHERITS ('role_a', 'role_b', ...)]
  [PERMISSIONS (
    permission_grant [, permission_grant ...]
  )]
```

#### Permission Grant Syntax

Each permission grant follows this structure:

```
ALLOW operation [, operation ...] ON workspace_pattern PATH 'path_pattern'
  [BRANCH 'branch_pattern']
  [NODE TYPES ('type_a', 'type_b', ...)]
  [FIELDS (field_a, field_b, ...)]
  [EXCEPT FIELDS (field_x, field_y, ...)]
  [WHERE rel_expression]
```

**Operations:** `create`, `read`, `update`, `delete`, `translate`, `relate`, `unrelate`

**Workspace pattern:** A glob pattern matching workspace names. Use `'*'` for all workspaces. Omitting `ON` defaults to all workspaces.

**Path pattern:** A glob path pattern. `*` matches a single segment, `**` matches recursively, `?` matches a single character.

**Branch pattern:** Optional glob for branch filtering. Omitting defaults to all branches.

**WHERE clause:** A REL (Raisin Expression Language) condition evaluated at runtime. Available variables:

| Variable | Description |
|----------|-------------|
| `auth.user_id` | Global identity ID |
| `auth.local_user_id` | Workspace-specific user node ID |
| `auth.email` | User's email |
| `auth.home` | User's home path |
| `auth.roles` | Array of role IDs |
| `auth.groups` | Array of group IDs |
| `auth.is_anonymous` | Boolean |
| `node.id` | Node UUID |
| `node.name` | Node name |
| `node.path` | Full hierarchical path |
| `node.node_type` | Node type |
| `node.created_by` | Creator identity |
| `node.owner_id` | Owner identity |
| `node.<property>` | Any node property |

#### Examples

Minimal role with a single permission:

```sql
CREATE ROLE 'viewer'
  PERMISSIONS (
    ALLOW read ON '*' PATH '/**'
  );
```

Role with inheritance and conditional access:

```sql
CREATE ROLE 'content-author'
  DESCRIPTION 'Authors can create and edit their own articles'
  INHERITS ('viewer')
  PERMISSIONS (
    ALLOW create, read, update ON 'content' PATH '/articles/**'
      NODE TYPES ('blog:Article')
      WHERE node.created_by == auth.user_id,

    ALLOW create, read, update, delete ON 'content' PATH '/drafts/**'
      WHERE node.created_by == auth.user_id
  );
```

Role with field-level filtering:

```sql
CREATE ROLE 'public-reader'
  PERMISSIONS (
    ALLOW read ON 'content' PATH '/articles/**'
      FIELDS (title, summary, author, published_at)
      WHERE node.status == 'published'
  );
```

Role with graph-based access (social features):

```sql
CREATE ROLE 'social-user'
  PERMISSIONS (
    -- Friends can see full profile
    ALLOW read ON 'raisin:access_control' PATH '/users/**/profile'
      WHERE node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH',

    -- Friends-of-friends see limited profile
    ALLOW read ON 'raisin:access_control' PATH '/users/**/profile'
      FIELDS (display_name, avatar, bio)
      WHERE node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 2
  );
```

### ALTER ROLE

```sql
-- Add permissions
ALTER ROLE 'editor'
  ADD PERMISSION
    ALLOW delete ON 'content' PATH '/articles/**'
      WHERE node.created_by == auth.user_id;

-- Drop a permission by index (1-based)
ALTER ROLE 'editor'
  DROP PERMISSION 2;

-- Change inheritance
ALTER ROLE 'editor'
  ADD INHERITS ('reviewer');

ALTER ROLE 'editor'
  DROP INHERITS ('reviewer');

-- Update description
ALTER ROLE 'editor'
  SET DESCRIPTION 'Updated role description';
```

#### Syntax

```
ALTER ROLE 'role_id'
  ADD PERMISSION permission_grant
| DROP PERMISSION index
| ADD INHERITS ('role_a', ...)
| DROP INHERITS ('role_a', ...)
| SET DESCRIPTION 'text'
```

### DROP ROLE

```sql
DROP ROLE 'editor';
DROP ROLE IF EXISTS 'temp-reviewer';
```

Dropping a role removes it from the `raisin:access_control` workspace. Any users or groups referencing the dropped role will retain the reference, but it will have no effect during permission resolution (missing roles are silently skipped).

### SHOW ROLES

```sql
-- List all roles
SHOW ROLES;

-- Filter by pattern
SHOW ROLES LIKE 'content-%';
```

Returns: `role_id`, `name`, `description`, `inherits`, `permission_count`.

### DESCRIBE ROLE

```sql
DESCRIBE ROLE 'editor';
```

Returns the full role definition including all permissions as rows:

| Column | Type | Description |
|--------|------|-------------|
| `role_id` | TEXT | Role identifier |
| `name` | TEXT | Display name |
| `description` | TEXT | Description |
| `inherits` | TEXT[] | Inherited role IDs |
| `permission_index` | INT | 1-based permission index |
| `workspace` | TEXT | Workspace pattern |
| `branch_pattern` | TEXT | Branch pattern |
| `path` | TEXT | Path pattern |
| `operations` | TEXT[] | Allowed operations |
| `node_types` | TEXT[] | Restricted node types |
| `fields` | TEXT[] | Field whitelist |
| `except_fields` | TEXT[] | Field blacklist |
| `condition` | TEXT | REL expression |

## Group Management

### CREATE GROUP

```sql
CREATE GROUP 'engineering'
  DESCRIPTION 'All engineers across projects'
  ROLES ('developer', 'viewer');
```

Creates a `raisin:Group` node under `/groups` in `raisin:access_control`.

#### Syntax

```
CREATE GROUP 'group_id'
  [DESCRIPTION 'text']
  [ROLES ('role_a', 'role_b', ...)]
```

### ALTER GROUP

```sql
ALTER GROUP 'engineering'
  ADD ROLES ('deployer');

ALTER GROUP 'engineering'
  DROP ROLES ('viewer');

ALTER GROUP 'engineering'
  SET DESCRIPTION 'Updated description';
```

#### Syntax

```
ALTER GROUP 'group_id'
  ADD ROLES ('role_a', ...)
| DROP ROLES ('role_a', ...)
| SET DESCRIPTION 'text'
```

### DROP GROUP

```sql
DROP GROUP 'engineering';
DROP GROUP IF EXISTS 'temp-team';
```

### SHOW GROUPS

```sql
SHOW GROUPS;
SHOW GROUPS LIKE 'team-%';
```

Returns: `group_id`, `name`, `description`, `roles`, `member_count`.

### DESCRIBE GROUP

```sql
DESCRIBE GROUP 'engineering';
```

Returns the group definition and its members (users who have this group in their `groups` array).

## User Management

### CREATE USER

```sql
CREATE USER 'alice'
  EMAIL 'alice@example.com'
  DISPLAY NAME 'Alice Smith'
  ROLES ('editor', 'reviewer')
  GROUPS ('engineering', 'content-team');
```

Creates a `raisin:User` node under `/users` in `raisin:access_control` with the initial child structure (profile, inbox, outbox, sent, notifications).

#### Syntax

```
CREATE USER 'user_id'
  EMAIL 'email'
  [DISPLAY NAME 'name']
  [ROLES ('role_a', ...)]
  [GROUPS ('group_a', ...)]
  [CAN LOGIN true|false]
  [BIRTH DATE 'YYYY-MM-DD']
  [IN FOLDER '/users/subfolder']
```

The optional `IN FOLDER` clause places the user under a specific `raisin:AclFolder` rather than the default `/users` root.

### ALTER USER

```sql
ALTER USER 'alice'
  ADD ROLES ('admin');

ALTER USER 'alice'
  DROP ROLES ('reviewer');

ALTER USER 'alice'
  ADD GROUPS ('analytics-team');

ALTER USER 'alice'
  DROP GROUPS ('content-team');

ALTER USER 'alice'
  SET EMAIL 'alice.smith@example.com';

ALTER USER 'alice'
  SET DISPLAY NAME 'Alice M. Smith';

ALTER USER 'alice'
  SET CAN LOGIN false;
```

#### Syntax

```
ALTER USER 'user_id'
  ADD ROLES ('role_a', ...)
| DROP ROLES ('role_a', ...)
| ADD GROUPS ('group_a', ...)
| DROP GROUPS ('group_a', ...)
| SET EMAIL 'email'
| SET DISPLAY NAME 'name'
| SET CAN LOGIN true|false
| SET BIRTH DATE 'YYYY-MM-DD'
```

### DROP USER

```sql
DROP USER 'alice';
DROP USER IF EXISTS 'temp-user';
```

### SHOW USERS

```sql
SHOW USERS;
SHOW USERS LIKE '%@example.com';
SHOW USERS IN GROUP 'engineering';
SHOW USERS WITH ROLE 'editor';
```

Returns: `user_id`, `email`, `display_name`, `roles`, `groups`, `can_login`.

### DESCRIBE USER

```sql
DESCRIBE USER 'alice';
```

Returns the user's properties, effective roles (including inherited through groups), and effective permissions.

## GRANT / REVOKE

The `GRANT` and `REVOKE` statements provide a concise syntax for assigning and removing roles and group memberships without using `ALTER USER` or `ALTER GROUP`.

### GRANT

```sql
-- Grant roles to a user
GRANT ROLE 'editor' TO USER 'alice';
GRANT ROLES ('editor', 'reviewer') TO USER 'alice';

-- Grant roles to a group
GRANT ROLE 'deployer' TO GROUP 'engineering';
GRANT ROLES ('deployer', 'viewer') TO GROUP 'engineering';

-- Add a user to groups
GRANT GROUP 'engineering' TO USER 'alice';
GRANT GROUPS ('engineering', 'content-team') TO USER 'alice';
```

### REVOKE

```sql
-- Revoke roles from a user
REVOKE ROLE 'editor' FROM USER 'alice';
REVOKE ROLES ('editor', 'reviewer') FROM USER 'alice';

-- Revoke roles from a group
REVOKE ROLE 'deployer' FROM GROUP 'engineering';

-- Remove a user from groups
REVOKE GROUP 'engineering' FROM USER 'alice';
```

### Syntax

```
GRANT ROLE[S] ('role_a', ...) TO USER|GROUP 'id'
GRANT GROUP[S] ('group_a', ...) TO USER 'id'
REVOKE ROLE[S] ('role_a', ...) FROM USER|GROUP 'id'
REVOKE GROUP[S] ('group_a', ...) FROM USER 'id'
```

## Security Policy Configuration

### ALTER SECURITY CONFIG

```sql
-- Change the global default policy
ALTER SECURITY CONFIG '*'
  SET DEFAULT POLICY 'deny';

-- Enable anonymous access with a specific role
ALTER SECURITY CONFIG '*'
  SET ANONYMOUS ENABLED true
  SET ANONYMOUS ROLE 'anonymous';

-- Configure per-interface overrides
ALTER SECURITY CONFIG 'content'
  SET DEFAULT POLICY 'deny'
  SET ANONYMOUS ENABLED true
  SET ANONYMOUS ROLE 'public-reader'
  SET INTERFACE rest ANONYMOUS ENABLED true
  SET INTERFACE pgwire ANONYMOUS ENABLED false
  SET INTERFACE websocket ANONYMOUS ENABLED false;
```

This modifies the `raisin:SecurityConfig` node for the specified workspace pattern. If no config exists for that pattern, one is created.

#### Syntax

```
ALTER SECURITY CONFIG 'workspace_pattern'
  SET DEFAULT POLICY 'deny'|'allow'
| SET ANONYMOUS ENABLED true|false
| SET ANONYMOUS ROLE 'role_id'
| SET INTERFACE interface_name setting value
```

Interface names: `rest`, `pgwire`, `websocket`.

### SHOW SECURITY CONFIG

```sql
-- Show all security configurations
SHOW SECURITY CONFIG;

-- Show config for a specific workspace
SHOW SECURITY CONFIG FOR 'content';
```

Returns: `workspace`, `default_policy`, `anonymous_enabled`, `anonymous_role`, `interfaces`.

## Inspection Queries

These read-only statements help administrators understand the effective permission state for a user.

### SHOW PERMISSIONS FOR

```sql
-- Show effective permissions for a user
SHOW PERMISSIONS FOR USER 'alice';

-- Show permissions for a user on a specific workspace
SHOW PERMISSIONS FOR USER 'alice' ON 'content';
```

Returns: the fully resolved permission set after role inheritance, group expansion, and deduplication. Each row is one permission grant with its source (direct role, group role, inherited role).

| Column | Type | Description |
|--------|------|-------------|
| `source_type` | TEXT | `direct_role`, `group_role`, or `inherited_role` |
| `source_name` | TEXT | Role or group that contributed this permission |
| `workspace` | TEXT | Workspace pattern |
| `path` | TEXT | Path pattern |
| `operations` | TEXT[] | Allowed operations |
| `node_types` | TEXT[] | Node type filter |
| `fields` | TEXT[] | Field whitelist |
| `except_fields` | TEXT[] | Field blacklist |
| `condition` | TEXT | REL expression |

### SHOW EFFECTIVE ROLES FOR

```sql
SHOW EFFECTIVE ROLES FOR USER 'alice';
```

Returns: all roles that apply to the user after resolving group memberships and role inheritance.

| Column | Type | Description |
|--------|------|-------------|
| `role_id` | TEXT | Role identifier |
| `source` | TEXT | How the role was acquired: `direct`, `group:group_name`, or `inherited:parent_role` |

## Transaction Support

Access control statements participate in RaisinDB's transaction model. Multiple changes can be grouped into a single atomic commit:

```sql
BEGIN;

CREATE ROLE 'blog-editor'
  PERMISSIONS (
    ALLOW create, read, update ON 'content' PATH '/blog/**'
  );

CREATE GROUP 'blog-team'
  ROLES ('blog-editor');

GRANT GROUP 'blog-team' TO USER 'alice';
GRANT GROUP 'blog-team' TO USER 'bob';

COMMIT WITH MESSAGE 'Set up blog team access' ACTOR 'admin';
```

All changes are applied atomically: either all succeed or none do. The commit message appears in the version history of `raisin:access_control`.

## Putting It Together

Here is a complete example that sets up access control for a content management system:

```sql
BEGIN;

-- 1. Create a read-only role for public content
CREATE ROLE 'public-viewer'
  DESCRIPTION 'Anonymous read access to published content'
  PERMISSIONS (
    ALLOW read ON 'content' PATH '/**'
      WHERE node.status == 'published'
  );

-- 2. Create an editor role that inherits from viewer
CREATE ROLE 'content-editor'
  DESCRIPTION 'Can manage articles and media'
  INHERITS ('public-viewer')
  PERMISSIONS (
    ALLOW create, read, update, delete ON 'content' PATH '/articles/**'
      WHERE node.created_by == auth.user_id,

    ALLOW read ON 'media' PATH '/**'
  );

-- 3. Create an admin role that inherits from editor
CREATE ROLE 'content-admin'
  DESCRIPTION 'Full control over content workspaces'
  INHERITS ('content-editor')
  PERMISSIONS (
    ALLOW create, read, update, delete ON 'content' PATH '/**',
    ALLOW create, read, update, delete ON 'media' PATH '/**'
  );

-- 4. Create groups
CREATE GROUP 'editors'
  DESCRIPTION 'Content editing team'
  ROLES ('content-editor');

CREATE GROUP 'admins'
  DESCRIPTION 'Content administration team'
  ROLES ('content-admin');

-- 5. Create users and assign them to groups
CREATE USER 'alice' EMAIL 'alice@example.com' DISPLAY NAME 'Alice' GROUPS ('admins');
CREATE USER 'bob'   EMAIL 'bob@example.com'   DISPLAY NAME 'Bob'   GROUPS ('editors');

-- 6. Enable anonymous access for the public-viewer role
ALTER SECURITY CONFIG 'content'
  SET ANONYMOUS ENABLED true
  SET ANONYMOUS ROLE 'public-viewer';

COMMIT WITH MESSAGE 'Initial CMS access control setup' ACTOR 'admin';
```

After this script runs, verify the setup:

```sql
-- Check alice's effective roles
SHOW EFFECTIVE ROLES FOR USER 'alice';
-- Returns: content-admin (direct via group), content-editor (inherited),
--          public-viewer (inherited)

-- Check alice's permissions on the content workspace
SHOW PERMISSIONS FOR USER 'alice' ON 'content';

-- Check security policy
SHOW SECURITY CONFIG FOR 'content';
```

## Implementation Architecture

These SQL extensions follow the same pipeline as existing RaisinDB custom statements:

```
SQL Input
    │
    ▼
┌───────────────────────┐
│  Parser (raisin-sql)  │  New: acl_parser.rs
│  nom-based parsers    │  Parses ROLE/GROUP/USER/GRANT/REVOKE/
│  for ACL statements   │  SECURITY CONFIG syntax
└───────────┬───────────┘
            │
            ▼
┌───────────────────────┐
│  Analyzer             │  New: acl_analysis.rs
│  Semantic validation  │  Validates identifiers, checks workspace
│  + catalog lookups    │  catalog, resolves references
└───────────┬───────────┘
            │
            ▼
┌───────────────────────┐
│  Analyzed Statement   │  New variants in AnalyzedStatement enum:
│  AnalyzedStatement::  │  AclCreateRole, AclAlterRole, AclDropRole,
│  Acl(AclStatement)    │  AclGrant, AclRevoke, etc.
└───────────┬───────────┘
            │
            ▼
┌───────────────────────────────┐
│  Execution (raisin-sql-exec)  │  New: acl_executor.rs
│  Translates ACL statements    │  Performs node CRUD in the
│  to node operations in        │  raisin:access_control workspace,
│  raisin:access_control        │  validates constraints, invalidates
│                               │  the permission cache
└───────────┬───────────────────┘
            │
            ▼
┌───────────────────────────┐
│  RowStream response       │  Returns result rows
│  (PGWire / HTTP / WS)    │  (e.g., SHOW results, confirmation)
└───────────────────────────┘
```

### Key Implementation Details

**Workspace isolation.** All ACL statements implicitly target the `raisin:access_control` workspace. The executor creates and updates `raisin:Role`, `raisin:Group`, and `raisin:User` nodes using the same storage APIs as the REST layer.

**Permission cache invalidation.** After any mutation (CREATE, ALTER, DROP, GRANT, REVOKE), the executor calls `invalidate_workspace("raisin:access_control")` on the permission cache, ensuring subsequent requests pick up the changes within the cache TTL window.

**Authorization.** ACL statements are themselves subject to access control. The executor checks that the current `AuthContext` has write permissions to the `raisin:access_control` workspace before proceeding. Typically this means the user must have the `system_admin` role or a custom role that grants write access to `/roles/**`, `/groups/**`, or `/users/**` in `raisin:access_control`.

**Audit trail.** Because ACL entities are versioned nodes, every change is tracked in the commit history with the actor, timestamp, and commit message (from the enclosing `COMMIT` statement or an auto-generated message for standalone statements).

**RLS enforcement on reads.** `SHOW` and `DESCRIBE` statements respect row-level security. A non-admin user running `SHOW USERS` will only see users they have read access to.

### New Source Files

| File | Crate | Purpose |
|------|-------|---------|
| `ast/acl_parser.rs` | raisin-sql | nom parsers for ACL statement syntax |
| `ast/acl.rs` | raisin-sql | AST types (AclStatement enum, CreateRole, AlterRole, etc.) |
| `analyzer/acl_analysis.rs` | raisin-sql | Semantic validation for ACL statements |
| `engine/handlers/acl.rs` | raisin-sql-execution | Execution logic translating AST to node operations |

### AST Types

```rust
pub enum AclStatement {
    CreateRole(CreateRole),
    AlterRole(AlterRole),
    DropRole(DropRole),
    ShowRoles(ShowRoles),
    DescribeRole(DescribeRole),

    CreateGroup(CreateGroup),
    AlterGroup(AlterGroup),
    DropGroup(DropGroup),
    ShowGroups(ShowGroups),
    DescribeGroup(DescribeGroup),

    CreateUser(CreateUser),
    AlterUser(AlterUser),
    DropUser(DropUser),
    ShowUsers(ShowUsers),
    DescribeUser(DescribeUser),

    Grant(Grant),
    Revoke(Revoke),

    AlterSecurityConfig(AlterSecurityConfig),
    ShowSecurityConfig(ShowSecurityConfig),

    ShowPermissionsFor(ShowPermissionsFor),
    ShowEffectiveRolesFor(ShowEffectiveRolesFor),
}
```

The `AnalyzedStatement` enum gains one new variant:

```rust
pub enum AnalyzedStatement {
    // ... existing variants ...
    Acl(AclStatement),
}
```

## Further Reading

- [Access Control & Authorization](./access-control.md) -- the architectural reference for the permission model
- [Authentication](./authentication.md) -- how users authenticate before ACL applies
- [Access Control Practical Guide](../guides/access-control-guide.md) -- hands-on guide with REST API and Admin Console examples
- [SQL Reference](../api/sql-reference.md) -- complete SQL reference including other custom extensions
