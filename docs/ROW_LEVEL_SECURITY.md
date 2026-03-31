# Row-Level Security (RLS) Implementation

This document describes the Row-Level Security implementation in RaisinDB, including architecture, data model, and testing.

## Overview

RaisinDB implements Row-Level Security (RLS) at the storage layer to ensure consistent data access control across all interfaces (REST API, SQL, WebSocket, pgwire). The implementation follows a role-based access control (RBAC) model with support for:

- **Direct role assignment** to users
- **Group-based roles** with role inheritance
- **Path-based permissions** with glob patterns
- **Field-level filtering** (whitelist/blacklist)
- **Row-level conditions** with auth variable substitution
- **Admin impersonation** for testing

## Architecture

### Single Enforcement Point

All data access flows through the storage layer, ensuring RLS is enforced regardless of the access method:

```
┌─────────────────────────────────────────────────────────────┐
│                     Access Interfaces                        │
├──────────┬──────────┬──────────┬──────────┬─────────────────┤
│ REST API │   SQL    │ WebSocket│  pgwire  │ JS Functions    │
└────┬─────┴────┬─────┴────┬─────┴────┬─────┴────────┬────────┘
     │          │          │          │              │
     └──────────┴──────────┴──────────┴──────────────┘
                           │
                    ┌──────▼──────┐
                    │ NodeService │
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │  RLS Filter │  ◄── AuthContext
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │   Storage   │
                    │ (RocksDB)   │
                    └─────────────┘
```

### Key Components

| Component | Location | Responsibility |
|-----------|----------|----------------|
| `PermissionService` | `crates/raisin-core/src/services/permission_service.rs` | Resolves user permissions from User → Groups → Roles → Inheritance |
| `CachedPermissionService` | Same file | Caches resolved permissions (5-minute TTL) |
| `RLS Filter` | `crates/raisin-core/src/services/rls_filter.rs` | Filters nodes/operations based on permissions |
| `AuthContext` | `crates/raisin-models/src/auth/context.rs` | Carries user identity and resolved permissions |
| `Permission Model` | `crates/raisin-models/src/permissions/` | Data structures for permissions, conditions |

## Data Model

### Access Control Workspace

Security entities are stored in the `raisin:access_control` workspace:

```
/raisin:access_control
├── /users
│   ├── /alice     (raisin:User)
│   └── /bob       (raisin:User)
├── /groups
│   ├── /editors   (raisin:Group)
│   └── /admins    (raisin:Group)
└── /roles
    ├── /reader    (raisin:Role)
    ├── /editor    (raisin:Role)
    └── /admin     (raisin:Role)
```

### User Node (`raisin:User`)

```json
{
  "node_type": "raisin:User",
  "properties": {
    "email": "alice@example.com",
    "roles": ["editor"],
    "groups": ["marketing_team"]
  }
}
```

### Group Node (`raisin:Group`)

```json
{
  "node_type": "raisin:Group",
  "properties": {
    "name": "marketing_team",
    "roles": ["content_creator", "asset_viewer"]
  }
}
```

### Role Node (`raisin:Role`)

```json
{
  "node_type": "raisin:Role",
  "properties": {
    "role_id": "editor",
    "name": "Content Editor",
    "inherits": ["viewer"],
    "permissions": [
      {
        "path": "content.**",
        "operations": ["read", "create", "update"],
        "node_types": ["my:Article"],
        "except_fields": ["internal_notes"],
        "conditions": [
          {
            "property_equals": {
              "key": "department",
              "value": "$auth.department"
            }
          }
        ]
      }
    ]
  }
}
```

## Permission Resolution Algorithm

