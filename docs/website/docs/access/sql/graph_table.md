# GRAPH_TABLE

RaisinDB supports the SQL/PGQ `GRAPH_TABLE` construct from ISO SQL:2023 for querying graph relationships. This is our preferred method for graph pattern matching as it integrates naturally with standard SQL.

## Overview

`GRAPH_TABLE` allows you to:
- Match graph patterns (nodes and relationships)
- Filter matches with WHERE clauses
- Project results into standard SQL rows via COLUMNS

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User)-[:follows]->(b:User)
  WHERE a.name = 'Alice'
  COLUMNS (a.name AS follower, b.name AS following)
);
```

## Syntax

```sql
SELECT ... FROM GRAPH_TABLE([graph_name]
  MATCH graph_pattern
  [WHERE filter_expression]
  COLUMNS (column_list)
) [AS alias];
```

| Clause | Description |
|--------|-------------|
| `graph_name` | Optional. Defaults to `NODES_GRAPH` (all nodes and relations) |
| `MATCH` | Graph pattern to match |
| `WHERE` | Optional filter expression |
| `COLUMNS` | Output columns with optional aliases |

## Node Patterns

Nodes are specified in parentheses:

```sql
-- Any node
(n)

-- Node with label (maps to node_type)
(n:User)

-- Multiple labels (OR - matches any)
(n:User|Admin)
```

### Label Matching

- **Case-insensitive**: `:user` matches `User`
- **Namespace support**: `:Article` matches `news:Article`
- **Multiple labels**: `(n:User|Admin)` matches User OR Admin

## Relationship Patterns

Relationships connect nodes with direction:

```sql
-- Right direction: a to b
(a)-[r]->(b)

-- Left direction: b to a
(a)<-[r]-(b)

-- Any direction
(a)-[r]-(b)

-- With relationship type
(a)-[:follows]->(b)

-- Multiple types (OR)
(a)-[:follows|likes]->(b)

-- With variable binding
(a)-[r:follows]->(b)
```

### Variable-Length Paths

Find paths of variable length with quantifiers:

```sql
-- 1 to 10 hops (default max)
(a)-[:follows*]->(b)

-- Exactly 2 hops
(a)-[:follows*2]->(b)

-- 1 to 3 hops
(a)-[:follows*1..3]->(b)

-- 2 or more hops
(a)-[:follows*2..]->(b)

-- Up to 5 hops
(a)-[:follows*..5]->(b)
```

| Quantifier | Meaning |
|------------|---------|
| `*` | 1 to 10 hops (default max) |
| `*n` | Exactly n hops |
| `*n..m` | n to m hops (inclusive) |
| `*n..` | n to 10 hops |
| `*..m` | 1 to m hops |

## Pattern Types

### Single Node

Match nodes by label:

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (u:User)
  COLUMNS (u.id, u.name)
);
```

### Single Hop

Match direct relationships:

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User)-[:follows]->(b:User)
  COLUMNS (a.name AS follower, b.name AS following)
);
```

### Chain Patterns

Match multi-hop paths:

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User)-[:follows]->(b:User)-[:likes]->(p:Post)
  COLUMNS (a.name, b.name, p.title)
);
```

### Variable-Length Traversal

Find all paths within a range:

```sql
-- Find users connected within 3 hops
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User)-[:follows*1..3]->(b:User)
  WHERE a.id = 'user-123'
  COLUMNS (a.name AS source, b.name AS target)
);
```

## WHERE Clause

Filter matches with expressions:

### Comparison Operators

```sql
WHERE a.age > 18
WHERE a.name = 'Alice'
WHERE a.score <> 0
WHERE a.status != 'inactive'
```

| Operator | Description |
|----------|-------------|
| `=` | Equal |
| `<>`, `!=` | Not equal |
| `<` | Less than |
| `<=` | Less than or equal |
| `>` | Greater than |
| `>=` | Greater than or equal |

### Logical Operators

```sql
WHERE a.active = true AND a.age > 18
WHERE a.role = 'admin' OR a.role = 'moderator'
WHERE NOT a.banned
```

### NULL Checks

```sql
WHERE a.email IS NOT NULL
WHERE a.phone IS NULL
```

### IN Lists

```sql
WHERE a.status IN ('active', 'pending', 'review')
WHERE a.id NOT IN ('user-1', 'user-2')
```

### BETWEEN

```sql
WHERE a.age BETWEEN 18 AND 65
WHERE a.created_at NOT BETWEEN '2023-01-01' AND '2023-12-31'
```

### LIKE Patterns

```sql
WHERE a.name LIKE 'John%'
WHERE a.email LIKE '%@example.com'
WHERE a.code LIKE 'PRD-___-2024'
```

| Pattern | Matches |
|---------|---------|
| `%` | Any sequence of characters |
| `_` | Any single character |

### Arithmetic

