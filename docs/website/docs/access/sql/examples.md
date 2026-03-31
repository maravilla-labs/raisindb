# Query Examples

This page provides real-world examples of common query patterns in RaisinDB. Each example includes SQL queries and explains the use case.

## Content Management

### List All Published Articles

```sql
SELECT
  id,
  name,
  properties ->> 'title' AS title,
  properties ->> 'author' AS author,
  created_at
FROM nodes
WHERE node_type = 'my:Article'
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 20;
```

### Find Articles by Author

```sql
SELECT
  id,
  properties ->> 'title' AS title,
  created_at
FROM nodes
WHERE node_type = 'my:Article'
  AND properties ->> 'author' = 'john.smith'
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC;
```

### Get Article with SEO Metadata

```sql
SELECT
  id,
  properties ->> 'title' AS title,
  JSON_VALUE(properties, '$.seo.title') AS seo_title,
  JSON_VALUE(properties, '$.seo.description') AS seo_description,
  JSON_VALUE(properties, '$.seo.keywords') AS keywords
FROM nodes
WHERE id = 'article-123';
```

### Find Articles Missing SEO

```sql
SELECT
  id,
  name,
  properties ->> 'title' AS title
FROM nodes
WHERE node_type = 'my:Article'
  AND properties ->> 'status' = 'published'
  AND NOT JSON_EXISTS(properties, '$.seo.title');
```

### Archive Old Draft Articles

```sql
UPDATE nodes
SET properties = properties || '{"status": "archived"}'
WHERE node_type = 'my:Article'
  AND properties ->> 'status' = 'draft'
  AND created_at < '2024-01-01';
```

## Hierarchical Queries

### Get All Blog Posts

```sql
SELECT
  id,
  name,
  path,
  properties ->> 'title' AS title
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path;
```

### Get Direct Children of a Page

```sql
SELECT
  id,
  name,
  path
FROM nodes
WHERE PARENT(path) = '/content/blog'
ORDER BY name;
```

### List Entire Subtree with Depth

```sql
SELECT
  id,
  path,
  DEPTH(path) AS depth,
  properties ->> 'title' AS title
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/docs/')
ORDER BY path;
```

### Find Pages at Specific Depth

```sql
-- Get all pages exactly 3 levels deep
SELECT
  id,
  path,
  name
FROM nodes
WHERE DEPTH(path) = 3
ORDER BY path;
```

### Find All Leaf Nodes (No Children)

```sql
SELECT
  id,
  path,
  name
FROM nodes n
WHERE NOT EXISTS (
  SELECT 1
  FROM nodes
  WHERE PARENT(path) = n.path
)
ORDER BY path;
```

### Breadcrumb Navigation

```sql
-- Get all ancestors of a path
WITH RECURSIVE ancestors AS (
  -- Base case: the current page
  SELECT id, path, name, PARENT(path) AS parent_path
  FROM nodes
  WHERE path = '/content/docs/guides/getting-started'

  UNION ALL

  -- Recursive case: parent pages
  SELECT n.id, n.path, n.name, PARENT(n.path)
  FROM nodes n
  INNER JOIN ancestors a ON n.path = a.parent_path
)
SELECT * FROM ancestors ORDER BY DEPTH(path);
```

Note: CTEs are planned but not yet implemented. Use application-side recursion for now.

## JSON Property Queries

### Filter by Price Range

```sql
SELECT
  id,
  properties ->> 'name' AS product_name,
  JSON_VALUE(properties, '$.price' RETURNING DOUBLE) AS price
FROM nodes
WHERE node_type = 'my:Product'
  AND JSON_VALUE(properties, '$.price' RETURNING DOUBLE) BETWEEN 10.0 AND 100.0
ORDER BY price;
```

### Find Products in Stock

```sql
SELECT
  id,
  properties ->> 'name' AS name,
  JSON_VALUE(properties, '$.inventory.quantity' RETURNING INTEGER) AS quantity
FROM nodes
WHERE node_type = 'my:Product'
  AND JSON_VALUE(properties, '$.inventory.quantity' RETURNING INTEGER) > 0
ORDER BY name;
```

### Search by Tags