```rust
// Pseudocode for permission resolution
fn resolve_permissions(user_id: &str) -> ResolvedPermissions {
    // 1. Load user node
    let user = load_user(user_id);

    // 2. Collect direct roles
    let mut all_roles: HashSet<String> = user.roles.into_iter().collect();

    // 3. Collect group roles
    for group_id in &user.groups {
        let group = load_group(group_id);
        all_roles.extend(group.roles);
    }

    // 4. Resolve role inheritance (with cycle detection)
    let mut visited: HashSet<String> = HashSet::new();
    let mut to_process: Vec<String> = all_roles.iter().cloned().collect();

    while let Some(role_id) = to_process.pop() {
        if visited.contains(&role_id) {
            continue; // Cycle detection
        }
        visited.insert(role_id.clone());

        if let Some(role) = load_role(&role_id) {
            for inherited in &role.inherits {
                if all_roles.insert(inherited.clone()) {
                    to_process.push(inherited.clone());
                }
            }
        }
    }

    // 5. Collect all permissions
    let permissions: Vec<Permission> = all_roles
        .iter()
        .flat_map(|role_id| load_role(role_id).permissions)
        .collect();

    ResolvedPermissions {
        user_id,
        effective_roles: all_roles,
        permissions,
        is_system_admin: all_roles.contains("system_admin"),
    }
}
```

## RLS Filter Logic

### Node Filtering

```rust
fn filter_node(node: Node, auth: &AuthContext) -> Option<Node> {
    // System context bypasses all checks
    if auth.is_system {
        return Some(node);
    }

    // System admin role bypasses all checks
    if auth.permissions()?.is_system_admin {
        return Some(node);
    }

    // Find matching permission by path (most specific wins)
    let permission = find_matching_permission(&node, &auth.permissions()?.permissions)?;

    // Check READ operation is allowed
    if !permission.operations.contains(&Operation::Read) {
        return None;
    }

    // Evaluate conditions
    if let Some(conditions) = &permission.conditions {
        if !evaluate_conditions(conditions, &node, auth) {
            return None;
        }
    }

    // Apply field filtering
    let filtered = apply_field_filter(node, permission);
    Some(filtered)
}
```

### Path Pattern Matching

| Pattern | Path | Matches? |
|---------|------|----------|
| `content.articles.**` | `/content/articles/news` | ✅ |
| `content.articles.**` | `/content/articles/2024/post` | ✅ |
| `content.articles.*` | `/content/articles/news` | ✅ |
| `content.articles.*` | `/content/articles/2024/post` | ❌ |
| `content.*.drafts` | `/content/blog/drafts` | ✅ |
| `**` | `/anything/at/all` | ✅ |

### Condition Evaluation

```rust
fn evaluate_condition(condition: &RoleCondition, node: &Node, auth: &AuthContext) -> bool {
    match condition {
        RoleCondition::PropertyEquals { key, value } => {
            let actual = node.properties.get(key);
            let expected = resolve_value(value, auth); // "$auth.user_id" → actual user_id
            actual == expected
        }
        RoleCondition::PropertyIn { key, values } => {
            let actual = node.properties.get(key)?;
            values.iter()
                .map(|v| resolve_value(v, auth))
                .any(|expected| actual == expected)
        }
        RoleCondition::All(conditions) => {
            conditions.iter().all(|c| evaluate_condition(c, node, auth))
        }
        RoleCondition::Any(conditions) => {
            conditions.iter().any(|c| evaluate_condition(c, node, auth))
        }
        // ... other condition types
    }
}
```

## Test Coverage

### Permission Resolution Tests (`permission_resolution_tests.rs`)

| Test | Description |
|------|-------------|
| `test_direct_role_resolution` | User with direct roles gets correct permissions |
| `test_group_role_aggregation` | User inherits roles from groups |
| `test_role_inheritance_chain` | Role A → B → C → D chain resolves correctly |
| `test_role_inheritance_cycle_detection` | Circular inheritance doesn't infinite loop |
| `test_role_deduplication` | Same role from multiple sources is deduplicated |

### RLS Integration Tests (`rls_integration_tests.rs`)

