# raisin-sql

SQL query engine for RaisinDB with PostgreSQL-compatible syntax and SQL/PGQ graph query support.

## Overview

This crate implements the complete SQL query pipeline for RaisinDB:

- **Parser**: PostgreSQL-compatible SQL with RaisinDB extensions
- **Analyzer**: Semantic analysis with type checking and validation
- **Logical Planner**: Relational algebra representation
- **Optimizer**: Rule-based query optimization
- **Physical Planner**: Executable plan generation
- **Executor**: Streaming query execution

## Key Features

| Feature | Description |
|---------|-------------|
| **PostgreSQL Syntax** | Standard SQL with familiar operators and functions |
| **Hierarchy Functions** | `PATH_STARTS_WITH`, `PARENT`, `DEPTH` for tree queries |
| **JSON Operations** | `->>`, `@>`, `JSON_VALUE`, `JSON_EXISTS` |
| **Full-Text Search** | `to_tsvector`, `to_tsquery`, `@@`, `ts_rank` |
| **SQL/PGQ** | `GRAPH_TABLE`, `MATCH`, pattern matching, graph algorithms |
| **Graph Mutations** | `RELATE`, `UNRELATE` for relationship management |

## Quick Start

```rust
use raisin_sql::QueryPlan;

// Parse, analyze, and optimize a query
let plan = QueryPlan::from_sql(
    "SELECT id, name, DEPTH(path) as level
     FROM nodes
     WHERE PATH_STARTS_WITH(path, '/content/')
       AND properties ->> 'status' = 'published'
     ORDER BY level
     LIMIT 10"
)?;

// View the execution plan
println!("{}", plan.explain());
```

## SQL Syntax

### SELECT Queries

```sql
-- Basic query with hierarchy
SELECT id, name, DEPTH(path) as depth
FROM nodes
WHERE PATH_STARTS_WITH(path, '/content/blog/')
  AND DEPTH(path) = 3
ORDER BY created_at DESC
LIMIT 20;

-- JSON property access
SELECT
    id,
    properties ->> 'title' AS title,
    properties ->> 'author' AS author
FROM nodes
WHERE properties @> '{"status": "published"}';

-- Full-text search with ranking
SELECT id, properties ->> 'title' AS title,
       ts_rank(to_tsvector('english', properties ->> 'body'),
               to_tsquery('english', 'rust & performance')) AS rank
FROM nodes
WHERE to_tsvector('english', properties ->> 'body')
   @@ to_tsquery('english', 'rust & performance')
ORDER BY rank DESC;
```

### Hierarchy Functions

| Function | Description | Example |
|----------|-------------|---------|
| `PATH_STARTS_WITH(path, prefix)` | Check path prefix | `WHERE PATH_STARTS_WITH(path, '/blog/')` |
| `PARENT(path)` | Get parent path | `WHERE PARENT(path) = '/content'` |
| `DEPTH(path)` | Get path depth | `WHERE DEPTH(path) <= 3` |

### JSON Operations

| Operator/Function | Description | Example |
|-------------------|-------------|---------|
| `->>` | Extract as text | `properties ->> 'title'` |
| `->` | Extract as JSON | `properties -> 'meta' ->> 'author'` |
| `@>` | Contains | `properties @> '{"featured": true}'` |
| `JSON_VALUE()` | Typed extraction | `JSON_VALUE(properties, '$.price' RETURNING DOUBLE)` |
| `JSON_EXISTS()` | Check path exists | `JSON_EXISTS(properties, '$.seo.title')` |

### Full-Text Search

```sql
-- Basic search
WHERE to_tsvector('english', properties ->> 'body')
   @@ to_tsquery('english', 'rust & performance')

-- Boolean operators: & (AND), | (OR), ! (NOT)
WHERE ... @@ to_tsquery('english', '(rust | python) & !java')

-- Prefix search
WHERE ... @@ to_tsquery('english', 'perform:*')

-- Ranking
ts_rank(tsvector, tsquery) AS rank
ts_rank_cd(tsvector, tsquery) AS rank  -- coverage density
```

## SQL/PGQ (Graph Queries)

### GRAPH_TABLE Syntax

```sql
SELECT author, article_count
FROM GRAPH_TABLE(
  MATCH (author:User)-[:authored]->(article:Article)
  WHERE article.status = 'published'
  COLUMNS (
    author.name AS author,
    COUNT(article.id) AS article_count
  )
)
ORDER BY article_count DESC;
```

### Pattern Matching

```sql
-- Node patterns
MATCH (n:Article)                    -- labeled node
MATCH (n:Article|Post)               -- multiple labels (OR)
MATCH (n WHERE n.featured = true)    -- with filter

-- Relationship patterns
MATCH (a)-[:follows]->(b)            -- directed, typed
MATCH (a)<-[:follows]-(b)            -- reversed
MATCH (a)-[:follows*1..3]->(b)       -- variable length (1-3 hops)
```

### Graph Mutations

```sql
-- Create relationship
RELATE
  FROM path='/articles/post-1'
  TO path='/tags/rust'
  TYPE 'tagged';

-- Remove relationship
UNRELATE
  FROM path='/articles/post-1'
  TO path='/tags/rust'
  TYPE 'tagged';
```

### Graph Algorithms

| Function | Description |
|----------|-------------|
| `degree(n)` | Total connections |
| `pageRank(n)` | Importance score |
| `shortestPath(a, b)` | Find shortest path |
| `distance(a, b)` | Hop count |
| `closeness(n)` | Centrality measure |
| `betweenness(n)` | Bridge importance |
| `communityId(n)` | Community detection |

## Query Pipeline

```
SQL String
    ↓
[Parse] → AST (sqlparser-rs)
    ↓
[Analyze] → TypedExpr, Schema validation
    ↓
[Plan] → LogicalPlan (relational operators)
    ↓
[Optimize] → Constant folding, hierarchy rewriting, projection pruning
    ↓
[Physical Plan] → Executable operators
    ↓
[Execute] → Stream<Row>
```

## Components

| Module | Description |
|--------|-------------|
| `parser` | SQL parsing with PostgreSQL dialect |
| `analyzer` | Type checking, name resolution, function registry |
| `logical_plan` | Relational algebra operators |
| `optimizer` | Query optimization passes |
| `physical_plan` | Executable plan generation and execution |

## Documentation

Additional documentation in [`docs/`](./docs/):

- [SQL/PGQ Features](./docs/SUPPORTED_PGQ_FEATURES.md) - Complete graph query reference
- [Full-Text Search](./docs/fulltext-search.md) - PostgreSQL-compatible FTS guide
- [JSON Operations](./docs/json-operations.md) - JSONB operators and functions
- [Pagination Patterns](./docs/pagination-quick-reference.md) - Cursor and offset pagination
- [Hierarchy Queries](./docs/children-queries-summary.md) - Child node query patterns

## Testing

```bash
# Run all tests
cargo test -p raisin-sql

# Run parser tests
cargo test -p raisin-sql --test parser_tests

# Run optimizer tests
cargo test -p raisin-sql optimizer
```

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
