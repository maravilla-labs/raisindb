-- JSON operations and functions

-- JSON extraction operator: ->>
SELECT properties ->> 'title' AS title FROM nodes;

-- JSON extraction with WHERE
SELECT id, name, properties ->> 'status' AS status
FROM nodes
WHERE properties ->> 'status' = 'published';

-- Multiple JSON extractions
SELECT
    id,
    name,
    properties ->> 'title' AS title,
    properties ->> 'status' AS status,
    properties ->> 'author' AS author
FROM nodes;

-- JSON containment operator: @>
SELECT * FROM nodes WHERE properties @> '{"tags": ["sale"]}';

-- JSON containment with complex structure
SELECT * FROM nodes WHERE properties @> '{"metadata": {"featured": true}}';

-- JSON key existence operator: ?
SELECT * FROM nodes WHERE properties ? 'title';

-- JSON key existence with multiple keys
SELECT id, name,
    properties ? 'title' AS has_title,
    properties ? 'author' AS has_author,
    properties ? 'tags' AS has_tags
FROM nodes;

-- JSON key existence in WHERE clause
SELECT * FROM nodes WHERE properties ? 'published_at' AND properties ? 'author';

-- JSON key existence with nested objects
SELECT * FROM nodes WHERE properties -> 'seo' ? 'description';

-- Combining ? with other JSON operators
SELECT id, name
FROM nodes
WHERE properties ? 'tags'
  AND properties @> '{"tags": ["rust"]}'
  AND properties ->> 'status' = 'published';

-- JSON key existence with metadata
SELECT * FROM nodes
WHERE properties -> 'metadata' ? 'category'
  AND properties -> 'metadata' ->> 'category' = 'tutorial';

-- JSON_VALUE function
SELECT JSON_VALUE(properties, '$.title') AS title FROM nodes;

-- JSON_VALUE with type casting
SELECT JSON_VALUE(properties, '$.price' RETURNING DOUBLE) AS price FROM nodes;

-- JSON_VALUE in WHERE clause
SELECT * FROM nodes WHERE JSON_VALUE(properties, '$.price' RETURNING DOUBLE) > 100.0;

-- JSON_EXISTS function
SELECT * FROM nodes WHERE JSON_EXISTS(properties, '$.seo.title');

-- JSON_EXISTS with complex path
SELECT id, name FROM nodes WHERE JSON_EXISTS(properties, '$.metadata.social.twitter');

-- Combining JSON operators
SELECT
    id,
    properties ->> 'title' AS title,
    properties ->> 'status' AS status
FROM nodes
WHERE properties @> '{"status": "published"}'
AND JSON_EXISTS(properties, '$.seo');

-- JSON with hierarchy functions
SELECT
    id,
    path,
    properties ->> 'title' AS title
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
AND properties ->> 'status' = 'published';

-- Complex JSON query
SELECT
    id,
    name,
    JSON_VALUE(properties, '$.title') AS title,
    JSON_VALUE(properties, '$.price' RETURNING DOUBLE) AS price
FROM nodes
WHERE JSON_VALUE(properties, '$.price' RETURNING DOUBLE) BETWEEN 10.0 AND 100.0
AND JSON_EXISTS(properties, '$.tags')
ORDER BY JSON_VALUE(properties, '$.price' RETURNING DOUBLE) DESC;

-- JSON with aggregations
SELECT
    properties ->> 'category' AS category,
    COUNT(*) AS count
FROM nodes
WHERE properties ->> 'status' = 'published'
GROUP BY properties ->> 'category';

-- Select entire JSONB column (Postgres-style)
-- Just select the column directly, no TO_JSON needed
SELECT properties FROM nodes WHERE id = 'node-123';

-- Select all columns as structured data
SELECT id, name, path, properties, created_at, updated_at
FROM nodes
WHERE id = 'node-123';