```sql
SELECT
  id,
  properties ->> 'title' AS title,
  properties ->> 'tags' AS tags
FROM nodes
WHERE properties @> '{"tags": ["rust"]}'
ORDER BY created_at DESC;
```

### Complex JSON Filtering

```sql
SELECT
  id,
  properties ->> 'title' AS title,
  properties ->> 'author' ->> 'name' AS author
FROM nodes
WHERE properties @> '{"status": "published", "featured": true}'
  AND JSON_EXISTS(properties, '$.author.verified')
  AND JSON_VALUE(properties, '$.author.verified' RETURNING BOOLEAN) = true
ORDER BY created_at DESC;
```

## Relationships & Graph Queries

### Find All Articles by an Author

```sql
SELECT
  n.id,
  n.properties ->> 'title' AS title,
  n.created_at
FROM NEIGHBORS('user-123', 'OUT', 'AUTHORED') AS e
JOIN nodes n ON n.id = e.dst_id
WHERE n.properties ->> 'status' = 'published'
ORDER BY n.created_at DESC;
```

### Find All Authors of an Article

```sql
SELECT
  u.id,
  u.name,
  u.properties ->> 'email' AS email
FROM NEIGHBORS('article-456', 'IN', 'AUTHORED') AS e
JOIN nodes u ON u.id = e.src_id;
```

### Organization Membership

```sql
-- Find all members of an organization
SELECT
  u.id,
  u.name,
  u.properties ->> 'email' AS email,
  u.properties ->> 'role' AS role
FROM NEIGHBORS('org-789', 'IN', 'MEMBER_OF') AS e
JOIN nodes u ON u.id = e.src_id
ORDER BY u.name;
```

### User's Organizations

```sql
-- Find all organizations a user belongs to
SELECT
  org.id,
  org.name,
  org.properties ->> 'industry' AS industry
FROM NEIGHBORS('user-123', 'OUT', 'MEMBER_OF') AS e
JOIN nodes org ON org.id = e.dst_id
ORDER BY org.name;
```

### Friends Network (Bidirectional)

```sql
-- Find all friends (both incoming and outgoing)
SELECT DISTINCT
  n.id,
  n.name,
  n.properties ->> 'email' AS email
FROM NEIGHBORS('user-123', 'BOTH', 'FRIEND') AS e
JOIN nodes n ON n.id = CASE
  WHEN e.direction = 'OUT' THEN e.dst_id
  WHEN e.direction = 'IN' THEN e.src_id
END
ORDER BY n.name;
```

### Content Categories

```sql
-- Find all articles in a category
SELECT
  a.id,
  a.properties ->> 'title' AS title,
  a.created_at
FROM NEIGHBORS('category-tech', 'IN', 'CATEGORIZED_AS') AS e
JOIN nodes a ON a.id = e.src_id
WHERE a.properties ->> 'status' = 'published'
ORDER BY a.created_at DESC;
```

### Related Content

```sql
-- Find related articles (same category)
SELECT DISTINCT
  related.id,
  related.properties ->> 'title' AS title,
  related.created_at
FROM NEIGHBORS('article-123', 'OUT', 'CATEGORIZED_AS') AS e1
CROSS JOIN LATERAL NEIGHBORS(e1.dst_id, 'IN', 'CATEGORIZED_AS') AS e2
JOIN nodes related ON related.id = e2.src_id
WHERE related.id != 'article-123'
  AND related.properties ->> 'status' = 'published'
ORDER BY related.created_at DESC
LIMIT 5;
```

## Vector Similarity Search

### Find Similar Articles

```sql
SELECT
  n.id,
  n.properties ->> 'title' AS title,
  knn.distance
FROM KNN(:query_embedding, 20) AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE n.properties ->> 'status' = 'published'
ORDER BY knn.distance
LIMIT 10;
```

### Semantic Search with Filtering

```sql
-- Find similar articles in a specific category
SELECT
  n.id,
  n.properties ->> 'title' AS title,
  n.properties ->> 'category' AS category,
  knn.distance
FROM KNN(:embedding, 50, 'node_type = "my:Article"') AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE PATH_STARTS_WITH(n.path, '/content/blog/')
  AND n.properties ->> 'status' = 'published'
ORDER BY knn.distance
LIMIT 10;
```

### Recommendation System