```sql
WHERE a.price * a.quantity > 100
WHERE (a.score + b.bonus) / 2 >= 50
```

## COLUMNS Clause

Specify output columns:

### Property Access

```sql
COLUMNS (
  a.id,
  a.name,
  a.properties.email,    -- Nested property
  a.created_at
)
```

### System Fields

These fields are available on all nodes:

| Field | Description |
|-------|-------------|
| `id` | Node UUID |
| `workspace` | Workspace identifier |
| `node_type` | Node type (with namespace) |
| `path` | Full node path |
| `name` | Node name (last path segment) |
| `created_at` | Creation timestamp |
| `updated_at` | Last update timestamp |

### Aliases

```sql
COLUMNS (
  a.name AS author_name,
  b.title AS book_title,
  r.weight AS relationship_strength
)
```

### Wildcards

```sql
-- All columns from all variables
COLUMNS (*)

-- All columns from specific variable
COLUMNS (a.*, b.name)
```

### Aggregate Functions

```sql
-- Count matches
COLUMNS (COUNT(*) AS total)

-- Count non-null values
COLUMNS (COUNT(a.email) AS with_email)

-- Count distinct values
COLUMNS (COUNT(DISTINCT a.status) AS unique_statuses)

-- Collect into array
COLUMNS (COLLECT(b.name) AS friend_names)

-- Numeric aggregates
COLUMNS (
  SUM(a.score) AS total_score,
  AVG(a.rating) AS avg_rating,
  MIN(a.age) AS youngest,
  MAX(a.age) AS oldest
)
```

| Function | Description |
|----------|-------------|
| `COUNT(*)` | Count all matches |
| `COUNT(expr)` | Count non-null values |
| `COUNT(DISTINCT expr)` | Count unique values |
| `COLLECT(expr)` | Gather values into array |
| `ARRAY_AGG(expr)` | Same as COLLECT |
| `SUM(expr)` | Sum numeric values |
| `AVG(expr)` | Average of numeric values |
| `MIN(expr)` | Minimum value |
| `MAX(expr)` | Maximum value |

## Examples

### Find Followers

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (user:User)-[:follows]->(follower:User)
  WHERE user.name = 'Alice'
  COLUMNS (
    follower.name,
    follower.path
  )
);
```

### Count Connections

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (user:User)-[:follows]->(friend:User)
  WHERE user.id = 'user-123'
  COLUMNS (
    user.name,
    COUNT(*) AS follower_count,
    COLLECT(friend.name) AS follower_names
  )
);
```

### Find Friends of Friends

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User)-[:follows*2]->(c:User)
  WHERE a.name = 'Alice'
  COLUMNS (
    a.name AS person,
    c.name AS friend_of_friend
  )
);
```

### Multi-Hop with Filters

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (author:Author)-[:wrote]->(book:Book)-[:published_by]->(pub:Publisher)
  WHERE author.country = 'USA' AND book.year > 2020
  COLUMNS (
    author.name,
    book.title,
    pub.name AS publisher
  )
);
```

### Articles with Tags

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (article:Article)-[:tagged]->(tag:Tag)
  COLUMNS (
    article.title,
    COLLECT(tag.name) AS tags
  )
);
```

### Find Paths Within Range

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (start:Location)-[:connected_to*1..5]->(end:Location)
  WHERE start.name = 'New York' AND end.name = 'Los Angeles'
  COLUMNS (
    start.name AS origin,
    end.name AS destination
  )
);
```

## Combining with SQL

GRAPH_TABLE returns a table that can be used in standard SQL:

```sql
-- Join with regular tables
SELECT g.*, u.email
FROM GRAPH_TABLE(
  MATCH (a:User)-[:follows]->(b:User)
  COLUMNS (a.id AS follower_id, b.id AS following_id)
) AS g
JOIN user_profiles u ON g.follower_id = u.id;

-- Subquery
SELECT * FROM products
WHERE category_id IN (
  SELECT id FROM GRAPH_TABLE(
    MATCH (c:Category)-[:parent_of*]->(sub:Category)
    WHERE c.name = 'Electronics'
    COLUMNS (sub.id)
  )
);

-- ORDER BY, LIMIT
SELECT * FROM GRAPH_TABLE(
  MATCH (u:User)-[:follows]->(f:User)
  COLUMNS (u.name, COUNT(*) AS followers)
)
ORDER BY followers DESC
LIMIT 10;
```

## Performance Considerations

- **Label filters**: Always specify labels when possible to reduce scan scope
- **Variable-length paths**: Use reasonable max depth (default 10, warn at >5)
- **Path limit**: Variable-length queries return max 10,000 paths
- **Direction**: Specify direction when known to avoid bidirectional scans

## Related

- [RaisinSQL Overview](./raisinsql.md) - General SQL syntax
- [Cypher](./cypher.md) - Alternative graph query syntax (less preferred)
