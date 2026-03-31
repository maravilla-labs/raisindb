# RaisinSQL Reference

RaisinSQL is a PostgreSQL-compatible SQL dialect with powerful extensions for hierarchical data, JSON operations, full-text search, and vector similarity.

## Overview

RaisinSQL currently supports **SELECT queries only** for reading data. In RaisinSQL, **each workspace is a table**. Query your workspace by using its name as the table name.

```sql
-- Basic query structure (using workspace as table)
SELECT [columns]
FROM workspace_name
WHERE [conditions]
ORDER BY [fields]
LIMIT [number] OFFSET [number];

-- Common workspace examples
SELECT * FROM default;   -- Query the "default" workspace
SELECT * FROM content;   -- Query the "content" workspace
SELECT * FROM users;     -- Query the "users" workspace
```

**Important:** If you query a workspace that doesn't exist, the query engine will return an error. Make sure the workspace exists in your repository before querying it.

## Supported Statement

### SELECT

Retrieve nodes matching specific criteria. This is currently the **only supported statement**.

```sql
-- Basic select (using workspace name as table)
SELECT * FROM default;

-- Select specific columns
SELECT id, name, path, node_type FROM content;

-- Select with JSON extraction
SELECT id, properties ->> 'title' AS title FROM content WHERE id = 'node-123';
```

**Note:** INSERT, UPDATE, and DELETE are not yet implemented.

## Core SQL Features

### WHERE Clauses

```sql
-- Equality
WHERE id = 'node-123'
WHERE node_type = 'my:Article'

-- Comparison
WHERE version > 10
WHERE created_at > '2025-01-01'

-- Pattern matching
WHERE name LIKE '%blog%'
WHERE path LIKE '/content/%'

-- IN lists
WHERE node_type IN ('my:Article', 'my:Page')
WHERE id IN ('node-1', 'node-2', 'node-3')
```

### Boolean Logic

```sql
-- AND
WHERE node_type = 'my:Article' AND properties ->> 'status' = 'published'

-- OR
WHERE path = '/home' OR path = '/about'

-- NOT
WHERE NOT (properties ->> 'archived' = 'true')

-- Complex combinations
WHERE (node_type = 'my:Article' OR node_type = 'my:Page')
  AND properties ->> 'status' = 'published'
  AND created_at > '2025-01-01'
```

### ORDER BY

```sql
-- Single field
ORDER BY created_at DESC
ORDER BY name ASC

-- Multiple fields
ORDER BY path ASC, created_at DESC

-- JSON fields
ORDER BY properties ->> 'title' ASC
```

### LIMIT and OFFSET

```sql
-- Pagination
LIMIT 10 OFFSET 0   -- First page
LIMIT 10 OFFSET 10  -- Second page

-- Just limit
LIMIT 100
```

## Hierarchical Functions

RaisinSQL's most powerful feature is built-in hierarchical path operations.

### PATH_STARTS_WITH

Find all nodes under a specific path (subtree query).

```sql
PATH_STARTS_WITH(path, prefix) → boolean
```

**Examples:**

```sql
-- Get all blog content
SELECT * FROM default WHERE PATH_STARTS_WITH(path, '/content/blog/');

-- Get all 2025 articles
SELECT * FROM default WHERE PATH_STARTS_WITH(path, '/content/blog/2025/');

-- Combine with other conditions
SELECT id, name, properties ->> 'title' AS title
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC;
```

**Performance:** This uses RocksDB prefix scans - `O(log n + k)` where k is the number of results. Very efficient!

### PARENT

Extract the parent path of a node, optionally going multiple levels up.

```sql
PARENT(path) → text (nullable)
PARENT(path, levels) → text (nullable)
```

**Parameters:**
- `path`: The node path to get the parent of
- `levels` (optional): Number of levels to go up the hierarchy (default: 1)

**Examples:**

