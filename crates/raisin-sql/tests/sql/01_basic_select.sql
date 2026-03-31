-- Basic SELECT queries for the nodes virtual table

-- Select all columns
SELECT * FROM nodes;

-- Select specific columns
SELECT id, name, path FROM nodes;

-- Select with WHERE clause
SELECT id, name FROM nodes WHERE id = 'node-123';

-- Select with LIKE
SELECT * FROM nodes WHERE name LIKE 'blog%';

-- Select with IN
SELECT * FROM nodes WHERE node_type IN ('my:Page', 'my:Article');

-- Select with ORDER BY
SELECT id, name, created_at FROM nodes ORDER BY created_at DESC;

-- Select with LIMIT and OFFSET
SELECT * FROM nodes LIMIT 10 OFFSET 20;

-- Select with multiple conditions
SELECT * FROM nodes
WHERE node_type = 'my:Article'
AND created_by = 'user-123'
ORDER BY updated_at DESC
LIMIT 50;

-- Select with comparison operators
SELECT * FROM nodes WHERE version > 5;

-- Select with BETWEEN
SELECT * FROM nodes WHERE created_at BETWEEN '2025-01-01' AND '2025-12-31';

-- Select COUNT
SELECT COUNT(*) FROM nodes;

-- Select with GROUP BY
SELECT node_type, COUNT(*) as count FROM nodes GROUP BY node_type;

-- Select with HAVING
SELECT node_type, COUNT(*) as count
FROM nodes
GROUP BY node_type
HAVING COUNT(*) > 10;

-- Aggregates with FILTER clause
SELECT
  COUNT(*) FILTER (WHERE properties ? 'description') AS has_description,
  COUNT(*) FILTER (WHERE NOT properties ? 'description') AS no_description,
  COUNT(*) AS total
FROM nodes;

-- Multiple filtered aggregates
SELECT
  node_type,
  COUNT(*) FILTER (WHERE version > 1) AS updated_count,
  COUNT(*) FILTER (WHERE version = 1) AS original_count,
  COUNT(*) AS total_count
FROM nodes
GROUP BY node_type;

-- SUM with FILTER
SELECT
  SUM(version) FILTER (WHERE node_type = 'my:Article') AS article_versions,
  SUM(version) FILTER (WHERE node_type = 'my:Page') AS page_versions,
  SUM(version) AS total_versions
FROM nodes;

-- AVG with FILTER
SELECT
  AVG(version) FILTER (WHERE created_at > '2025-01-01') AS recent_avg_version,
  AVG(version) AS all_avg_version
FROM nodes;

-- MIN/MAX with FILTER
SELECT
  MIN(created_at) FILTER (WHERE node_type = 'my:Article') AS first_article,
  MAX(created_at) FILTER (WHERE node_type = 'my:Article') AS last_article
FROM nodes;

-- FILTER with complex conditions
SELECT
  COUNT(*) FILTER (WHERE properties ? 'tags' AND version > 1) AS tagged_updated,
  COUNT(*) FILTER (WHERE properties ? 'tags' OR version > 1) AS tagged_or_updated
FROM nodes;