```sql
-- Find products similar to recently viewed
SELECT
  p.id,
  p.properties ->> 'name' AS name,
  JSON_VALUE(p.properties, '$.price' RETURNING DOUBLE) AS price,
  knn.distance
FROM KNN(:product_embedding, 30) AS knn
JOIN nodes p ON p.id = knn.node_id
WHERE p.node_type = 'my:Product'
  AND JSON_VALUE(p.properties, '$.inventory.quantity' RETURNING INTEGER) > 0
  AND p.id NOT IN ('viewed-1', 'viewed-2', 'viewed-3')
ORDER BY knn.distance
LIMIT 12;
```

### Hybrid Search (Keyword + Semantic)

```sql
-- Combine keyword matching with semantic similarity
SELECT
  n.id,
  n.properties ->> 'title' AS title,
  knn.distance,
  CASE
    WHEN n.properties ->> 'title' LIKE '%rust%' THEN knn.distance * 0.5
    ELSE knn.distance
  END AS boosted_score
FROM KNN(:embedding, 100) AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE (
  n.properties ->> 'title' LIKE '%rust%'
  OR n.properties ->> 'content' LIKE '%rust%'
  OR knn.distance < 0.5
)
ORDER BY boosted_score
LIMIT 20;
```

## Aggregation & Analytics

### Count by Node Type

```sql
SELECT
  node_type,
  COUNT(*) AS count
FROM nodes
GROUP BY node_type
ORDER BY count DESC;
```

### Articles by Status

```sql
SELECT
  properties ->> 'status' AS status,
  COUNT(*) AS count
FROM nodes
WHERE node_type = 'my:Article'
GROUP BY properties ->> 'status';
```

### Content Distribution by Depth

```sql
SELECT
  DEPTH(path) AS depth,
  COUNT(*) AS count
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/')
GROUP BY DEPTH(path)
ORDER BY depth;
```

### Top Authors by Article Count

```sql
SELECT
  properties ->> 'author' AS author,
  COUNT(*) AS article_count
FROM nodes
WHERE node_type = 'my:Article'
  AND properties ->> 'status' = 'published'
GROUP BY properties ->> 'author'
ORDER BY article_count DESC
LIMIT 10;
```

### Monthly Publishing Trends

```sql
SELECT
  DATE_TRUNC('month', created_at) AS month,
  COUNT(*) AS articles_published
FROM nodes
WHERE node_type = 'my:Article'
  AND properties ->> 'status' = 'published'
GROUP BY DATE_TRUNC('month', created_at)
ORDER BY month DESC;
```

Note: `DATE_TRUNC` support depends on storage backend.

## Pagination Patterns

### Offset-Based Pagination (Small Datasets)

```sql
-- Page 1 (items 1-10)
SELECT * FROM nodes
WHERE node_type = 'my:Article'
ORDER BY created_at DESC
LIMIT 10 OFFSET 0;

-- Page 2 (items 11-20)
SELECT * FROM nodes
WHERE node_type = 'my:Article'
ORDER BY created_at DESC
LIMIT 10 OFFSET 10;
```

### Cursor-Based Pagination (Large Datasets)

```sql
-- First page
SELECT id, created_at, name
FROM nodes
WHERE node_type = 'my:Article'
ORDER BY created_at DESC, id ASC
LIMIT 10;

-- Next page (using last item as cursor)
SELECT id, created_at, name
FROM nodes
WHERE node_type = 'my:Article'
  AND (
    created_at < :last_created_at
    OR (created_at = :last_created_at AND id > :last_id)
  )
ORDER BY created_at DESC, id ASC
LIMIT 10;
```

### Path-Based Pagination (Hierarchical)

```sql
-- First page
SELECT id, path, name
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path
LIMIT 10;

-- Next page
SELECT id, path, name
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND path > :last_path
ORDER BY path
LIMIT 10;
```

## Multi-Tenant Queries

### Tenant-Specific Content

```sql
SELECT
  id,
  name,
  properties ->> 'title' AS title
FROM nodes
WHERE properties ->> 'tenant_id' = 'tenant-123'
  AND node_type = 'my:Article'
ORDER BY created_at DESC;
```

### Cross-Tenant Statistics (Admin)