```sql
-- Find direct children of a node (most common use case)
SELECT * FROM default WHERE PARENT(path) = '/content/blog';

-- Navigate multiple levels up
SELECT
  path,
  PARENT(path) AS parent,             -- 1 level up (default)
  PARENT(path, 1) AS parent_explicit,  -- Same as above
  PARENT(path, 2) AS grandparent,      -- 2 levels up
  PARENT(path, 3) AS great_grandparent -- 3 levels up
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND DEPTH(path) >= 4;

-- Find nodes with same grandparent (sibling branches)
SELECT
  d1.path AS path1,
  d2.path AS path2,
  PARENT(d1.path, 2) AS shared_grandparent
FROM default d1
JOIN default d2 ON PARENT(d1.path, 2) = PARENT(d2.path, 2)
WHERE d1.path < d2.path  -- Avoid duplicates
  AND PATH_STARTS_WITH(d1.path, '/content/')
  AND DEPTH(d1.path) >= 3
LIMIT 20;

-- List children with parent info
SELECT
  c.id,
  c.name AS child_name,
  p.name AS parent_name
FROM default c
LEFT JOIN default p ON p.path = PARENT(c.path)
WHERE PARENT(c.path) = '/content';

-- Find nodes whose grandparent ends with '/blog'
SELECT id, name, path, PARENT(path, 2) AS grandparent
FROM default
WHERE PARENT(path, 2) LIKE '%/blog'
  AND DEPTH(path) >= 3;
```

**Note:** Returns empty string if the requested levels exceed the path depth, or if the path is root.

### ANCESTOR

Get ancestor node at a specific absolute depth from root.

```sql
ANCESTOR(path, depth) → text
```

**Parameters:**
- `path`: The node path to get the ancestor of
- `depth`: The absolute depth level from root (1 = first level, 2 = second level, etc.)

**Examples:**

```sql
-- Get ancestor at depth 2 (e.g., category level)
SELECT
  path,
  ANCESTOR(path, 1) AS root_ancestor,
  ANCESTOR(path, 2) AS category_ancestor,
  DEPTH(path) AS current_depth
FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND DEPTH(path) >= 2
ORDER BY path;

-- Group documents by their depth-2 ancestor (category)
SELECT
  ANCESTOR(path, 2) AS category,
  COUNT(*) AS doc_count,
  ARRAY_AGG(name) AS doc_names
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
  AND DEPTH(path) >= 3
GROUP BY ANCESTOR(path, 2)
ORDER BY doc_count DESC;

-- Filter by ancestor pattern
SELECT id, name, path
FROM default
WHERE ANCESTOR(path, 2) = '/content/blog'
  AND DEPTH(path) > 2
ORDER BY path;
```

**Note:** Returns empty string if the requested depth exceeds the path depth. Use `PARENT(path, levels)` for relative navigation from current node.

**Difference between ANCESTOR and PARENT:**
- `ANCESTOR(path, N)`: Returns ancestor at absolute depth N from root
- `PARENT(path, N)`: Returns parent N levels up from current node

Example:
```sql
-- For path '/a/b/c/d/e' (depth 5):
SELECT
  ANCESTOR('/a/b/c/d/e', 2) AS ancestor_at_depth_2,  -- '/a/b' (always depth 2)
  PARENT('/a/b/c/d/e', 3) AS parent_3_levels_up      -- '/a/b' (depth 5 - 3 = 2)
FROM default;
```

### DEPTH

Get the hierarchical depth of a path.

```sql
DEPTH(path) → integer
```

**Examples:**

```sql
-- Find nodes at specific depth
SELECT * FROM default WHERE DEPTH(path) = 3;

-- Find nodes within depth range
SELECT * FROM default WHERE DEPTH(path) BETWEEN 2 AND 4;

-- Combine with path queries
SELECT id, path, DEPTH(path) as depth
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
  AND DEPTH(path) > 2
ORDER BY depth, path;

-- Find leaf nodes (deepest in each branch)
SELECT path, DEPTH(path) as depth
FROM default
WHERE PATH_STARTS_WITH(path, '/content/')
ORDER BY depth DESC
LIMIT 10;
```

## Reference Resolution

Resolve `PropertyValue::Reference` entries in node properties into full node data.

### RESOLVE

Resolve references in a JSONB value, replacing reference objects with the full referenced node data.

```sql
RESOLVE(jsonb) → jsonb
RESOLVE(jsonb, depth) → jsonb
```

**Parameters:**
- `jsonb`: A JSONB value containing references. Can be a single reference (`properties -> 'field'`) or an entire properties object (`properties`).
- `depth` (optional): How many levels of nested references to resolve (default: `1`, max: `10`). Depth `0` returns the input unchanged.

