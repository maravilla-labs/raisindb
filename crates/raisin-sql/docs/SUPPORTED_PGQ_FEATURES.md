# SQL/PGQ Supported Features

> **TODO**: Review and update this documentation to ensure accuracy with current implementation.

Complete reference for RaisinDB's SQL/PGQ implementation.

## Table of Contents

1. [GRAPH_TABLE Syntax](#graph_table-syntax)
2. [Pattern Matching](#pattern-matching)
3. [COLUMNS Clause](#columns-clause)
4. [Graph Mutations (RELATE/UNRELATE)](#graph-mutations)
5. [SQL Integration](#sql-integration)
6. [Graph Functions](#graph-functions)
7. [Graph Algorithms](#graph-algorithms)

---

## GRAPH_TABLE Syntax

### Basic Structure

```sql
SELECT [columns]
FROM GRAPH_TABLE([graph_name]
  MATCH graph_pattern
  [WHERE filter_conditions]
  COLUMNS (column_expressions)
) [AS alias]
[WHERE sql_conditions]
[ORDER BY columns]
[LIMIT n [OFFSET m]];
```

### Graph Name

| Syntax | Description |
|--------|-------------|
| `GRAPH_TABLE(MATCH ...)` | Uses default `NODES_GRAPH` |
| `GRAPH_TABLE(NODES_GRAPH MATCH ...)` | Explicit default graph |
| `GRAPH_TABLE(custom_graph MATCH ...)` | Named graph (future) |

### Complete Example

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
WHERE article_count > 5
ORDER BY article_count DESC
LIMIT 10;
```

---

## Pattern Matching

### Node Patterns

| Pattern | Description | Example |
|---------|-------------|---------|
| `(n)` | Any node | `MATCH (n)` |
| `(n:Label)` | Node with label | `MATCH (n:Article)` |
| `(n:Label1\|Label2)` | Multiple labels (OR) | `MATCH (n:Article\|Post)` |
| `(n:Label WHERE cond)` | With inline filter | `MATCH (n:Article WHERE n.featured = true)` |
| `(n WHERE cond)` | Filter without label | `MATCH (n WHERE n.id = 'x')` |

### Relationship Patterns

| Pattern | Direction | Description |
|---------|-----------|-------------|
| `(a)-[r]->(b)` | Right | a points to b |
| `(a)<-[r]-(b)` | Left | b points to a |
| `(a)-[r]-(b)` | Any | Either direction |
| `(a)-[:TYPE]->(b)` | Right, typed | Specific relation type |
| `(a)-[:T1\|T2]->(b)` | Right, multi-type | Multiple types (OR) |
| `(a)-[]->(b)` | Right, anonymous | No variable binding |

### Variable-Length Paths

| Pattern | Hops | Description |
|---------|------|-------------|
| `*` | 1..10 | Default bounded (1 to 10) |
| `*n` | exactly n | Exactly n hops |
| `*n..m` | n to m | Range inclusive |
| `*n..` | n to 10 | Minimum n, max default |
| `*..m` | 1 to m | At least 1, max m |

**Examples:**
```sql
-- Exactly 2 hops (friend of friend)
MATCH (a)-[:follows*2]->(b)

-- 1 to 3 hops
MATCH (a)-[:follows*1..3]->(b)

-- Any path length (capped at 10)
MATCH (a)-[:follows*]->(b)
```

**Performance Notes:**
- Default max depth: 10
- Max results per path query: 10,000
- Cycle detection: automatic
- Algorithm: DFS (memory efficient)

### Complex Patterns

```sql
-- Chain of relationships
MATCH (a:User)-[:follows]->(b:User)-[:likes]->(c:Post)

-- Triangle pattern
MATCH (a:User)-[:follows]->(b:User)-[:follows]->(c:User)-[:follows]->(a)

-- Multiple independent patterns (implicit CROSS JOIN)
MATCH (a:User), (b:Post)
WHERE a.id = b.author_id
```

---

## COLUMNS Clause

### Property Resolution

Properties are resolved in this order:

1. **System fields** (direct access):
   - `id` - Node UUID
   - `workspace` - Workspace name
   - `node_type` - Node type string
   - `path` - Hierarchical path
   - `created_at` - Creation timestamp
   - `updated_at` - Last update timestamp

2. **User properties** (from `properties` JSONB):
   - `title` -> `properties.title`
   - `author` -> `properties.author`
   - Any custom property

### Syntax

```sql
COLUMNS (
  -- Direct access
  node.id,
  node.workspace,
  node.path,

  -- Property access (auto-resolved)
  node.title,
  node.author,

  -- With alias
  node.title AS article_title,

  -- Expressions
  CONCAT(author.first_name, ' ', author.last_name) AS full_name,

  -- Aggregates (when grouped)
  COUNT(article.id) AS article_count,

  -- Graph functions
  degree(node) AS connections
)
```

### Supported Expressions in COLUMNS

| Type | Examples |
|------|----------|
| Column reference | `a.id`, `a.title` |
| Aliased column | `a.title AS name` |
| String functions | `CONCAT(a, b)`, `UPPER(a.name)` |
| Numeric expressions | `a.price * 1.1` |
| Aggregates | `COUNT(*)`, `SUM(a.value)`, `AVG(a.score)` |
| Graph functions | `degree(n)`, `shortestPath(a, b)` |
| CASE expressions | `CASE WHEN a.status = 'active' THEN 1 ELSE 0 END` |

---

## Graph Mutations

### RELATE Statement

Creates a directed relationship between nodes.

```sql
RELATE [IN BRANCH 'branch_name']
  FROM node_reference [IN WORKSPACE 'workspace']
  TO node_reference [IN WORKSPACE 'workspace']
  [TYPE 'relation_type']
  [WEIGHT numeric_value];
```

**Node References:**
- `path='/content/article-1'` - By path
- `id='uuid-here'` - By ID

**Examples:**
```sql
-- Simple relationship
RELATE
  FROM path='/articles/post-1'
  TO path='/tags/rust'
  TYPE 'tagged';

-- Cross-workspace with weight
RELATE
  FROM path='/content/page' IN WORKSPACE 'main'
  TO path='/assets/hero.jpg' IN WORKSPACE 'media'
  TYPE 'references'
  WEIGHT 2.0;

-- By ID
RELATE
  FROM id='abc123'
  TO id='def456'
  TYPE 'links_to';
```

### UNRELATE Statement

Removes relationships between nodes.

```sql
UNRELATE [IN BRANCH 'branch_name']
  FROM node_reference [IN WORKSPACE 'workspace']
  TO node_reference [IN WORKSPACE 'workspace']
  [TYPE 'relation_type'];
```

**Examples:**
```sql
-- Remove specific type
UNRELATE
  FROM path='/articles/post-1'
  TO path='/tags/rust'
  TYPE 'tagged';

-- Remove all relations between nodes (when TYPE omitted)
UNRELATE
  FROM path='/articles/post-1'
  TO path='/tags/rust';
```

### Defaults

| Field | Default Value |
|-------|---------------|
| TYPE | `'references'` |
| WEIGHT | `NULL` |
| WORKSPACE | Current context workspace |
| BRANCH | Current context branch |

---

## SQL Integration

### JOIN with Regular Tables

```sql
-- Graph result joined with metadata table
SELECT g.*, m.last_viewed
FROM GRAPH_TABLE(
  MATCH (a:Article)-[:references]->(b:Article)
  COLUMNS (a.id AS source_id, b.id AS target_id)
) g
LEFT JOIN article_metadata m ON g.source_id = m.article_id;
```

### Subqueries

```sql
-- Graph query in WHERE EXISTS
SELECT * FROM articles a
WHERE EXISTS (
  SELECT 1 FROM GRAPH_TABLE(
    MATCH (src:Article)-[:references]->(tgt:Article)
    WHERE src.id = a.id
    COLUMNS (tgt.id)
  )
);
```

### CTEs (Common Table Expressions)

```sql
-- Using graph results in CTE
WITH popular_authors AS (
  SELECT author_id, article_count
  FROM GRAPH_TABLE(
    MATCH (author:User)-[:authored]->(article:Article)
    COLUMNS (author.id AS author_id, COUNT(*) AS article_count)
  )
  WHERE article_count > 10
)
SELECT u.*, pa.article_count
FROM users u
JOIN popular_authors pa ON u.id = pa.author_id;
```

### Aggregation

```sql
-- Aggregates work naturally
SELECT category, COUNT(*) as connections
FROM GRAPH_TABLE(
  MATCH (a:Article)-[:references]->(b:Article)
  COLUMNS (a.category)
)
GROUP BY category
HAVING COUNT(*) > 5
ORDER BY connections DESC;
```

---

## Graph Functions

### Real-Time Functions

These execute directly during query without full graph scan:

| Function | Signature | Description | Complexity |
|----------|-----------|-------------|------------|
| `degree(node)` | `degree(n) -> Integer` | Total in + out relationships | O(1) |
| `inDegree(node)` | `inDegree(n) -> Integer` | Incoming relationships count | O(k) |
| `outDegree(node)` | `outDegree(n) -> Integer` | Outgoing relationships count | O(k) |

**Examples:**
```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (n:User)
  COLUMNS (n.name, degree(n) AS connections)
)
ORDER BY connections DESC
LIMIT 10;
```

### Path Functions

These build adjacency graph per query execution:

| Function | Signature | Description |
|----------|-----------|-------------|
| `shortestPath(start, end)` | `shortestPath(a, b) -> Path` | Single shortest path via BFS |
| `shortestPath(start, end, maxDepth)` | `shortestPath(a, b, 5) -> Path` | With depth limit |
| `allShortestPaths(start, end)` | `allShortestPaths(a, b) -> Path[]` | All minimum-length paths (max 100) |
| `distance(start, end)` | `distance(a, b) -> Integer` | Hop count (-1 if no path) |

**Path Object Structure:**
```json
{
  "nodes": ["id1", "id2", "id3"],
  "relationships": ["rel1", "rel2"],
  "length": 2
}
```

**Examples:**
```sql
-- Find shortest path
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User), (b:User)
  WHERE a.id = 'alice' AND b.id = 'bob'
  COLUMNS (shortestPath(a, b) AS path)
);

-- Distance between nodes
SELECT * FROM GRAPH_TABLE(
  MATCH (a:User), (b:User)
  WHERE a.id = 'alice' AND b.id = 'bob'
  COLUMNS (distance(a, b) AS hops)
);
```

---

## Graph Algorithms

### Real-Time Algorithms

Can be called in queries, but scan full graph:

| Algorithm | Function | Description | Complexity |
|-----------|----------|-------------|------------|
| **Closeness Centrality** | `closeness(n)` | How close node is to all others | O(V + E) per node |
| **Betweenness Centrality** | `betweenness(n)` | Bridge/bottleneck importance | O(V * (V + E)) |
| **Connected Components** | `componentId(n)` | Which component node belongs to | O(V + E) |
| **Component Count** | `componentCount()` | Number of weakly connected components | O(V + E) |
| **Community Detection** | `communityId(n)` | Community via label propagation | O(k * E) |
| **Community Count** | `communityCount()` | Number of detected communities | O(k * E) |
| **PageRank** | `pageRank(n)` | Node importance score | O(k * (V + E)) |

**Examples:**
```sql
-- Find bridge nodes (high betweenness)
SELECT * FROM GRAPH_TABLE(
  MATCH (n:User)
  COLUMNS (n.name, betweenness(n) AS importance)
)
WHERE importance > 0.1
ORDER BY importance DESC;

-- Find communities
SELECT * FROM GRAPH_TABLE(
  MATCH (n:User)
  COLUMNS (n.name, communityId(n) AS community)
)
ORDER BY community, n.name;

-- PageRank with custom parameters
SELECT * FROM GRAPH_TABLE(
  MATCH (n:Article)
  COLUMNS (n.title, pageRank(n, 0.85, 100) AS rank)
)
ORDER BY rank DESC
LIMIT 10;
```

### Batch/Job Algorithms (Future)

These algorithms are computationally expensive and should be pre-computed via background jobs:

| Algorithm | Description | Use Case | Status |
|-----------|-------------|----------|--------|
| **PageRank** | Iterative importance scoring | Search ranking, recommendations | Available (real-time), Job version planned |
| **Label Propagation** | Community detection | User segmentation, clustering | Available (real-time), Job version planned |
| **Betweenness Centrality** | Bridge node detection | Network analysis | Available (real-time), Job version planned |

**Job-Based Algorithm Design (Planned):**

```sql
-- Create a computed graph metric (future syntax)
CREATE GRAPH METRIC pagerank_scores ON NODES_GRAPH
  USING PAGERANK(damping_factor := 0.85, max_iterations := 100)
  REFRESH INTERVAL '1 hour';

-- Query pre-computed values
SELECT * FROM GRAPH_TABLE(
  MATCH (n:Article)
  COLUMNS (n.title, n.$pagerank_scores AS rank)
)
ORDER BY rank DESC;
```

### Algorithm Details

#### PageRank
- **File**: `crates/raisin-sql-execution/src/physical_plan/cypher/algorithms/pagerank.rs`
- **Default damping**: 0.85
- **Default iterations**: 100
- **Convergence threshold**: 0.0001
- **Returns**: Float 0.0-1.0

#### Closeness Centrality
- **File**: `crates/raisin-sql-execution/src/physical_plan/cypher/algorithms/centrality.rs`
- **Formula**: (N-1) / sum_of_distances
- **Returns**: Float 0.0-1.0 (1.0 = maximally central)

#### Betweenness Centrality
- **File**: `crates/raisin-sql-execution/src/physical_plan/cypher/algorithms/betweenness.rs`
- **Algorithm**: Brandes' algorithm
- **Returns**: Float 0.0-1.0

#### Connected Components
- **File**: `crates/raisin-sql-execution/src/physical_plan/cypher/algorithms/connected_components.rs`
- **Type**: Weakly connected (treats directed as undirected)
- **Returns**: Integer component ID (0-indexed)

#### Label Propagation (Community Detection)
- **File**: `crates/raisin-sql-execution/src/physical_plan/cypher/algorithms/label_propagation.rs`
- **Default iterations**: 100
- **Randomize ties**: true
- **Reference**: Raghavan, Albert, and Kumara (2007)
- **Returns**: Integer community ID

---

## Comparison with Other Systems

### vs. Neo4j Cypher

| Feature | RaisinDB SQL/PGQ | Neo4j Cypher |
|---------|------------------|--------------|
| Query syntax | `GRAPH_TABLE(MATCH ... COLUMNS ...)` | `MATCH ... RETURN ...` |
| Create relationships | `RELATE FROM ... TO ...` | `CREATE (a)-[:REL]->(b)` |
| Delete relationships | `UNRELATE FROM ... TO ...` | `DELETE r` |
| Graph definition | Implicit `NODES_GRAPH` | Single implicit graph |
| SQL integration | Native (same query) | Requires APOC or connectors |
| Standard | ISO SQL:2023 | Proprietary |

### vs. Apache AGE

| Feature | RaisinDB SQL/PGQ | Apache AGE |
|---------|------------------|------------|
| Query syntax | `GRAPH_TABLE(...)` | `cypher('graph', $$ ... $$)` |
| Property access | `COLUMNS (n.name)` | `RETURN n.name` |
| Graph mutations | `RELATE/UNRELATE` | Cypher CREATE/DELETE |
| Return type | SQL rows | `agtype` |

### vs. Oracle SQL/PGQ

| Feature | RaisinDB | Oracle 23ai |
|---------|----------|-------------|
| Default graph | `NODES_GRAPH` (automatic) | Must `CREATE PROPERTY GRAPH` |
| Property resolution | Automatic from `properties` | Defined in graph schema |
| Relationship mutations | `RELATE/UNRELATE` | Standard DML on edge tables |

---

## Limitations

### Current Limitations

1. **Single implicit graph**: Only `NODES_GRAPH` supported (no `CREATE PROPERTY GRAPH` yet)
2. **No OPTIONAL MATCH**: All patterns must match
3. **No graph mutations in GRAPH_TABLE**: Use `RELATE/UNRELATE` separately
4. **Algorithm performance**: Full graph scan algorithms may be slow on large graphs

### Planned Features

- [ ] `CREATE PROPERTY GRAPH` for custom graph views
- [ ] `OPTIONAL MATCH` for optional patterns
- [ ] Pre-computed algorithm results via jobs
- [ ] Streaming/incremental algorithm updates
- [ ] Graph partitioning hints

---

## Performance Considerations

### Pattern Matching

| Pattern | Performance | Notes |
|---------|-------------|-------|
| Single hop | Fast | Direct index lookup |
| Multi-hop fixed | O(k^n) | k = avg degree, n = hops |
| Variable-length | Bounded | Max depth limits explosion |
| Full graph scan | Slow | Algorithms like PageRank |

### Recommendations

1. **Always filter early**: Use `WHERE` in pattern, not after
2. **Limit variable paths**: Prefer `*1..3` over `*`
3. **Use indexes**: Label filters use `node_type` index
4. **Pre-compute heavy algorithms**: Use jobs for PageRank, etc.

---

## Version History

| Version | Changes |
|---------|---------|
| 0.1.0 | Initial SQL/PGQ support with GRAPH_TABLE |