```sql
SELECT
  properties ->> 'tenant_id' AS tenant,
  node_type,
  COUNT(*) AS count
FROM nodes
GROUP BY properties ->> 'tenant_id', node_type
ORDER BY count DESC;
```

## Bulk Operations

### Bulk Update Status

```sql
UPDATE nodes
SET properties = properties || '{"status": "published"}'
WHERE id IN (
  'article-1', 'article-2', 'article-3',
  'article-4', 'article-5'
);
```

### Bulk Delete Old Drafts

```sql
DELETE FROM nodes
WHERE node_type = 'my:Article'
  AND properties ->> 'status' = 'draft'
  AND created_at < '2023-01-01';
```

### Copy Nodes to New Path

```sql
-- Note: This is conceptual - actual implementation depends on storage backend
INSERT INTO nodes (path, node_type, properties)
SELECT
  REPLACE(path, '/old-location/', '/new-location/'),
  node_type,
  properties
FROM nodes
WHERE PATH_STARTS_WITH(path, '/old-location/');
```

## Advanced Patterns

### Find Orphaned Nodes

```sql
-- Nodes whose parent path doesn't exist
SELECT
  n.id,
  n.path,
  PARENT(n.path) AS parent_path
FROM nodes n
WHERE PARENT(n.path) IS NOT NULL
  AND NOT EXISTS (
    SELECT 1
    FROM nodes
    WHERE path = PARENT(n.path)
  );
```

### Duplicate Detection

```sql
-- Find nodes with duplicate names under same parent
SELECT
  name,
  PARENT(path) AS parent,
  COUNT(*) AS duplicate_count
FROM nodes
GROUP BY name, PARENT(path)
HAVING COUNT(*) > 1;
```

### Content Audit

```sql
-- Find published articles missing required fields
SELECT
  id,
  path,
  properties ->> 'title' AS title,
  CASE WHEN JSON_EXISTS(properties, '$.author') THEN 'Yes' ELSE 'No' END AS has_author,
  CASE WHEN JSON_EXISTS(properties, '$.seo.title') THEN 'Yes' ELSE 'No' END AS has_seo
FROM nodes
WHERE node_type = 'my:Article'
  AND properties ->> 'status' = 'published'
  AND (
    NOT JSON_EXISTS(properties, '$.author')
    OR NOT JSON_EXISTS(properties, '$.seo.title')
  );
```

### Search Score Boosting

```sql
-- Combine multiple signals for ranking
SELECT
  n.id,
  n.properties ->> 'title' AS title,
  knn.distance AS semantic_score,
  JSON_VALUE(n.properties, '$.views' RETURNING INTEGER) AS view_count,
  (knn.distance * 0.7 + (10000.0 / (JSON_VALUE(n.properties, '$.views' RETURNING INTEGER) + 1)) * 0.3) AS combined_score
FROM KNN(:embedding, 100) AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE n.properties ->> 'status' = 'published'
ORDER BY combined_score
LIMIT 20;
```

## Performance Tips

### Efficient Filtering

```sql
-- Good: Filter early with indexed fields
SELECT * FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND node_type = 'my:Article'
  AND properties ->> 'status' = 'published';

-- Less efficient: Filter late with JSON
SELECT * FROM nodes
WHERE properties ->> 'status' = 'published'
  AND PATH_STARTS_WITH(path, '/content/blog/');
```

### Avoid Large Offsets

```sql
-- Bad: Large offset requires scanning many rows
SELECT * FROM nodes ORDER BY created_at DESC LIMIT 10 OFFSET 10000;

-- Good: Use cursor-based pagination
SELECT * FROM nodes
WHERE created_at < :cursor_time
ORDER BY created_at DESC
LIMIT 10;
```

### Batch Reads

```sql
-- Good: Single query for multiple nodes
SELECT * FROM nodes
WHERE id IN ('node-1', 'node-2', 'node-3', ...);

-- Bad: Multiple queries
-- SELECT * FROM nodes WHERE id = 'node-1';
-- SELECT * FROM nodes WHERE id = 'node-2';
-- SELECT * FROM nodes WHERE id = 'node-3';
```

## What's Next?

- [RaisinSQL Reference](raisinsql.md) - Complete SQL syntax
- [Full-Text Search](fulltext.md) - Search capabilities
- [Query Overview](overview.md) - Query engines and capabilities