**Returns:** JSONB with reference objects replaced by the full node data (`id`, `name`, `path`, `node_type`, plus all properties). Returns `NULL` if input is `NULL`. Unresolvable references are kept as-is.

**Examples:**

```sql
-- Resolve a single reference property
SELECT RESOLVE(properties -> 'author') AS author
FROM content
WHERE id = 'article-123';

-- Result: {"id": "author-1", "name": "Jane", "path": "/authors/jane", "node_type": "my:Author", "bio": "..."}
-- (Instead of: {"raisin:ref": "author-1", "raisin:workspace": "content", "raisin:path": "/authors/jane"})
```

```sql
-- Resolve all references in properties at once
SELECT id, name, RESOLVE(properties) AS resolved
FROM content
WHERE node_type = 'my:Article'
LIMIT 10;
```

```sql
-- Access fields from a resolved reference
SELECT
  id,
  RESOLVE(properties -> 'author') ->> 'name' AS author_name,
  RESOLVE(properties -> 'author') ->> 'bio' AS author_bio
FROM content
WHERE node_type = 'my:Article';
```

```sql
-- Resolve nested references (depth 2)
-- If the resolved author node itself contains references (e.g., author -> company),
-- those will also be resolved.
SELECT RESOLVE(properties, 2) AS deep_resolved
FROM content
WHERE id = 'article-123';
```

```sql
-- Combine with other functions
SELECT
  id,
  properties ->> 'title' AS title,
  RESOLVE(properties -> 'category') ->> 'name' AS category_name
FROM content
WHERE PATH_STARTS_WITH(path, '/blog/')
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 20;
```

**How it works:**

| Input | Behavior |
|-------|----------|
| Single reference (`{"raisin:ref": "...", ...}`) | Fetches the referenced node by ID, returns it as JSONB |
| Properties object with references | Walks the entire object, finds all references, and replaces each with the full node |
| No references found | Returns the input unchanged |
| `NULL` input | Returns `NULL` |
| Unresolvable reference (deleted node) | Keeps the original reference object |

**Depth control:**

| Depth | Behavior |
|-------|----------|
| `0` | No resolution, return input as-is |
| `1` (default) | Resolve immediate references only |
| `2` | Resolve references, then resolve references within the resolved nodes |
| `N` (max 10) | Resolve N levels deep |

**Cross-workspace:** References that point to nodes in other workspaces are resolved using the reference's `raisin:workspace` field.

**Circular references:** Protected by a visited-set. If a circular reference chain is detected (A &rarr; B &rarr; A), the cycle is broken and the already-visited reference is kept as-is.

**Performance:** For a single reference (`RESOLVE(properties -> 'field')`), this is a single node lookup. For full properties (`RESOLVE(properties)`), the function walks the property tree to find all references. Use single-reference resolution when you only need one field.

## JSON Operations

Node properties are stored as JSONB, supporting PostgreSQL-style JSON operations.

### Arrow Operator (->>)

Extract JSON values as text.

```sql
properties ->> 'field' → text (nullable)
```

**Examples:**

```sql
-- Extract single field
SELECT properties ->> 'title' AS title FROM default;

-- Filter by JSON value
SELECT * FROM default WHERE properties ->> 'status' = 'published';

-- Multiple extractions
SELECT
  id,
  properties ->> 'title' AS title,
  properties ->> 'author' AS author,
  properties ->> 'status' AS status
FROM default
WHERE node_type = 'my:Article';
```

**Note:** Nested extraction (e.g., `properties ->> 'author' ->> 'name'`) is not currently supported. Use JSON_VALUE for nested paths.

### JSON_VALUE

Extract JSON values from nested paths.

```sql
JSON_VALUE(properties, '$.path') → text (nullable)
```

**Examples:**

```sql
-- Extract nested path
SELECT JSON_VALUE(properties, '$.seo.title') AS seo_title FROM default;
SELECT JSON_VALUE(properties, '$.author.email') AS author_email FROM default;

-- Use in WHERE clause
SELECT * FROM default
WHERE JSON_VALUE(properties, '$.author.name') = 'John Smith';
```

### JSON_EXISTS

