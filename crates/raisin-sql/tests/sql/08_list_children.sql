-- List Children Operations
-- Common patterns for querying child nodes in the hierarchical structure

-- ============================================================================
-- DIRECT CHILDREN: Get immediate children of a node
-- ============================================================================

-- Method 1: Using PARENT() function (RECOMMENDED)
-- Get direct children of '/content/blog'
SELECT id, name, path
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY name;

-- Method 2: Using PATH_STARTS_WITH + DEPTH
-- More explicit but also works
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND DEPTH(path) = DEPTH('/content/blog/') + 1
ORDER BY name;

-- Direct children with specific node type
SELECT id, name, path, node_type
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND node_type = 'my:Article'
ORDER BY created_at DESC;

-- Direct children with property filtering
SELECT id, name, path, properties ->> 'status' AS status
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties ->> 'status' = 'published'
ORDER BY created_at DESC;

-- Count direct children
SELECT COUNT(*) AS child_count
FROM nodes
WHERE PARENT(path) = '/content/blog';

-- Group direct children by node_type
SELECT node_type, COUNT(*) AS count
FROM nodes
WHERE PARENT(path) = '/content/blog'
GROUP BY node_type;

-- ============================================================================
-- RECURSIVE CHILDREN: Get all descendants (subtree)
-- ============================================================================

-- Get all descendants under '/content/blog' (entire subtree)
SELECT id, name, path, DEPTH(path) AS level
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path;

-- Get all descendants with depth limit (e.g., max 2 levels deep)
SELECT id, name, path, DEPTH(path) - DEPTH('/content/blog/') AS relative_depth
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND DEPTH(path) <= DEPTH('/content/blog/') + 2
ORDER BY path;

-- Count total descendants
SELECT COUNT(*) AS total_descendants
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/');

-- ============================================================================
-- CHILDREN WITH METADATA
-- ============================================================================

-- List children with full metadata
SELECT
    id,
    name,
    path,
    node_type,
    properties ->> 'title' AS title,
    created_at,
    updated_at,
    created_by
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC;

-- Children with JSON property extraction
SELECT
    id,
    name,
    path,
    properties ->> 'title' AS title,
    properties ->> 'status' AS status,
    properties ->> 'author' AS author,
    JSON_VALUE(properties, '$.views' RETURNING DOUBLE) AS views
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY JSON_VALUE(properties, '$.views' RETURNING DOUBLE) DESC;

-- ============================================================================
-- PAGINATION OF CHILDREN
-- ============================================================================

-- Paginated children (page 1, 10 items per page)
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

-- Paginated children (page 2)
SELECT id, name, path, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC
LIMIT 10 OFFSET 10;

-- Children with total count for pagination
-- Note: In practice, you'd run two queries - one for count, one for data
SELECT COUNT(*) AS total_count FROM nodes WHERE PARENT(path) = '/content/blog';
SELECT id, name, path FROM nodes WHERE PARENT(path) = '/content/blog' LIMIT 10 OFFSET 0;

-- ============================================================================
-- CHILDREN WITH SPECIFIC PROPERTIES
-- ============================================================================

-- Children that have a specific property
SELECT id, name, path
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND JSON_EXISTS(properties, '$.featured');

-- Children with property value match
SELECT id, name, path, properties ->> 'category' AS category
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties @> '{"category": "technology"}';

-- Children with array contains
SELECT id, name, path
FROM nodes
WHERE PARENT(path) = '/content/blog'
AND properties @> '{"tags": ["rust"]}';

-- ============================================================================
-- BREADCRUMB / ANCESTRY QUERIES
-- ============================================================================

-- Get all ancestors of a node (breadcrumb trail)
-- For node at '/content/blog/2025/article-1'
-- This is tricky in pure SQL without recursive CTEs
-- We can approximate by selecting nodes whose path is a prefix of our target

-- Get parent
SELECT id, name, path
FROM nodes
WHERE path = PARENT('/content/blog/2025/article-1');

-- Get grandparent
SELECT id, name, path
FROM nodes
WHERE path = PARENT(PARENT('/content/blog/2025/article-1'));

-- Note: Full ancestry would require recursive query or application-level logic

-- ============================================================================
-- SIBLINGS: Nodes at the same level with same parent
-- ============================================================================

-- Get siblings of a node
SELECT id, name, path
FROM nodes
WHERE PARENT(path) = PARENT('/content/blog/article-1')
AND path != '/content/blog/article-1'
ORDER BY name;

-- ============================================================================
-- TREE STRUCTURE WITH DEPTH INDICATORS
-- ============================================================================

-- Get tree with indentation hints (relative depth)
SELECT
    id,
    name,
    path,
    DEPTH(path) - DEPTH('/content/') AS indent_level,
    node_type
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
AND DEPTH(path) <= DEPTH('/content/') + 3
ORDER BY path;

-- ============================================================================
-- CHILDREN BY HIERARCHY LEVEL
-- ============================================================================

-- Get only level 3 nodes under /content
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
AND DEPTH(path) = 3
ORDER BY path;

-- Get nodes at specific depth range
SELECT id, name, path, DEPTH(path) AS depth
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
AND DEPTH(path) BETWEEN 2 AND 4
ORDER BY DEPTH(path), path;

-- ============================================================================
-- COMBINED: Children with parent information
-- ============================================================================

-- Children with their parent's name (self-join)
SELECT
    c.id,
    c.name AS child_name,
    c.path AS child_path,
    p.name AS parent_name
FROM nodes c
LEFT JOIN nodes p ON p.path = PARENT(c.path)
WHERE c.path LIKE '/content/blog/%'
AND DEPTH(c.path) = DEPTH('/content/blog/') + 1;

-- ============================================================================
-- EMPTY FOLDERS: Parents with no children
-- ============================================================================

-- Find nodes that have no children (leaf nodes)
SELECT id, name, path
FROM nodes n
WHERE NOT EXISTS (
    SELECT 1 FROM nodes WHERE PARENT(path) = n.path
)
AND PATH_STARTS_WITH(n.path, '/content/');

-- Find nodes that have children (branches)
SELECT id, name, path
FROM nodes n
WHERE EXISTS (
    SELECT 1 FROM nodes WHERE PARENT(path) = n.path
)
AND PATH_STARTS_WITH(n.path, '/content/');

-- Count children per parent
SELECT
    PARENT(path) AS parent_path,
    COUNT(*) AS child_count
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
GROUP BY PARENT(path)
ORDER BY child_count DESC;

-- ============================================================================
-- PERFORMANCE OPTIMIZED: Direct children lookup
-- ============================================================================

-- Most efficient query for listing children (uses RocksDB prefix scan)
-- This should map to O(log n + k) where k is number of results
SELECT id, name, path, node_type, created_at
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY created_at DESC;

-- Most efficient subtree query (also uses prefix scan)
SELECT id, name, path
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
LIMIT 1000;
