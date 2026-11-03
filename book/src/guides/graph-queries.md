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

## RELATE, UNRELATE and NEIGHBORS semantics

- `RELATE FROM ... TO ... TYPE 't'` is idempotent: relating an existing
  `(source, type, target)` again answers `affected_rows 1` and leaves one
  relation (its weight is the last one written). It is never an error.
- `UNRELATE ... TYPE 't'` removes only that type; without `TYPE` it removes
  every relation between the pair. Removing a missing relation answers
  `affected_rows 0`.
- `NEIGHBORS(id, 'OUT' | 'IN' | 'BOTH' [, 'TYPE'])` lists the other end of
  each relation. A friendship is two relations (A to B and B to A); `IN` on A
  lists B only when B to A exists. With named arguments it returns the distinct
  nodes within `depth` hops instead; see
  [Neighbourhood queries](#neighbourhood-queries-neighbors-with-depth).
- Over `POST /api/sql/{repo}/{branch}` every execution error (an unknown node
  id, a refused permission) is answered as HTTP 400 `VALIDATION_FAILED` with
  the cause in `message`. A 400 on RELATE therefore means the RELATE did not
  happen (check the message), never "already related".
- Role conditions may walk relations with
  `node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 1..2`;
  `DEPTH n` alone means exactly `n` hops.

## Neighbourhood queries: `NEIGHBORS` with `depth`

"Friends of friends", "everyone within three introductions" and "is X within N
hops of me" are questions about **which nodes are reachable**, not about the
routes that reach them. `NEIGHBORS` answers them directly.

```sql
NEIGHBORS(start, direction [, relation_type]
          [, depth => n] [, min_depth => m] [, types => 'A,B'] [, max_nodes => k])
```

Without a named argument `NEIGHBORS` is the one-hop form described above: one row
per **relation**, with `relation_type` and `weight`. **Any named argument selects
the set form:** one row per **distinct node** within `depth` hops.

| Argument | Meaning | Default | Limits |
|---|---|---|---|
| `start` | node id, `path`, or `workspace:/path` | | an id is looked up in the query workspace first, then in every workspace |
| `direction` | `'OUT'`, `'IN'` or `'BOTH'` | | any other value is an error |
| third positional / `types =>` | relation types to follow, `'A'` or `'A,B'` (exact, case sensitive) | every type | giving both is an error unless they agree |
| `depth =>` | deepest hop to follow | `1` | 1 to 16 |
| `min_depth =>` | shallowest depth to **report**; nearer nodes are still walked through | `1` | 1 to `depth` |
| `max_nodes =>` | visited-node budget, the start included | `200000` | 1 to 5,000,000 |

| Column | Meaning |
|---|---|
| `id`, `path`, `node_type`, `name`, `properties`, ... | the node, as in the one-hop form |
| `depth` | the **shortest** distance from the start (a node is reported once) |
| `via_count` | the number of distinct nodes one level closer that reach it. At depth 2 this is the number of **mutual connections**. At depth 1 it is 1 |

Rows come back ordered by depth, then id. Unknown, repeated or non-integer
arguments are errors that list the valid names. The start node is never part of
the result, and a start that does not exist (or has no relations) gives no rows.

```sql
-- Friends of friends of Ann, who are not already her friends, with the number
-- of friends they have in common with her.
SELECT id, via_count
FROM NEIGHBORS('people:/ann', 'OUT', depth => 2, min_depth => 2, types => 'CONNECTED')
ORDER BY via_count DESC, id
LIMIT 20;

-- Is Bob within three hops of Ann? (membership: an empty result is "no")
SELECT depth
FROM NEIGHBORS('people:/ann', 'OUT', depth => 3, types => 'CONNECTED')
WHERE id = 'bob-node-id';

-- The people Ann and Bob both know: two one-hop neighbourhoods, joined.
SELECT a.id
FROM NEIGHBORS('ann-node-id', 'OUT', depth => 1, types => 'CONNECTED') a
JOIN NEIGHBORS('bob-node-id', 'OUT', depth => 1, types => 'CONNECTED') b ON a.id = b.id;
```

### How it works, and what it costs

The set form is a breadth-first search over the per-node relation indexes with a
visited set. Each node is read once, one seek of O(its degree), and reported once
at its shortest distance: the work is bounded by the size of the neighbourhood,
never by the number of paths through it and never by the number of relations in
the repository. A variable-length `GRAPH_TABLE` pattern enumerates one result per
**route**, which grows with the product of the degrees along the way.

The visited-node budget **fails, it does not truncate**. When the search would
visit more than `max_nodes` nodes it stops with an error naming the budget and
the remedies (lower `depth`, restrict `types`, raise `max_nodes`). A neighbourhood
cut short is indistinguishable from a complete one once it reaches the client, so
none is returned.

### Row-level security

The policy is the one `GRAPH_TABLE` documents: **endpoints only**. A node is
checked against the caller's permissions when it is about to be reported (and the
root node is skipped, as in the one-hop form). A node the caller cannot read is
left out of the result but is still walked through, so a friend of a friend is
found even if the friend in between is private, and `via_count` counts every
intermediate. A system caller (no auth context) is not filtered.