Check if a JSON path exists.

```sql
JSON_EXISTS(properties, '$.path') → boolean
```

**Examples:**

```sql
-- Check if field exists
SELECT * FROM default WHERE JSON_EXISTS(properties, '$.seo');

-- Find nodes with specific structure
SELECT * FROM default
WHERE JSON_EXISTS(properties, '$.seo.title')
  AND JSON_EXISTS(properties, '$.seo.description');

-- Find nodes missing fields
SELECT * FROM default
WHERE NOT JSON_EXISTS(properties, '$.publishedAt')
  AND properties ->> 'status' = 'published';
```

### JSON Containment (@>)

Check if JSONB contains another JSONB value.

```sql
properties @> '{"key": "value"}' → boolean
```

**Examples:**

```sql
-- Check single property
SELECT * FROM default WHERE properties @> '{"status": "published"}';

-- Check nested object
SELECT * FROM default WHERE properties @> '{"author": {"role": "admin"}}';

-- Combine with other conditions
SELECT id, name, properties ->> 'title' AS title
FROM default
WHERE properties @> '{"status": "published"}'
  AND PATH_STARTS_WITH(path, '/content/blog/');
```

### Typed JSON Extractors

Extract JSON values with automatic type conversion:

```sql
-- Get as text (nullable)
JSON_GET_TEXT(properties, 'key') → text?

-- Get as number (nullable)
JSON_GET_DOUBLE(properties, 'price') → double?
JSON_GET_INT(properties, 'count') → int?

-- Get as boolean (nullable)
JSON_GET_BOOL(properties, 'active') → bool?
```

**Examples:**

```sql
-- Extract with type conversion
SELECT
  JSON_GET_DOUBLE(properties, 'price') AS price,
  JSON_GET_INT(properties, 'views') AS views,
  JSON_GET_BOOL(properties, 'featured') AS featured
FROM default
WHERE JSON_GET_DOUBLE(properties, 'price') > 100.0;
```

## Full-Text Search

Search content using Tantivy full-text indexing.

### FULLTEXT_MATCH

Search indexed content.

```sql
FULLTEXT_MATCH(query, language) → boolean
```

**Parameters:**
- `query`: Search query using Tantivy syntax
- `language`: Language for stemming (`'english'`, `'german'`, `'french'`, `'spanish'`, `'simple'`)

**Query Syntax:**
- `'rust AND web'` - Both terms required
- `'rust OR python'` - Either term
- `'rust NOT javascript'` - Exclude term
- `'perform*'` - Prefix wildcard
- `'performnce~2'` - Fuzzy match (edit distance 2)
- `'"high performance"'` - Exact phrase

**Examples:**

```sql
-- Basic search
SELECT id, name, properties ->> 'title' AS title
FROM default
WHERE FULLTEXT_MATCH('database performance', 'english')
ORDER BY updated_at DESC
LIMIT 20;

-- Boolean operators
SELECT * FROM default
WHERE FULLTEXT_MATCH('(database OR storage) AND NOT legacy', 'english')
  AND properties ->> 'status' = 'published';

-- With hierarchy filter
SELECT id, name, path
FROM default
WHERE FULLTEXT_MATCH('architecture', 'english')
  AND PATH_STARTS_WITH(path, '/content/blog/')
LIMIT 20;

-- Multi-language search
SELECT id, properties ->> 'title' AS title
FROM default
WHERE FULLTEXT_MATCH('datenbank AND leistung', 'german')
LIMIT 10;
```

**Note:** Only properties listed in the node type's "properties_to_index" configuration are searchable. You must configure indexing in your node type schema first.

## Vector Search

Search by vector embeddings using k-nearest neighbors.

### EMBEDDING Function

Generate embedding vectors from text (requires embedding provider configuration).

```sql
EMBEDDING(text) → vector
```

### Vector Distance Operators

```sql
-- L2 distance (Euclidean)
vector1 <-> vector2 → double

-- Cosine distance
vector1 <=> vector2 → double

-- Inner product (negative dot product)
vector1 <#> vector2 → double
```

**Note:** These operators are parsed but currently mapped to function calls. Direct operator syntax may not work in all contexts. Use the function equivalents:

