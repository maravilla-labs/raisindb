-- Pagination Patterns in RaisinDB
-- Comparing traditional Postgres approaches with RaisinDB optimizations

-- ============================================================================
-- TRADITIONAL OFFSET-BASED PAGINATION (Standard Postgres Pattern)
-- ============================================================================

-- Page 1: First 10 items
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

-- Page 2: Next 10 items
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 10;

-- Page 3: Items 21-30
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 20;

-- Generic pagination formula: OFFSET = (page_number - 1) * page_size

-- Get total count for pagination UI (separate query)
SELECT COUNT(*) AS total_count
FROM nodes
WHERE PARENT(path) = '/content/blog';

-- Combined: Get data + total count
-- Note: Most efficient to run these as separate queries
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

SELECT COUNT(*) AS total_count
FROM nodes
WHERE PARENT(path) = '/content/blog';

-- ============================================================================
-- CURSOR-BASED PAGINATION (Better for large datasets)
-- ============================================================================

-- Traditional Postgres cursor pattern adapted for RaisinDB

-- Page 1: First 10 items (no cursor needed)
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC, id DESC
LIMIT 10;

-- Page 2: Using last item from Page 1 as cursor
-- Assume last item had created_at='2025-01-15T10:00:00Z', id='node-123'
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND (
    created_at < '2025-01-15T10:00:00Z'
    OR (created_at = '2025-01-15T10:00:00Z' AND id < 'node-123')
)
ORDER BY created_at DESC, id DESC
LIMIT 10;

-- Page 3: Using last item from Page 2 as cursor
-- Assume last item had created_at='2025-01-10T08:30:00Z', id='node-456'
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND (
    created_at < '2025-01-10T08:30:00Z'
    OR (created_at = '2025-01-10T08:30:00Z' AND id < 'node-456')
)
ORDER BY created_at DESC, id DESC
LIMIT 10;

-- Cursor pagination with ascending order
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND (
    created_at > '2025-01-10T08:30:00Z'
    OR (created_at = '2025-01-10T08:30:00Z' AND id > 'node-456')
)
ORDER BY created_at ASC, id ASC
LIMIT 10;

-- ============================================================================
-- RAISINDB-SPECIFIC: PATH-BASED PAGINATION
-- ============================================================================

-- RaisinDB optimization: Use path as natural cursor
-- Paths are lexicographically ordered and unique

-- Page 1: First 10 nodes in subtree
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path
LIMIT 10;

-- Page 2: Continue from last path
-- Assume last path was '/content/blog/2025/article-010'
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND path > '/content/blog/2025/article-010'
ORDER BY path
LIMIT 10;

-- Page 3: Continue from next cursor
-- Assume last path was '/content/blog/2025/article-020'
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND path > '/content/blog/2025/article-020'
ORDER BY path
LIMIT 10;

-- Reverse pagination (previous page)
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND path < '/content/blog/2025/article-010'
ORDER BY path DESC
LIMIT 10;

-- ============================================================================
-- PAGINATION WITH FILTERS
-- ============================================================================

-- Offset pagination with filters
SELECT id, name, path, properties ->> 'status' AS status
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

-- Cursor pagination with filters
SELECT id, name, path, properties ->> 'status' AS status, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties ->> 'status' = 'published'
AND created_at < '2025-01-15T10:00:00Z'
ORDER BY created_at DESC
LIMIT 10;

-- Path-based pagination with filters
SELECT id, name, path, node_type
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND node_type = 'my:Article'
AND path > '/content/blog/2025/article-010'
ORDER BY path
LIMIT 10;

-- ============================================================================
-- PAGINATION WITH JSON PROPERTY SORTING
-- ============================================================================

-- Sort by JSON property (e.g., title)
SELECT
    id,
    name,
    path,
    properties ->> 'title' AS title,
    properties ->> 'views' AS views
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY properties ->> 'title'
LIMIT 10 OFFSET 0;

-- Sort by numeric JSON property (e.g., view count)
SELECT
    id,
    name,
    path,
    JSON_VALUE(properties, '$.views' RETURNING DOUBLE) AS views
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY JSON_VALUE(properties, '$.views' RETURNING DOUBLE) DESC
LIMIT 10;