## `GRAPH_TABLE` with a pinned endpoint

A `WHERE` clause that pins a pattern variable to one node does not scan the
repository's relations any more. The matcher reads the pinned node's own relations
from the per-node index, O(its degree), instead of every relation of the type
from the global index, O(all relations of that type).

What counts as a pin: top-level `AND` conjuncts of the form
`var.id = 'text'`, `var.path = 'text'` or `var.workspace = 'text'`, in either
operand order. Nothing under `OR`, `NOT` or a function is used, nor is a
non-text literal (`a.id = 5` is never true and is left to the filter). The
`WHERE` clause is still evaluated, unchanged, on every row the narrowed read
produces: a pin can make a query faster, never different.

```sql
-- reads Ann's outgoing relations only
SELECT * FROM GRAPH_TABLE(
  MATCH (me:Person)-[:CONNECTED]->(f:Person)
  WHERE me.id = 'ann-node-id'
  COLUMNS (f.id AS friend));
```

* A single hop, and the first hop of a chain, use the pinned end for all three
  directions (`->`, `<-`, `-`) and for alternations (`-[:a|b]->`). For a chain
  the later hops already read per node.
* A variable-length pattern whose **source** is pinned expands outward from it,
  level by level, reading only the nodes within `depth - 1` hops, and runs the
  usual path search over that part of the graph. If the expansion passes 50,000
  nodes the query silently takes the global route instead; the answer is the same.
* `ANY SHORTEST` / `ALL SHORTEST` between two pinned nodes is answered by a
  breadth-first search, not by enumerating every path.
* A pin that names no existing node falls back to the global scan, so a relation
  that outlives its node is still found. Pin by an id that exists.
* A pin on the **target** of a variable-length pattern is not used (the path
  search starts from sources). Write the pattern the other way round, or use
  `NEIGHBORS(..., 'IN', ...)`.

### Recommended query shapes

| Question | Query |
|---|---|
| Friends of friends of me, with mutual counts | `NEIGHBORS(me, 'OUT', depth => 2, min_depth => 2, types => 'T')`; `via_count` is the mutual count |
| Mutual connections with one person | `NEIGHBORS(me, 'OUT', depth => 1, types => 'T') a JOIN NEIGHBORS(them, 'OUT', depth => 1, types => 'T') b ON a.id = b.id` (use `'IN'` for the second when relations are not stored both ways) |
| Is X within N hops of me | `SELECT depth FROM NEIGHBORS(me, 'OUT', depth => N, types => 'T') WHERE id = X` |
| How are we connected | `GRAPH_TABLE(MATCH ANY SHORTEST p = (a)-[:T]->{1,N}(b) WHERE a.id = 'me' AND b.id = 'them' COLUMNS (path_length(p) AS hops, ...))` |
| Who are my direct connections | `NEIGHBORS(me, 'OUT', depth => 1, types => 'T')` |

`GROUP BY` and `COUNT` work over `GRAPH_TABLE` output and, with a pinned start,
touch only the neighbourhood, but a `GRAPH_TABLE` row is one **route**: counting
`MATCH (me)-[:T]->(f)-[:T]->(c) WHERE me.id = '...'` grouped by `c` counts routes
to `c`, including routes to people who are already direct connections, and every
route is materialised. Prefer the set form for neighbourhood questions and keep
`GRAPH_TABLE` for patterns that name node types or properties along the way.

### Measured cost

One node, a random graph, `(a)-[:k]->(b) WHERE a.id = '...'`. "Relations read" is
what each access path returned from the relation indexes. Timings are the matcher
alone (no SQL parse, no `WHERE`) on an **unoptimised** build, so read the ratio
rather than the milliseconds.

| Query | 10,000 relations | 100,000 relations |
|---|---|---|
| 1 hop, source pinned, before (global scan) | 9,946 read, 54 ms | 99,797 read, 566 ms |
| 1 hop, source pinned, now (per-node) | 5 read, 0.3 ms | 30 read, 0.6 ms |
| 1 hop, target pinned, now | 13 read, 0.3 ms | 24 read, 0.5 ms |
| `{1,2}` hops, source pinned, before | 9,946 read, **fails: path cap** | 99,797 read, **fails: path cap** |
| `{1,2}` hops, source pinned, now | 54 read, 0.7 ms (54 rows) | 621 read, 7 ms (621 rows) |
| `NEIGHBORS(..., depth => 2)` | 54 read, 0.5 ms (49 nodes) | 621 read, 5 ms (556 nodes) |

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
