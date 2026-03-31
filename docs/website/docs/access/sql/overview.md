---
sidebar_position: 1
---

# SQL & Query Capabilities

RaisinDB provides multiple powerful ways to query and retrieve your data. Whether you need simple lookups, complex hierarchical queries, or semantic search across millions of documents, RaisinDB has you covered.

## Query Interfaces

RaisinDB offers two complementary query interfaces:

### RaisinSQL (HTTP & WebSocket)

A PostgreSQL-compatible SQL dialect with custom extensions for hierarchical and graph operations. RaisinSQL is the most powerful and expressive way to query RaisinDB.

**HTTP endpoint:** `POST /api/sql/{repo}`  
**WebSocket:** `RequestType.SqlQuery`

```sql
SELECT id, name, properties ->> 'title' AS title
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND properties ->> 'status' = 'published'
ORDER BY created_at DESC
LIMIT 10;
```

**Best for:**
- Complex queries with multiple conditions
- JSON property filtering and extraction
- Hierarchical tree traversal
- Graph relationship navigation
- Advanced filtering and sorting

[Learn more about RaisinSQL →](raisinsql.md)

### Cypher Graph Queries

An openCypher-based graph query language for expressive relationship traversal. Perfect for navigating connections between nodes.

```sql
SELECT * FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  RETURN user.id, collect(article.id) AS articles
');
```

**Best for:**
- Graph pattern matching
- Relationship traversal
- Finding connected nodes
- Aggregating relationship data
- Expressive graph queries

[Learn more about Cypher →](cypher.md)

### Query DSL

A JSON-based query format perfect for programmatic access and REST APIs.

```json
{
  "and": [
    { "field": { "path": { "like": "/content/blog/" } } },
    { "field": { "nodeType": { "eq": "my:Article" } } }
  ],
  "orderBy": { "created_at": "desc" },
  "limit": 10
}
```

**Best for:**
- REST API integration
- JavaScript/TypeScript clients
- Simple field-based queries
- Programmatic query construction

## Query Engines

### Hierarchical Queries

Every node in RaisinDB has a hierarchical path (like `/content/blog/2025/article-1`). This enables efficient tree operations without complex joins.

**Path-based operations:**
- Find all descendants of a path (subtree queries)
- Navigate parent-child relationships
- Query by depth level
- Efficient prefix scans

```sql
-- Get all blog articles and their children
SELECT * FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/');

-- Find direct children only
SELECT * FROM nodes
WHERE PARENT(path) = '/content/blog';

-- Query by depth
SELECT * FROM nodes
WHERE DEPTH(path) = 3;
```

### JSON Property Queries

Node properties are stored as JSONB (like PostgreSQL), giving you powerful JSON querying capabilities.

**Supported operations:**
- Extract values with `->>` operator
- Type-safe extraction with `JSON_VALUE()`
- Check existence with `JSON_EXISTS()`
- Containment checks with `@>` operator
- JSON merging for updates

```sql
SELECT
  id,
  properties ->> 'title' AS title,
  properties ->> 'author' AS author,
  JSON_VALUE(properties, '$.price' RETURNING DOUBLE) AS price
FROM nodes
WHERE properties @> '{"status": "published"}'
  AND JSON_EXISTS(properties, '$.seo.title');
```

### Graph Queries

Navigate relationships between nodes using either Cypher pattern matching or SQL table functions.

**Cypher Pattern Matching:**

```sql
-- Find folder relationships using graph patterns
SELECT * FROM cypher('
  MATCH (s)-[:ntRaisinFolder]->(t)
  RETURN s.id, s.workspace, t.id, t.workspace
');

-- Fetch full node data with lookup() function
SELECT * FROM cypher('
  MATCH (user)-[:AUTHORED]->(article)
  RETURN
    lookup(user.id, user.workspace) AS user_data,
    lookup(article.id, article.workspace) AS article_data
');
```

**SQL Table Functions:**

```sql
-- Find all articles authored by a user
SELECT n.name, n.properties ->> 'title' AS title
FROM NEIGHBORS('user-123', 'OUT', 'AUTHORED') AS e
JOIN nodes n ON n.id = e.dst_id;

-- Find all organizations a user belongs to
SELECT org.name
FROM NEIGHBORS('user-456', 'OUT', 'MEMBER_OF') AS e
JOIN nodes org ON org.id = e.dst_id;
```

[Learn more about Graph Queries →](cypher.md)

### Vector Similarity Search

Perform k-nearest neighbors search on vector embeddings for semantic search and recommendations.

```sql
-- Find similar articles
SELECT n.id, n.name, knn.distance
FROM KNN(:query_embedding, 20) AS knn
JOIN nodes n ON n.id = knn.node_id
WHERE properties ->> 'status' = 'published'
ORDER BY knn.distance
LIMIT 10;
```

### Full-Text Search

Powered by Tantivy, RaisinDB provides blazing-fast full-text search with support for 20+ languages, fuzzy matching, and wildcards.

**Features:**
- Multi-language stemming (English, German, French, Spanish, and more)
- Fuzzy search with edit distance
- Wildcard queries (`optim*`, `log?`)
- Boolean operators (`AND`, `OR`, `NOT`)
- Branch-aware indexing
- Relevance scoring

```sql
-- Coming soon: Full-text search via SQL
-- For now, use the full-text search API
```

[Learn more about Full-Text Search →](fulltext.md)

## Performance Characteristics

RaisinDB is optimized for different query patterns:

| Query Type | Complexity | Notes |
|------------|-----------|-------|
| Path prefix scan | `O(log n + k)` | Uses RocksDB prefix scans, very efficient |
| Point lookup by ID | `O(log n)` | Direct key lookup |
| Parent lookup | `O(n)` | Full scan, consider indexing |
| JSON extraction | `O(n)` | Applied post-read |
| Full-text search | `O(1) + ranking` | Inverted index lookup |
| Vector KNN | `O(log n)` | HNSW index |

**Best Practices:**

1. **Use path-based queries** whenever possible - they leverage RocksDB's prefix scanning
2. **Prefer cursor-based pagination** over offset for large datasets
3. **Filter early** - put most selective conditions first
4. **Index important JSON properties** via full-text search
5. **Batch reads** when fetching multiple nodes

## Query Comparison

Here's how to express the same query across different interfaces:

**Task:** Find published blog articles with titles containing "performance"

### RaisinSQL
```sql
SELECT id, name, properties ->> 'title' AS title
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND properties ->> 'status' = 'published'
  AND properties ->> 'title' LIKE '%performance%'
ORDER BY created_at DESC;
```

### Query DSL
```json
{
  "and": [
    { "field": { "path": { "like": "/content/blog/" } } },
    { "field": { "nodeType": { "eq": "my:Article" } } }
  ],
  "orderBy": { "created_at": "desc" }
}
```
*Note: Query DSL cannot filter by JSON properties - use RaisinSQL for this*

### Full-Text Search
```
status:published AND title:performance*
```

## What's Next?

- [RaisinSQL Reference](raisinsql.md) - Complete SQL syntax and functions
- [Full-Text Search](fulltext.md) - Search capabilities and configuration
- [Query Examples](examples.md) - Common patterns and use cases