-- Cursor-based with JSON sorting
SELECT
    id,
    name,
    path,
    JSON_VALUE(properties, '$.views' RETURNING DOUBLE) AS views
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND JSON_VALUE(properties, '$.views' RETURNING DOUBLE) < 1000
ORDER BY JSON_VALUE(properties, '$.views' RETURNING DOUBLE) DESC
LIMIT 10;

-- ============================================================================
-- DEEP PAGINATION OPTIMIZATION
-- ============================================================================

-- Problem: OFFSET becomes slow for large offsets
-- Example: Page 100 (offset 1000) is slow
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 1000;  -- ❌ Slow for large offsets

-- Solution 1: Use cursor-based pagination (recommended)
-- Already shown above

-- Solution 2: Use keyset pagination (ID-based cursor)
-- Requires knowing the last ID from previous page
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND id > 'last-id-from-previous-page'
ORDER BY id
LIMIT 10;

-- Solution 3: For RaisinDB, use path-based iteration
-- Most efficient for hierarchical queries
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND path > 'last-path-from-previous-page'
ORDER BY path
LIMIT 10;

-- ============================================================================
-- PAGINATION METADATA QUERY
-- ============================================================================

-- Get pagination metadata in one query
SELECT
    COUNT(*) AS total_count,
    COUNT(*) / 10 + 1 AS total_pages,
    MIN(created_at) AS oldest,
    MAX(created_at) AS newest
FROM nodes
WHERE PARENT(path) = '/content/blog';

-- With filter applied
SELECT
    COUNT(*) AS total_count,
    COUNT(*) / 10 + 1 AS total_pages
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties ->> 'status' = 'published';

-- ============================================================================
-- BIDIRECTIONAL PAGINATION (Next & Previous)
-- ============================================================================

-- Get next page (forward)
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND created_at < '2025-01-15T10:00:00Z'
ORDER BY created_at DESC
LIMIT 10;

-- Get previous page (backward)
-- Need to reverse the query
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND created_at > '2025-01-15T10:00:00Z'
ORDER BY created_at ASC
LIMIT 10;

-- Then reverse results in application code

-- ============================================================================
-- INFINITE SCROLL PATTERN
-- ============================================================================

-- Load more pattern (append to existing results)
-- Same as cursor pagination, just keep appending

-- Initial load: 20 items
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 20;

-- Load more: Next 20 items
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND created_at < '2025-01-15T10:00:00Z'
ORDER BY created_at DESC
LIMIT 20;

-- Continue loading
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND created_at < '2025-01-10T08:00:00Z'
ORDER BY created_at DESC
LIMIT 20;

-- ============================================================================
-- RAISINDB BEST PRACTICES SUMMARY
-- ============================================================================

-- ✅ RECOMMENDED: Path-based cursor for hierarchical queries
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND path > :cursor_path
ORDER BY path
LIMIT :page_size;

-- ✅ RECOMMENDED: Timestamp + ID cursor for time-ordered queries
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND (
    created_at < :cursor_time
    OR (created_at = :cursor_time AND id < :cursor_id)
)
ORDER BY created_at DESC, id DESC
LIMIT :page_size;

-- ✅ ACCEPTABLE: OFFSET for small datasets or first few pages
SELECT id, name, path
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

-- ❌ AVOID: Large OFFSET values (> 100)
SELECT id, name, path
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 1000;  -- Slow!

-- ============================================================================
-- COMPARISON: Postgres vs RaisinDB Patterns
-- ============================================================================

-- Postgres Traditional: Window functions for row numbers
-- Note: RaisinDB may not support window functions yet
-- This would be: SELECT *, ROW_NUMBER() OVER (ORDER BY created_at DESC) AS rn

-- Postgres: Fetch with OFFSET and total in one query
-- This requires window functions or subqueries
-- RaisinDB: Run count and data queries separately

-- Both support:
-- ✅ LIMIT/OFFSET
-- ✅ Cursor-based pagination with WHERE clauses
-- ✅ ORDER BY with multiple columns

-- RaisinDB specific:
-- ✅ PATH_STARTS_WITH for efficient hierarchical pagination
-- ✅ PARENT() for direct children pagination
-- ✅ Path-based cursors leveraging natural tree order
