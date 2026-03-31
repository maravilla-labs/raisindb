# Graph Queries and Algorithms

RaisinDB supports graph queries through two mechanisms: the Cypher query language parser (`raisin-cypher-parser`) and a graph algorithm engine (`raisin-graph-algorithms`). Together they enable pattern matching on relationships and global graph analysis like PageRank and community detection.

## Cypher Query Language

The `raisin-cypher-parser` crate provides a complete openCypher parser built with nom. It integrates with the SQL engine so you can run graph queries alongside relational queries.

### Basic Pattern Matching

```cypher
-- Find all Person nodes
MATCH (n:Person) RETURN n.name

-- Find relationships
MATCH (a:Person)-[:KNOWS]->(b:Person)
RETURN a.name, b.name

-- Filter with WHERE
MATCH (n:Person {name: 'Alice'})
RETURN n.age
```

### Query Parsing

```rust
use raisin_cypher_parser::{parse_query, parse_expr};

// Parse a complete Cypher query
let query = parse_query("MATCH (n:Person {name: 'Alice'}) RETURN n.age")?;

// Parse individual expressions
let expr = parse_expr("n.name = 'Alice' AND n.age > 25")?;
```

### AST Structure

The parser produces a strongly-typed AST:

- **`Query`** -- top-level query with a list of clauses
- **`Clause`** -- MATCH, WHERE, RETURN, ORDER BY, LIMIT, CREATE, DELETE, SET, etc.
- **`GraphPattern`** -- pattern elements (nodes and relationships)
- **`NodePattern`** -- `(variable:Label {properties})`
- **`RelPattern`** -- `-[variable:TYPE {properties}]->`
- **`Expr`** -- expressions with operators, literals, function calls, property access

### Pattern Elements

```rust
use raisin_cypher_parser::{NodePattern, RelPattern, Direction};

// Node pattern: (n:Person {age: 30})
// RelPattern: -[r:KNOWS {since: 2020}]->

// Direction variants:
// Direction::Outgoing  ->
// Direction::Incoming  <-
// Direction::Both      --
```

### SQL Integration

Cypher queries can be executed through the SQL engine:

```sql
-- Call Cypher from SQL
SELECT * FROM cypher('MATCH (n:Person) RETURN n.name, n.age')

-- SQL/PGQ graph pattern matching (ISO standard)
SELECT * FROM GRAPH_TABLE (social_graph
    MATCH (p:Person)-[k:KNOWS]->(f:Person)
    COLUMNS (p.name AS person, f.name AS friend)
)
```

### Error Handling

The parser provides detailed error messages with line and column information:

```rust
let result = parse_query("MATCH (n RETURN n");
// Error: Syntax error at line 1, column 10: expected ')', found 'RETURN'
```

## Graph Algorithms

The `raisin-graph-algorithms` crate implements a "Graph Projection" engine for running global graph algorithms on subgraphs loaded from storage.

### Architecture

The algorithm pipeline works in five stages:

1. **Projection** -- scan storage to find relevant nodes and edges
2. **Mapping** -- map string IDs (UUIDs/paths) to dense integers (u32)
3. **CSR Construction** -- build a Compressed Sparse Row graph (petgraph)
4. **Execution** -- run algorithms with rayon parallelism
5. **Writeback** -- map results back to string IDs and update node properties

### Graph Projection

```rust
use raisin_graph_algorithms::GraphProjection;

// Build a projection from nodes and edges
let nodes = vec!["user-1".into(), "user-2".into(), "user-3".into()];
let edges = vec![
    ("user-1".into(), "user-2".into()),
    ("user-2".into(), "user-3".into()),
    ("user-1".into(), "user-3".into()),
];

let projection = GraphProjection::from_parts(nodes, edges);
```

The projection maps RaisinDB string IDs to contiguous u32 indices for efficient algorithm execution.

### Available Algorithms

#### PageRank

Computes the importance of each node based on the link structure:

```rust
use raisin_graph_algorithms::algorithms::page_rank;

let scores = page_rank(&projection, 0.85, 100, 1e-6); // damping=0.85, max_iter=100, tolerance
// Returns: HashMap<String, f64> mapping node IDs to PageRank scores
```

#### Community Detection

**Louvain Algorithm** -- detects communities by optimizing modularity:

```rust
use raisin_graph_algorithms::algorithms::louvain;

let communities = louvain(&projection, 100, 1.0); // iterations, resolution
// Returns: HashMap<String, usize> mapping node IDs to community IDs
```

**Weakly Connected Components** -- finds connected subgraphs:

```rust
use raisin_graph_algorithms::algorithms::weakly_connected_components;

let components = weakly_connected_components(&projection);
```

#### Pathfinding

**A\* Search** -- finds the shortest path between two nodes:

```rust
use raisin_graph_algorithms::algorithms::astar;

let path = astar(
    &projection, "user-1", "user-3",
    |_from, _to| 1.0,  // edge_cost function
    |_node| 0.0,        // heuristic function
);
// Returns: Option<Vec<String>> -- the shortest path
```

**K-Shortest Paths** -- finds the top-k shortest paths:

```rust
use raisin_graph_algorithms::algorithms::k_shortest_paths;

let paths = k_shortest_paths(
    &projection, "user-1", "user-3", 3,
    |_from, _to| 1.0,  // edge_cost function
);
```

#### Triangle Count

Counts the number of triangles in the graph, useful for measuring clustering:

```rust
use raisin_graph_algorithms::algorithms::triangle_count;

let count = triangle_count(&projection);
```

### Writeback

Algorithm results can be written back to node properties in storage:

```rust
use raisin_graph_algorithms::writeback;

// Write PageRank scores back to nodes as a property
writeback::write_results(&storage, &scores, "pagerank_score").await?;
```

### Performance

- The CSR (Compressed Sparse Row) format provides cache-friendly memory layout
- Rayon is used for parallel algorithm execution across CPU cores
- Graph projection uses dense integer IDs to avoid hash lookups during computation
