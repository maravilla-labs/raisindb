-- Hierarchy functions for path-based queries

-- PATH_STARTS_WITH: Find all nodes under a specific path
SELECT * FROM nodes WHERE PATH_STARTS_WITH(path, '/content/');

-- PATH_STARTS_WITH: Find blog posts
SELECT id, name, path FROM nodes WHERE PATH_STARTS_WITH(path, '/content/blog/');

-- PATH_STARTS_WITH with other conditions
SELECT * FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
AND node_type = 'my:Page';

-- PARENT: Find direct children of a node
SELECT * FROM nodes WHERE PARENT(path) = '/content/blog';

-- PARENT with comparison
SELECT id, name, PARENT(path) as parent_path FROM nodes WHERE PARENT(path) = '/archive';

-- DEPTH: Find nodes at specific depth
SELECT * FROM nodes WHERE DEPTH(path) = 3;

-- DEPTH: Range query
SELECT * FROM nodes WHERE DEPTH(path) > 2 AND DEPTH(path) < 5;

-- DEPTH in SELECT
SELECT id, path, DEPTH(path) as depth FROM nodes;

-- Combining multiple hierarchy functions
SELECT id, name, path, DEPTH(path) as depth
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
AND DEPTH(path) = 3
AND PARENT(path) = '/content/blog';

-- PATH_STARTS_WITH for subtree queries
SELECT * FROM nodes
WHERE PATH_STARTS_WITH(path, '/projects/2025/')
ORDER BY created_at DESC;

-- Complex hierarchy query
SELECT
    id,
    name,
    path,
    PARENT(path) as parent,
    DEPTH(path) as level
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
AND DEPTH(path) BETWEEN 2 AND 4
ORDER BY path;
