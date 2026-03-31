---
sidebar_position: 3
---

# Condition Reference

Complete reference for RLS condition syntax.

## Condition Types

### property_equals

Checks if a node property exactly matches a value.

```yaml
conditions:
  - property_equals:
      key: "status"           # Property name
      value: "published"      # Literal value

  - property_equals:
      key: "author"
      value: "$auth.user_id"  # Auth variable
```

**Supported value types:**
- String: `"published"`
- Number: `42`, `3.14`
- Boolean: `true`, `false`
- Auth variable: `"$auth.user_id"`, `"$auth.email"`

### property_in

Checks if a node property matches any value in a list.

```yaml
conditions:
  - property_in:
      key: "status"
      values: ["published", "featured"]

  - property_in:
      key: "department"
      values: ["$auth.department", "shared", "public"]
```

### property_greater_than

Numeric or string comparison (greater than).

```yaml
conditions:
  - property_greater_than:
      key: "priority"
      value: 5

  - property_greater_than:
      key: "publish_date"
      value: "2024-01-01"
```

### property_less_than

Numeric or string comparison (less than).

```yaml
conditions:
  - property_less_than:
      key: "price"
      value: 100

  - property_less_than:
      key: "expires_at"
      value: "$auth.current_date"
```

### user_has_role

Checks if the current user has a specific role.

```yaml
conditions:
  - user_has_role: "premium"
  - user_has_role: "verified_author"
```

### user_in_group

Checks if the current user is in a specific group.

```yaml
conditions:
  - user_in_group: "beta_testers"
  - user_in_group: "enterprise_customers"
```

## Logical Operators

### all (AND)

All conditions must be true.

```yaml
conditions:
  - all:
    - property_equals:
        key: "author"
        value: "$auth.user_id"
    - property_equals:
        key: "status"
        value: "draft"
    - property_less_than:
        key: "revision"
        value: 10
```

### any (OR)

At least one condition must be true.

```yaml
conditions:
  - any:
    - property_equals:
        key: "visibility"
        value: "public"
    - property_equals:
        key: "author"
        value: "$auth.user_id"
    - user_has_role: "admin"
```

### Nested Logical Operators

Combine `all` and `any` for complex logic:

```yaml
# (author = me AND status = draft) OR (status = published)
conditions:
  - any:
    - all:
      - property_equals:
          key: "author"
          value: "$auth.user_id"
      - property_equals:
          key: "status"
          value: "draft"
    - property_equals:
        key: "status"
        value: "published"
```

## Auth Variables

Variables that resolve to the current user's context:

| Variable | Description | Example Value |
|----------|-------------|---------------|
| `$auth.user_id` | User's unique identifier | `"user_abc123"` |
| `$auth.email` | User's email address | `"alice@example.com"` |

**Usage:**

```yaml
conditions:
  - property_equals:
      key: "owner_id"
      value: "$auth.user_id"
```

## Type Coercion

Conditions handle type coercion automatically:

| Node Property | Condition Value | Result |
|---------------|-----------------|--------|
| `"42"` (string) | `42` (number) | No match |
| `42` (number) | `42` (number) | Match |
| `42` (number) | `42.0` (float) | Match |
| `true` (boolean) | `"true"` (string) | No match |

**Best practice:** Ensure property types match condition value types.

## Null Handling

```yaml
# Match if property is null
conditions:
  - property_equals:
      key: "deleted_at"
      value: null

# Match if property is NOT null (use property_in)
conditions:
  - any:
    - property_greater_than:
        key: "deleted_at"
        value: ""
```

## Array Properties

For array properties, use `property_in` to check if any element matches:

```yaml
# Node: { "tags": ["tech", "news", "featured"] }
# Check if "featured" is in tags
conditions:
  - property_in:
      key: "tags"
      values: ["featured"]
```

## Performance Considerations

1. **Simple conditions first** - Put most selective conditions first
2. **Avoid deeply nested logic** - Keep nesting to 2-3 levels max
3. **Index frequently-filtered properties** - Use `indexed_for_sql: true` in NodeType
4. **Cache permission resolution** - Permissions are cached per user (5-minute TTL)

## Debugging Conditions

Enable debug logging to see condition evaluation:

```bash
RUST_LOG=raisin_core::services::rls_filter=debug ./raisindb
```

Output shows which conditions pass/fail:

```
DEBUG rls_filter: Evaluating conditions for node path=/content/article-1
DEBUG rls_filter: property_equals author=$auth.user_id -> true
DEBUG rls_filter: property_equals status=published -> false
DEBUG rls_filter: all conditions -> false (node filtered out)
```

## Common Patterns

### Owner-Only Access

```yaml
conditions:
  - property_equals:
      key: "owner_id"
      value: "$auth.user_id"
```

### Published Content Only

```yaml
conditions:
  - property_equals:
      key: "status"
      value: "published"
```

### Team-Based Access

```yaml
conditions:
  - property_in:
      key: "team_id"
      values: ["$auth.team_id"]
```

### Time-Based Access

```yaml
# Only show content after publish date
conditions:
  - property_less_than:
      key: "publish_at"
      value: "$auth.current_timestamp"
```

### Role-Gated Features

```yaml
conditions:
  - any:
    - user_has_role: "premium"
    - user_has_role: "enterprise"
```