```sql
-- Alternative function syntax (always works)
VECTOR_L2_DISTANCE(vec1, vec2) → double
VECTOR_COSINE_DISTANCE(vec1, vec2) → double
VECTOR_INNER_PRODUCT(vec1, vec2) → double
```

**Examples:**

```sql
-- Find similar nodes (assuming embeddings are stored)
SELECT
  id,
  name,
  properties ->> 'title' AS title,
  VECTOR_COSINE_DISTANCE(
    properties -> 'embedding',
    EMBEDDING('search query text')
  ) AS distance
FROM default
WHERE JSON_EXISTS(properties, '$.embedding')
ORDER BY distance
LIMIT 10;
```

**Important:** Vector search requires:
1. An embedding provider configured (e.g., OpenAI, local model)
2. Embeddings stored in node properties
3. HNSW index built for performance (optional but recommended)

## Translation & Localization

RaisinDB provides built-in support for multi-language content through the virtual `locale` column.

### Virtual Locale Column

Every query result includes a `locale` column that indicates which language/locale was used for translation resolution.

```sql
-- Query with default language
SELECT id, path, name, locale FROM default LIMIT 5;

-- Result includes locale column:
-- id       | path          | name         | locale
-- ---------|---------------|--------------|-------
-- node-123 | /content/blog | My Blog Post | en
```

### Filtering by Locale

**Single Locale:**

```sql
-- Get nodes with French translations
SELECT id, name, path, locale
FROM default
WHERE locale = 'fr'
  AND PATH_STARTS_WITH(path, '/content/');
```

**Multiple Locales (returns duplicate rows):**

```sql
-- Get nodes in both English and German
-- Returns ONE ROW PER LOCALE PER NODE
SELECT id, name, path, locale
FROM default
WHERE locale IN ('en', 'de')
ORDER BY path, locale;

-- Result:
-- id       | name             | path          | locale
-- ---------|------------------|---------------|-------
-- node-123 | My Blog Post     | /content/blog | en
-- node-123 | Mein Blogbeitrag | /content/blog | de
```

**Default Language:**

When no locale filter is specified, the repository's configured default language is used:

```sql
-- Uses repository default (e.g., 'en')
SELECT * FROM default;
```

**Use Cases:**

```sql
-- Export content in multiple languages
SELECT id, path, locale, properties ->> 'title' AS title
FROM default
WHERE locale IN ('en', 'de', 'fr', 'es')
  AND node_type = 'my:Article';

-- Compare translations side-by-side
SELECT
  e.id,
  e.properties ->> 'title' AS title_en,
  d.properties ->> 'title' AS title_de
FROM (SELECT * FROM default WHERE locale = 'en') e
JOIN (SELECT * FROM default WHERE locale = 'de') d ON d.id = e.id;

-- Find missing translations
SELECT e.id, e.path
FROM (SELECT id, path FROM default WHERE locale = 'en') e
LEFT JOIN (SELECT id FROM default WHERE locale = 'de') d ON d.id = e.id
WHERE d.id IS NULL;
```

## Advanced Queries

### Aggregation

```sql
-- Count by type
SELECT node_type, COUNT(*) as count
FROM default
GROUP BY node_type;

-- Count by status
SELECT properties ->> 'status' AS status, COUNT(*) as count
FROM default
WHERE node_type = 'my:Article'
GROUP BY properties ->> 'status';

-- Depth distribution
SELECT DEPTH(path) as depth, COUNT(*) as count
FROM default
GROUP BY DEPTH(path)
ORDER BY depth;

-- With HAVING
SELECT PARENT(path) AS parent, COUNT(*) AS child_count
FROM default
WHERE PARENT(path) IS NOT NULL
GROUP BY PARENT(path)
HAVING COUNT(*) > 5
ORDER BY child_count DESC;
```

**Supported Aggregates:**
- `COUNT(*)` / `COUNT(column)`
- `SUM(number)`
- `AVG(number)`
- `MIN(value)`
- `MAX(value)`
- `ARRAY_AGG(value)`

### Subqueries

```sql
-- Find nodes with no children
SELECT id, name, path
FROM default n
WHERE NOT EXISTS (
  SELECT 1 FROM default WHERE PARENT(path) = n.path
);

-- Find most recent articles per author
SELECT *
FROM default n1
WHERE node_type = 'my:Article'
  AND created_at = (
    SELECT MAX(created_at)
    FROM default n2
    WHERE n2.properties ->> 'author' = n1.properties ->> 'author'
  );
```

