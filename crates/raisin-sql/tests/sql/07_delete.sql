-- DELETE statements for removing nodes

-- Delete a single node by ID
DELETE FROM nodes WHERE id = 'node-xyz-123';

-- Delete by path
DELETE FROM nodes WHERE path = '/content/blog/old-post';

-- Delete a subtree (recursive) using PATH_STARTS_WITH
DELETE FROM nodes WHERE PATH_STARTS_WITH(path, '/archive/old-projects/');

-- Delete with status filter
DELETE FROM nodes WHERE properties ->> 'status' = 'deleted';

-- Delete old draft articles
DELETE FROM nodes
WHERE node_type = 'my:Article'
AND properties ->> 'status' = 'draft'
AND created_at < '2024-01-01';

-- Delete by node type
DELETE FROM nodes WHERE node_type = 'my:TempData';

-- Delete with depth constraint
DELETE FROM nodes WHERE DEPTH(path) > 10;

-- Delete nodes without required property
DELETE FROM nodes
WHERE node_type = 'my:Product'
AND NOT JSON_EXISTS(properties, '$.price');

-- Delete entire subtree under specific path
DELETE FROM nodes WHERE PATH_STARTS_WITH(path, '/temp/');

-- Delete orphaned nodes (nodes with non-existent parent)
DELETE FROM nodes
WHERE PARENT(path) NOT IN (SELECT path FROM nodes);

-- Delete with created_by filter
DELETE FROM nodes
WHERE created_by = 'deleted-user-123';

-- Delete unpublished old content
DELETE FROM nodes
WHERE properties ->> 'status' = 'unpublished'
AND updated_at < '2023-01-01';

-- Delete test data
DELETE FROM nodes WHERE PATH_STARTS_WITH(path, '/test/');

-- Delete specific version nodes (with version check)
DELETE FROM nodes WHERE id = 'node-abc' AND version = 3;

-- Delete by owner
DELETE FROM nodes WHERE owner_id = 'deleted-org-456';

-- Delete nodes with specific tag
DELETE FROM nodes WHERE properties @> '{"tags": ["obsolete"]}';

-- Cascading delete (remove entire subtree)
DELETE FROM nodes WHERE PATH_STARTS_WITH(path, '/projects/cancelled/project-x/');
