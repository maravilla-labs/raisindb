# Pagination Quick Reference

> **TODO**: Review and update this documentation to ensure accuracy with current implementation.

## Three Methods at a Glance

### 1. 🔢 OFFSET-Based (Traditional Postgres)

**When:** Small datasets (< 1000), need page jumping, admin UIs

```sql
-- Page 1
SELECT * FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

-- Page 2
LIMIT 10 OFFSET 10;

-- Page N
LIMIT 10 OFFSET (N-1)*10;
```

✅ Simple, can jump to any page
❌ Slow for deep pagination (page 100+)

---

### 2. ⏩ CURSOR-Based (Recommended for Postgres & RaisinDB)

**When:** Large datasets, infinite scroll, APIs, time-ordered results

```sql
-- Page 1: Initial
SELECT id, name, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC, id DESC
LIMIT 10;

-- Page 2: Use last item as cursor
-- Last: {created_at: '2025-01-15T10:00:00Z', id: 'node-123'}
SELECT id, name, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND (
    created_at < '2025-01-15T10:00:00Z'
    OR (created_at = '2025-01-15T10:00:00Z' AND id < 'node-123')
)
ORDER BY created_at DESC, id DESC
LIMIT 10;
```

✅ Fast everywhere, consistent performance
❌ Can't jump to arbitrary page

---

### 3. 🌲 PATH-Based (RaisinDB-Specific)

**When:** File browsers, tree views, alphabetical order, hierarchical queries

```sql
-- Page 1
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path
LIMIT 10;

-- Page 2: Use last path as cursor
-- Last path: '/content/blog/2025/article-010'
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND path > '/content/blog/2025/article-010'
ORDER BY path
LIMIT 10;
```

✅ **Fastest** method, single-column cursor, natural tree order
❌ Only works for path-ordered results

---

## Decision Tree

```
┌─ Dataset < 1000 items?
│  └─ YES → Use OFFSET (simplest)
│
└─ NO
   ├─ Sorting by path/tree order?
   │  └─ YES → Use PATH-BASED (fastest) ⚡
   │
   └─ NO (sorting by date/property)
      └─ Use CURSOR-BASED (scalable)
```

---

## Performance Comparison

**Dataset: 100,000 nodes**

| Method | Page 1 | Page 10 | Page 100 | Page 1000 |
|--------|--------|---------|----------|-----------|
| OFFSET | 1ms | 10ms | 100ms | **1000ms** 🐌 |
| CURSOR | 1ms | 1ms | 1ms | 1ms ⚡ |
| PATH-BASED | 0.5ms | 0.5ms | 0.5ms | 0.5ms ⚡⚡ |

---

## Code Examples

### Offset-Based (TypeScript)

```typescript
async function getPage(page: number, pageSize: number) {
  const offset = (page - 1) * pageSize;

  const query = `
    SELECT * FROM nodes
    WHERE PARENT(path) = '/content/blog'
    ORDER BY created_at DESC
    LIMIT ${pageSize} OFFSET ${offset}
  `;

  return await db.query(query);
}
```

### Cursor-Based (TypeScript)

```typescript
interface Cursor {
  created_at: string;
  id: string;
}

async function getPage(cursor?: Cursor, limit = 10) {
  let query = `
    SELECT id, name, created_at
    FROM nodes
    WHERE PARENT(path) = '/content/blog'
  `;

  if (cursor) {
    query += `
      AND (
        created_at < '${cursor.created_at}'
        OR (created_at = '${cursor.created_at}' AND id < '${cursor.id}')
      )
    `;
  }

  query += `
    ORDER BY created_at DESC, id DESC
    LIMIT ${limit + 1}
  `;

  const items = await db.query(query);
  const hasMore = items.length > limit;

  if (hasMore) items.pop();

  const nextCursor = hasMore ? {
    created_at: items[items.length - 1].created_at,
    id: items[items.length - 1].id
  } : null;

  return { items, nextCursor, hasMore };
}
```

### Path-Based (TypeScript)

```typescript
async function getPage(cursorPath?: string, limit = 20) {
  let query = `
    SELECT id, name, path
    FROM nodes
    WHERE PATH_STARTS_WITH(path, '/content/blog/')
  `;

  if (cursorPath) {
    query += ` AND path > '${cursorPath}'`;
  }

  query += `
    ORDER BY path
    LIMIT ${limit + 1}
  `;

  const items = await db.query(query);
  const hasMore = items.length > limit;

  if (hasMore) items.pop();

  const nextCursor = hasMore ? items[items.length - 1].path : null;

  return { items, nextCursor, hasMore };
}
```

---

## Common Patterns

### Infinite Scroll

```sql
-- Initial load: 20 items
SELECT * FROM nodes
WHERE PARENT(path) = '/feed'
ORDER BY created_at DESC, id DESC
LIMIT 20;

-- Load more: Next 20
SELECT * FROM nodes
WHERE PARENT(path) = '/feed'
AND created_at < :last_created_at
ORDER BY created_at DESC, id DESC
LIMIT 20;
```

### Bidirectional (Next & Previous)

```sql
-- Next page
WHERE created_at < :cursor
ORDER BY created_at DESC
LIMIT 10;

-- Previous page
WHERE created_at > :cursor
ORDER BY created_at ASC  -- Reversed!
LIMIT 10;
-- Then reverse results in app
```

### With Filters

```sql
SELECT * FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties ->> 'status' = 'published'
AND created_at < :cursor
ORDER BY created_at DESC, id DESC
LIMIT 10;
```

---

## Best Practices

### ✅ DO

- Use cursor-based for APIs and large datasets
- Use path-based for file browsers
- Include tiebreaker in ORDER BY: `ORDER BY created_at DESC, id DESC`
- Fetch `LIMIT + 1` to detect `has_more`
- Encode cursors (Base64)
- Cache total counts

### ❌ DON'T

- Use OFFSET > 100
- Use LIKE for paths (use `PATH_STARTS_WITH`)
- Paginate in application code
- Use unstable sort without tiebreaker
- Mix pagination methods in same endpoint

---

## See Also

📖 **Full Documentation:** `docs/pagination-design.md`
📝 **34 SQL Examples:** `tests/sql/09_pagination.sql`
🌲 **Children Queries:** `docs/list-children-design.md`