| Test | Security Scenario |
|------|-------------------|
| `test_user_can_only_read_own_articles` | Ownership condition (`author = $auth.user_id`) |
| `test_admin_bypasses_rls` | System admin bypasses all checks |
| `test_field_filtering_applied` | `except_fields` removes sensitive data |
| `test_create_permission_enforced` | CREATE operation enforcement |
| `test_delete_permission_enforced` | DELETE operation enforcement |
| `test_cross_user_data_isolation` | User A cannot see User B's data |
| `test_role_based_visibility` | Different roles see different content |
| `test_path_based_workspace_isolation` | Department isolation |
| `test_anonymous_vs_authenticated_isolation` | Subscription tier isolation |
| `test_combined_conditions_isolation` | Multiple conditions (AND/OR) |

### Running Tests

```bash
# Run all RLS tests
cargo test --package raisin-rocksdb --test permission_resolution_tests --test rls_integration_tests

# Run with output
cargo test --package raisin-rocksdb --test rls_integration_tests -- --nocapture
```

## Performance Considerations

### Permission Caching

Resolved permissions are cached per user with 5-minute TTL:

```rust
let cache = CachedPermissionService::new(storage.clone(), Duration::from_secs(300));

// First call: computes and caches
let perms = cache.resolve_for_user_id(tenant, repo, branch, "user123").await?;

// Subsequent calls: returns from cache
let perms = cache.resolve_for_user_id(tenant, repo, branch, "user123").await?;

// Invalidate on user/role/group changes
cache.invalidate_user(tenant, repo, branch, "user123");
```

### Cache Invalidation

Invalidate cache when:
- User's roles or groups change
- Group's roles change
- Role's permissions or inheritance change

```rust
// Invalidate single user
cache.invalidate_user(tenant, repo, branch, user_id);

// Invalidate all users (after role change)
cache.invalidate_branch(tenant, repo, branch);
```

## Admin Impersonation

Admin users with `can_impersonate` flag can view content as another user:

```rust
// HTTP header
X-Raisin-Impersonate: user_123

// Backend handling
if headers.contains("X-Raisin-Impersonate") {
    if admin_user.access_flags.can_impersonate {
        let target_user = headers.get("X-Raisin-Impersonate");
        let auth = permission_service.resolve_for_user(target_user).await?;
        auth.impersonated_by = Some(admin_user.id);
        return Ok(auth);
    }
}
```

## Files Reference

### Core Implementation

| File | Description |
|------|-------------|
| `crates/raisin-models/src/auth/context.rs` | AuthContext struct |
| `crates/raisin-models/src/permissions/mod.rs` | Permission, RoleCondition, Operation |
| `crates/raisin-core/src/services/permission_service.rs` | Permission resolution |
| `crates/raisin-core/src/services/permission_cache.rs` | Caching layer |
| `crates/raisin-core/src/services/rls_filter.rs` | Node/operation filtering |

### Tests

| File | Description |
|------|-------------|
| `crates/raisin-rocksdb/tests/permission_resolution_tests.rs` | Resolution algorithm tests |
| `crates/raisin-rocksdb/tests/rls_integration_tests.rs` | Full RLS pipeline tests |
| `crates/raisin-core/src/services/rls_filter.rs` (mod tests) | Unit tests for filtering |

### Node Types

| File | Description |
|------|-------------|
| `crates/raisin-core/global_nodetypes/raisin_user.yaml` | User node type |
| `crates/raisin-core/global_nodetypes/raisin_group.yaml` | Group node type |
| `crates/raisin-core/global_nodetypes/raisin_role.yaml` | Role node type |

## Security Guarantees

1. **Single enforcement point** - All data access goes through RLS filter at storage layer
2. **No SQL bypass** - SQL queries are filtered during execution
3. **Consistent across interfaces** - REST, SQL, WebSocket, pgwire all enforce same rules
4. **Default deny** - No permission = no access
5. **Cycle-safe inheritance** - Role inheritance handles cycles gracefully
6. **Audit-ready** - All operations logged with user context

## Future Enhancements

- [ ] Property indexing for condition evaluation optimization
- [ ] Row-level security hints in SQL query planner
- [ ] Permission inheritance visualization in admin console
- [ ] Dynamic permission conditions (time-based, location-based)
- [ ] Fine-grained cache invalidation (per-role, per-permission)