### Joins

```sql
-- Parent-child join
SELECT
  p.name AS parent_name,
  c.name AS child_name
FROM default c
JOIN default p ON p.path = PARENT(c.path)
WHERE PARENT(c.path) = '/content';

-- Self-join for siblings
SELECT
  n1.name AS node1,
  n2.name AS node2
FROM default n1
JOIN default n2 ON PARENT(n1.path) = PARENT(n2.path)
WHERE n1.id < n2.id  -- Avoid duplicates
  AND PARENT(n1.path) = '/content/blog';
```

## Pagination Strategies

### Offset-based (Simple, Small Datasets)

```sql
-- Page 1
SELECT * FROM default ORDER BY created_at DESC LIMIT 10 OFFSET 0;

-- Page 2
SELECT * FROM default ORDER BY created_at DESC LIMIT 10 OFFSET 10;
```

**Pros:** Simple to implement
**Cons:** Slow for large offsets, unstable if data changes

### Cursor-based (Recommended, Large Datasets)

```sql
-- First page
SELECT * FROM default
ORDER BY created_at DESC, id ASC
LIMIT 10;

-- Next page (after cursor)
SELECT * FROM default
WHERE created_at < :cursor_created_at
   OR (created_at = :cursor_created_at AND id > :cursor_id)
ORDER BY created_at DESC, id ASC
LIMIT 10;
```

**Pros:** Stable, efficient at any page
**Cons:** Slightly more complex

### Path-based (Best for Hierarchies)

```sql
-- First page
SELECT * FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
ORDER BY path
LIMIT 10;

-- Next page
SELECT * FROM default
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND path > :last_path
ORDER BY path
LIMIT 10;
```

**Pros:** Most efficient for tree traversal
**Cons:** Only works for path-sorted results

## Scalar Functions

```sql
-- String functions
LOWER(text) → text
UPPER(text) → text
LENGTH(text) → int

-- Examples
SELECT LOWER(name), UPPER(node_type), LENGTH(path)
FROM default;
```

## Performance Tips

1. **Use PATH_STARTS_WITH** for hierarchical queries - it's optimized for prefix scans
2. **Filter early** - put most selective conditions first in WHERE
3. **Index via full-text search** - for complex JSON property queries
4. **Batch operations** - fetch multiple nodes in one query when possible
5. **Use cursor pagination** - avoid large OFFSET values
6. **Limit result sets** - always use LIMIT for large tables
7. **Leverage path structure** - design paths for efficient querying

## Current Limitations

- ❌ Only SELECT queries supported (no INSERT, UPDATE, DELETE)
- ❌ Each workspace is a table (table name = workspace name)
- ❌ No CTEs (WITH clauses) yet
- ❌ No window functions yet
- ❌ No UNION/INTERSECT/EXCEPT yet
- ❌ Full-text search requires separate index configuration
- ❌ Vector operators (`<->`, `<=>`, `<#>`) may require function equivalents
- ❌ Limited nested JSON extraction (use JSON_VALUE for complex paths)

## Available Columns

All workspace tables (e.g., `default`, `content`, `users`) provide these columns:

- `id` (text) - Unique node identifier
- `name` (text) - Node name
- `path` (path/text) - Hierarchical path
- `node_type` (text) - Node type identifier
- `properties` (jsonb) - JSON properties
- `version` (bigint) - Revision number
- `created_at` (timestamp) - Creation timestamp
- `updated_at` (timestamp) - Last update timestamp
- `locale` (text, virtual) - Resolved locale for translations
- `depth` (int, virtual) - Path depth
- `parent_path` (text, virtual) - Parent node path
- `__revision` (bigint, virtual) - Same as version
- `__branch` (text, virtual) - Branch name
- `__workspace` (text, virtual) - Workspace name
- `embedding` (vector, virtual) - Vector embedding if available

## What's Next?

- [Query Examples](examples.md) - Real-world query patterns
- [Full-Text Search](fulltext.md) - Advanced search capabilities
- [Cypher Graph Queries](cypher.md) - Graph pattern matching
