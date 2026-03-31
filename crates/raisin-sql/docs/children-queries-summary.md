# Children Queries - Quick Reference

> **TODO**: Review and update this documentation to ensure accuracy with current implementation.

## Most Common Patterns

### 1. List Direct Children (⭐ Most Common)

```sql
-- Simple: Get all direct children
SELECT id, name, path FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY name;

-- With metadata
SELECT id, name, path, node_type, created_at FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC;

-- Filtered by status
SELECT id, name, path FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties ->> 'status' = 'published';
```

**Performance:** O(log n + k) using parent secondary index

---

### 2. List All Descendants (Recursive)

```sql
-- Get entire subtree
SELECT id, name, path, DEPTH(path) AS level FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path;

-- With depth limit (max 2 levels deep)
SELECT id, name, path FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND DEPTH(path) <= DEPTH('/content/blog/') + 2;
```

**Performance:** O(log n + k) using path prefix scan

---

### 3. Paginated Children

```sql
-- Page 1 (10 items)
SELECT id, name, path, created_at FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

-- Get total count
SELECT COUNT(*) FROM nodes
WHERE PARENT(path) = '/content/blog';
```

---

### 4. Siblings

```sql
-- Get siblings of a node
SELECT id, name, path FROM nodes
WHERE PARENT(path) = PARENT('/content/blog/article-1')
AND path != '/content/blog/article-1';
```

---

### 5. Tree with Indentation

```sql
SELECT
    id,
    name,
    path,
    DEPTH(path) - DEPTH('/content/') AS indent_level
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
AND DEPTH(path) <= DEPTH('/content/') + 3
ORDER BY path;
```

---

## Comparison Table

| Pattern | Use Case | Index Used | Speed |
|---------|----------|-----------|-------|
| `PARENT(path) = '/foo'` | Direct children | Secondary (parent) | ⚡ Fastest |
| `PATH_STARTS_WITH(path, '/foo/')` | Subtree | Primary (path) | ⚡ Fastest |
| `DEPTH(path) = N` | Specific level | None | 🐌 Slow (scan) |
| Self-join with PARENT | Need parent data | Both | ⚡ Fast |

---

## When to Use What

| Scenario | Query Pattern |
|----------|---------------|
| **File browser - show folder contents** | `PARENT(path) = '/path'` |
| **Export entire folder** | `PATH_STARTS_WITH(path, '/path/')` |
| **Tree widget with depth limit** | `PATH_STARTS_WITH() + DEPTH() <=` |
| **Find files (no subfolders)** | `PARENT(path) = '/path' AND node_type = 'File'` |
| **Breadcrumb navigation** | Multiple queries or app-level path parsing |
| **Search within folder** | `PATH_STARTS_WITH() + JSON filters` |

---

## Examples: 30 Working Queries

See `tests/sql/08_list_children.sql` for complete examples:
- ✅ Direct children queries (7 variants)
- ✅ Recursive descendants (3 variants)
- ✅ Pagination patterns (3 examples)
- ✅ Property filtering (4 examples)
- ✅ Siblings and ancestry (3 examples)
- ✅ Tree structure queries (4 examples)
- ✅ Advanced patterns (6 examples)

**All 122 SQL statements parse successfully!**

---

## Design Document

For detailed performance analysis, index usage, and best practices, see:
📖 `docs/list-children-design.md`
