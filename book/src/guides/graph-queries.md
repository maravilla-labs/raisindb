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
| `max_nodes =>` | visited-node budget, the start included | `200000` | 1 to 1,000,000; above 200,000 only for a system or system-admin caller (see below) |

| Column | Meaning |
|---|---|
| `id`, `path`, `node_type`, `name`, `properties`, ... | the node, as in the one-hop form |
| `depth` | the **shortest** distance from the start (a node is reported once) |
| `via_count` | the number of distinct nodes one level closer that reach it **and that the caller may read**. At depth 2 this is the number of **mutual connections**. At depth 1 it is 1 |

Rows come back ordered by depth, then id. Unknown, repeated or non-integer
arguments are errors that list the valid names. The start node is never part of
the result, and a start that does not exist, has no relations, or that the caller
may not read gives no rows.

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

### Limits

Three limits stop the search, and each one **fails the query, it does not
truncate** it (a neighbourhood cut short is indistinguishable from a complete one
once it reaches the client, so none is returned):

| Limit | Value | Error |
|---|---|---|
| visited nodes, the start included | `max_nodes` (default 200,000) | `neighbourhood too large: narrow the query with depth, types or max_nodes` |
| relations read from the per-node indexes (every relation a seek returns, followed or not) | 200,000, or `max_nodes` when that is larger (the ceiling every scan has) | the same text |
| wall-clock time | 3 seconds (the time limit every scan has) | `neighbourhood search ran out of time: narrow the query with depth, types or max_nodes` |

`max_nodes` accepts 1 to 1,000,000, but only a system context or a system admin
is given more than the default 200,000. Any other caller (an ordinary user, an
anonymous visitor, a query with no auth context) is held to 200,000 whatever it
asks for: asking for more is not an error, the search simply fails at 200,000.
Lowering `max_nodes` is always allowed.

The errors carry no numbers: not how many nodes were visited, not how many
relations were read, no node ids. Those counts include nodes the caller may not
read, so they are logged on the server at debug level only.

Rows are produced as the search goes, a few hundred nodes at a time, so a large
result is not loaded into memory before the first row is returned.

### Row-level security

* **The start** is checked first. A start the caller may not read gives exactly
  what a start that does not exist gives: no rows, no error, and nothing is read
  from it.
* **Reported nodes** are checked against the caller's permissions (the root node
  is never reported, as in the one-hop form). A node the caller cannot read is
  left out of the result.
* **Walked-through nodes**: the policy is the one `GRAPH_TABLE` documents,
  **endpoints only**. A node the caller cannot read is still walked through, so
  a friend of a friend is found even if the friend in between is private.
* **`via_count`** counts only nodes one level closer that the caller may read. A
  node reached only through private nodes is reported with `via_count = 0`.

What a caller can still learn about private nodes, by design: a reported node's
`depth` is its shortest distance through any node, private ones included (as a
`GRAPH_TABLE` path through a private middle node still produces its row), and
whether the search hits a limit depends on the whole neighbourhood, private
nodes included (but the error never says by how much). A system context is not
filtered.

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
  nodes, reads more than 200,000 relations, or runs for more than 3 seconds, the
  query takes the global route instead; the answer is the same, and the extra
  work the narrowed read can add is bounded.
* `ANY SHORTEST` / `ALL SHORTEST` between two pinned nodes is answered by a
  breadth-first search over that expanded part of the graph (so it has the same
  limits), not by enumerating every path; `ALL SHORTEST` still stops at the
  10,000 path cap.
* In a single hop, a pin that names no existing node falls back to the global
  scan, so a relation that outlives its node is still found; the rows are the
  ones the pinned node would have given. In a variable-length pattern a pinned
  source or `SHORTEST` target that names no existing node gives no rows, exactly
  like an existing node without relations: whether a pinned id exists never
  changes the rows or turns an answer into an error. Pin by an id that exists.
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

Random graph, one start node of about 10 (10,000 relations) and about 20
(100,000 relations) outgoing relations, `SELECT ... GRAPH_TABLE(... WHERE a.id = '...')`
over HTTP against an **unoptimised** `raisin-server` (compare the two columns, not
the absolute milliseconds). "Before" is the same query on the previous engine.

| Query | 10,000 relations: before | now | 100,000 relations: before | now |
|---|---|---|---|---|
| 1 hop, source pinned | 100 ms | 3 ms | 934 ms | 4 ms |
| 1 hop, target pinned | 100 ms | 3 ms | 932 ms | 3 ms |
| 2-hop chain `(a)->(m)->(c)`, source pinned | 1,467 ms | 10 ms | 28,274 ms | 53 ms |
| `{1,2}` hops, source pinned | **error: path cap** | 10 ms (111 rows) | **error: path cap** | 50 ms (666 rows) |
| `ANY SHORTEST {1,4}`, both ends pinned | **error: path cap** | 76 ms | **error: path cap** | 870 ms |
| `NEIGHBORS(id, 'OUT', 'T')` of a node with no relations | 69 ms | 2 ms | 684 ms | 2 ms |
| `NEIGHBORS(id, 'OUT', depth => 2, min_depth => 2)` | not available | 12 ms (98 nodes) | not available | 59 ms (593 nodes) |

The same queries counted at the storage layer (relations the matcher read, before
`WHERE`): the global scan reads every relation of the type (9,946 and 99,797 here),
the per-node read 5 and 30 for a one-hop query, 54 and 621 for two hops.
A 3-hop `NEIGHBORS` from a start in a 5,000-node, 100,000-relation graph reaches
almost every node and takes about 0.5 s, which is the size of the answer, not of
the repository.

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
