# GRAPH_TABLE - Graph Analytics in RaisinDB

RaisinDB provides powerful graph analytics capabilities through the `GRAPH_TABLE` SQL extension, enabling you to run graph algorithms on your data and query relationship patterns efficiently.

## Overview

RaisinDB stores relationships between nodes as first-class citizens, creating a natural graph structure. The `GRAPH_TABLE` feature allows you to:

- Run graph algorithms (PageRank, community detection, etc.) on your data
- Query relationship patterns using graph pattern matching
- Precompute and cache algorithm results for fast queries
- Analyze social networks, knowledge graphs, and hierarchical data

## Quick Start

```sql
-- Find the most influential users by PageRank
SELECT n.name, pageRank(n) as influence
FROM GRAPH_TABLE (social_graph
  MATCH (n:User)
  COLUMNS (n.name, n)
) ORDER BY influence DESC LIMIT 10;

-- Detect communities in your network
SELECT n.name, louvain(n) as community_id
FROM GRAPH_TABLE (social_graph
  MATCH (n:User)-[:FRIENDS_WITH]-(m:User)
  COLUMNS (n.name, n)
) GROUP BY community_id;

-- Find paths between two nodes
-- A path variable is addressed through accessors; there is no PATH column type,
-- so COLUMNS (p) is a compile error naming them.
SELECT hops, nodes
FROM GRAPH_TABLE (knowledge_graph
  MATCH p = (a:Concept)-[:RELATED_TO]-{1,3}(b:Concept)
  WHERE a.name = 'Machine Learning' AND b.name = 'Neural Networks'
  COLUMNS (path_length(p) AS hops, nodes(p) AS nodes)
);
```

## MATCH Grammar

```text
match_clause   := MATCH top_level_path { ',' top_level_path }
top_level_path := [ path_variable '=' ] [ selector ] [ restrictor ]
                  [ path_variable '=' ] path_expr
selector       := ANY SHORTEST | ANY CHEAPEST | ALL SHORTEST | ANY
restrictor     := WALK | TRAIL | ACYCLIC
path_expr      := node_pattern { edge_pattern node_pattern }
node_pattern   := '(' [ variable ] [ ':' label { '|' label } ] ')'
edge_pattern   := '-[' edge_body ']->' [ quantifier ]
                | '<-[' edge_body ']-'  [ quantifier ]
                | '-['  edge_body ']-'  [ quantifier ]
edge_body      := [ variable ] [ ':' type { '|' type } ] [ COST expr ]
quantifier     := '{' m [ ',' [ n ] ] '}' | '*' | '+' | '?'
```

### Path variables

`MATCH p = ...` binds the whole path so the [path accessors](#path-accessors)
can address it. The variable may be written before the selector
(`p = ANY SHORTEST (...)`) or after the restrictor
(`ANY SHORTEST TRAIL p = (...)`) — both spellings are in circulation. Selector
before restrictor is fixed; the other order is a parse error naming the fix.

### Selectors

| Selector | Meaning |
|----------|---------|
| *(none)* | Every path matching the pattern |
| `ANY` | One arbitrary path per endpoint pair (**not** minimum-hop) |
| `ANY SHORTEST` | One minimum-hop path per endpoint pair |
| `ALL SHORTEST` | Every minimum-hop path per endpoint pair |
| `ANY CHEAPEST` | One minimum-cost path — **RaisinDB extension**, requires `COST` |

`SHORTEST k`, `SHORTEST k GROUP` and `ANY k` are not supported yet and parse to
a named error rather than being silently accepted.

### Restrictors

| Restrictor | Meaning |
|------------|---------|
| `WALK` | No distinctness requirement; nodes and edges may repeat |
| `TRAIL` | Edge-distinct — no edge traversed twice |
| `ACYCLIC` | Node-distinct — no node visited twice |

**The default is `ACYCLIC`**, chosen to preserve existing behaviour: the engine
has always skipped already-visited nodes during variable-length traversal. The
ISO default could not be confirmed, so `WALK` must be requested explicitly.
`SIMPLE` is not supported yet — it differs from `ACYCLIC` only by permitting a
closed walk, and shipping a subtly-wrong `SIMPLE` is worse than not shipping it.

### Quantifiers

The canonical form is the postfix brace form, written **after** the arrow:

| Quantifier | Hops |
|------------|------|
| `->{2}` | exactly 2 |
| `->{1,3}` | 1 to 3 inclusive |
| `->{2,}` | 2 or more (unbounded) |
| `->*` | `{0,}` |
| `->+` | `{1,}` |
| `->?` | `{0,1}` |

**Rule Q-SCOPE:** an unbounded quantifier (`*`, `+`, `{m,}`) must be contained
in the scope of a selector or a restrictor. `MATCH (a)-[:t]->*(b)` is a parse
error; `MATCH ANY SHORTEST p = (a)-[:t]->*(b)` and `MATCH TRAIL (a)-[:t]->*(b)`
are not. Bounded quantifiers need neither. Even under a selector or restrictor
an unbounded quantifier is capped at 10 hops.

#### Deprecated: the Cypher-style quantifier

The old form written **inside** the brackets is still accepted:

| Deprecated | Canonical |
|------------|-----------|
| `-[:t*2]->` | `-[:t]->{2}` |
| `-[:t*1..3]->` | `-[:t]->{1,3}` |
| `-[:t*2..]->` | `-[:t]->{2,}` |
| `-[:t*]->` | `-[:t]->{1,}` |

The two forms occupy different syntactic slots, so they are never ambiguous,
but they are not interchangeable: **legacy `*` means `{1,}` while standard `*`
means `{0,}`**. The legacy form is exempt from rule Q-SCOPE (it predates it and
is capped at 10 hops instead), and using it emits a deprecation warning that
also appears in `EXPLAIN`. Cypher (`raisin-cypher-parser`) is a different
dialect where `*1..3` is native and not deprecated.

### Inline WHERE is rejected

`MATCH (n:User WHERE n.active = true)` and
`MATCH (a)-[r:t WHERE r.since > 2020]->(b)` are parse errors. They used to parse
into a field no execution path ever read, so the predicate was silently dropped
and the query returned unfiltered rows. Put predicates in the `GRAPH_TABLE`
`WHERE` clause:

```sql
MATCH (n:User) WHERE n.active = true COLUMNS (n.id)
```

## Path Length with CARDINALITY

When using variable-length paths, use `CARDINALITY(r)` to get the number of hops traversed:

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (me)-[r:FRIENDS_WITH*2..3]->(fof)
  WHERE me.path = '/users/alice'
  COLUMNS (
    fof.id AS id,
    fof.path AS path,
    fof.properties AS properties,
    CARDINALITY(r) AS degree  -- Returns 2 or 3
  )
) AS g
ORDER BY degree ASC
```

**What it does:** `CARDINALITY(r)` returns the number of relationships traversed in the path. This is the ISO SQL standard function for element count.

**Use cases:**
- Friend suggestion ranking by connection degree (2nd degree vs 3rd degree)
- Finding shortest paths in social networks
- Analyzing reachability depth in knowledge graphs
- LinkedIn-style connection indicators

**Return values:**
| Pattern | CARDINALITY(r) Returns |
|---------|------------------------|
| `-[r:TYPE]->` | 1 (single hop) |
| `-[r:TYPE*2]->` | 2 (exactly 2 hops) |
| `-[r:TYPE*2..3]->` | 2 or 3 (depending on path) |
| `-[r:TYPE*1..5]->` | 1 to 5 (depending on path) |

**Example - Friend suggestions with degree:**

```sql
-- Get friend-of-friend suggestions ranked by closeness
SELECT DISTINCT * FROM GRAPH_TABLE(
  MATCH (me)-[r:FRIENDS_WITH*2..3]->(fof)
  WHERE me.path = '/users/current-user'
    AND fof.path <> '/users/current-user'
  COLUMNS (
    fof.id AS id,
    fof.path AS path,
    fof.properties AS properties,
    CARDINALITY(r) AS degree
  )
) AS suggestions
ORDER BY degree ASC  -- 2nd degree first, then 3rd degree
LIMIT 10
```

## Supported Algorithms

### PageRank

**What it does:** Measures the importance/influence of nodes based on the structure of incoming links.

**Use cases:**
- Find influential users in a social network
- Identify important documents in a citation network
- Rank pages by authority in a web graph
- Detect key entities in a knowledge graph

**Theory:** PageRank models a "random surfer" who follows links randomly. Nodes that receive many links from other important nodes get higher scores. The algorithm iterates until scores converge.

**SQL Usage:**
```sql
SELECT n.name, pageRank(n) as rank
FROM GRAPH_TABLE (my_graph MATCH (n:User) COLUMNS (n.name, n))
ORDER BY rank DESC;
```

**Configuration Parameters:**
| Parameter | Default | Description |
|-----------|---------|-------------|
| `damping_factor` | 0.85 | Probability of following a link (vs. random jump) |
| `max_iterations` | 100 | Maximum iterations before stopping |
| `convergence_threshold` | 0.0001 | Stop when score changes are below this |

### Louvain (Community Detection)

**What it does:** Detects communities/clusters by optimizing modularity - groups nodes that are more densely connected internally than externally.

**Use cases:**
- Find friend groups in social networks
- Identify topic clusters in knowledge graphs
- Segment customers by behavior patterns
- Detect organizational structures

**Theory:** The Louvain algorithm works in two phases: (1) each node moves to the community that maximizes modularity gain, (2) communities are collapsed into super-nodes. This repeats until no improvement is possible.

**SQL Usage:**
```sql
SELECT n.name, louvain(n) as community_id
FROM GRAPH_TABLE (my_graph MATCH (n:User) COLUMNS (n.name, n))
ORDER BY community_id;

-- Count community sizes
SELECT louvain(n) as community_id, COUNT(*) as size
FROM GRAPH_TABLE (my_graph MATCH (n:User) COLUMNS (n))
GROUP BY community_id ORDER BY size DESC;
```

**Configuration Parameters:**
| Parameter | Default | Description |
|-----------|---------|-------------|
| `resolution` | 1.0 | Higher = more smaller communities, Lower = fewer larger communities |
| `max_iterations` | 100 | Maximum optimization iterations |

### Connected Components

**What it does:** Identifies groups of nodes that are connected to each other but disconnected from other groups.

**Use cases:**
- Find isolated clusters in a network
- Identify data quality issues (orphan records)
- Detect separate sub-graphs
- Partition large graphs for processing

**Theory:** Uses Union-Find algorithm to efficiently group nodes. Two nodes are in the same component if there's any path between them (ignoring edge direction for weakly connected components).

**SQL Usage:**
```sql
SELECT n.name, connectedComponents(n) as component_id
FROM GRAPH_TABLE (my_graph MATCH (n:User) COLUMNS (n.name, n));

-- Find isolated nodes (components of size 1)
SELECT component_id, COUNT(*) as size
FROM (
  SELECT connectedComponents(n) as component_id
  FROM GRAPH_TABLE (my_graph MATCH (n:User) COLUMNS (n))
)
GROUP BY component_id
HAVING size = 1;
```

### Triangle Count

**What it does:** Counts the number of triangles (3-node cycles) each node participates in.

**Use cases:**
- Measure clustering tendency in social networks
- Detect tightly-knit groups
- Calculate local clustering coefficient
- Identify bridge nodes (low triangle count but high degree)

**Theory:** A triangle exists when three nodes are all mutually connected (A-B, B-C, and C-A). High triangle counts indicate tightly clustered neighborhoods. The algorithm efficiently finds common neighbors between pairs.

**SQL Usage:**
```sql
SELECT n.name, triangleCount(n) as triangles,
       CAST(triangleCount(n) as FLOAT) / (degree(n) * (degree(n) - 1) / 2) as clustering_coefficient
FROM GRAPH_TABLE (my_graph MATCH (n:User) COLUMNS (n.name, n))
WHERE degree(n) > 1;
```

### Shortest Path

**What it does:** Finds a minimum-hop path between two nodes.

Written as a path *selector* on the MATCH pattern — there is no `shortestPath()`
function.

**Use cases:**
- Navigation and routing
- Game AI pathfinding
- Network flow optimization
- Finding optimal sequences

**SQL Usage:**
```sql
SELECT hops, stops
FROM GRAPH_TABLE (road_network
  MATCH ANY SHORTEST p = (a:City)-[:ROAD]-{1,10}(b:City)
  WHERE a.name = 'New York' AND b.name = 'Los Angeles'
  COLUMNS (path_length(p) AS hops, nodes(p) AS stops)
);
```

Use `ALL SHORTEST` to get every minimum-hop path rather than one of them.

### Cheapest Path (RaisinDB extension)

> **Not standardised.** Neither GQL nor SQL/PGQ standardises weighted path
> search; the committee lists "cheapest path search, by adding weights to
> edges" among features not ready for the current drafts. The spelling here
> follows Google Spanner Graph. **Portable queries should use `ANY SHORTEST`
> (hop count).**

**SQL Usage:**
```sql
SELECT hops, route
FROM GRAPH_TABLE (road_network
  MATCH ANY CHEAPEST p = (a:City)-[r:ROAD COST r.weight]-{1,10}(b:City)
  WHERE a.name = 'New York' AND b.name = 'Los Angeles'
  COLUMNS (path_length(p) AS hops, edges(p) AS route)
);
```

`COST` may only reference the edge's `weight` — a RaisinDB relation carries
`target`, `workspace`, `target_node_type`, `relation_type` and `weight`, and no
property map. `ANY CHEAPEST` without a `COST`, or `COST` without
`ANY CHEAPEST`, is a compile error: a weighted query that silently answers by
hop count is worse than one that refuses.

### K-Shortest Paths — not yet supported

There is no way to ask for the *k* shortest paths today. `SHORTEST k` and
`SHORTEST k GROUP` parse to a named error rather than being silently ignored.

Yen's algorithm is implemented (`raisin-sql-execution`, `graph_algo/yen.rs`) but
is not reachable from `GRAPH_TABLE`: returning *k* paths per endpoint pair
multiplies rows, and that interaction with `ORDER BY` / `LIMIT` is deliberately
left for a later pass. Use `ALL SHORTEST` when every minimum-hop path is enough.

## Graph Algorithm Configuration

Graph algorithms can be configured for automatic precomputation using the `raisin:GraphAlgorithmConfig` node type.

### Configuration Structure

Create configuration nodes at `/raisin:access_control/graph-config/`:

```yaml
# /raisin:access_control/graph-config/pagerank-social
node_type: raisin:GraphAlgorithmConfig
properties:
  algorithm: "pagerank"
  enabled: true

  # Target branches/revisions
  target:
    mode: "branch"           # branch | all_branches | revision | branch_pattern
    branches:
      - "main"
      - "staging"

  # Scope: which nodes to include
  scope:
    paths:
      - "social/users/**"
    node_types:
      - "raisin:User"
    workspaces:
      - "social"
    relation_types:
      - "FRIENDS_WITH"
      - "FOLLOWS"

  # Algorithm-specific parameters
  config:
    damping_factor: 0.85
    max_iterations: 100
    convergence_threshold: 0.0001

  # Refresh triggers
  refresh:
    ttl_seconds: 300         # Recompute every 5 minutes
    on_branch_change: true   # Recompute when branch HEAD changes
    on_relation_change: true # Recompute when relationships change
```

### Target Modes

| Mode | Description | Invalidation |
|------|-------------|--------------|
| `branch` | Specific branches, tracks HEAD | Recalculates when HEAD changes |
| `all_branches` | All branches, tracks each HEAD | Per-branch recalculation |
| `revision` | Specific commits (immutable) | Never - computed once |
| `branch_pattern` | Glob pattern (e.g., `feature/*`) | Per-matching-branch |

### Scope Options

| Option | Description | Example |
|--------|-------------|---------|
| `paths` | Glob patterns for node paths | `["social/users/**", "community/**"]` |
| `node_types` | Filter by node type | `["raisin:User", "raisin:Post"]` |
| `workspaces` | Filter by workspace | `["social", "community"]` |
| `relation_types` | Only include nodes connected via these | `["FRIENDS_WITH", "FOLLOWS"]` |

### Refresh Configuration

| Option | Description | Default |
|--------|-------------|---------|
| `ttl_seconds` | Time before automatic recomputation | 0 (disabled) |
| `on_branch_change` | Recompute when branch HEAD changes | false |
| `on_relation_change` | Recompute when relations in scope change | false |
| `cron` | Cron schedule for recomputation | null |

## Background Precomputation

RaisinDB runs graph algorithms as background tasks (similar to RocksDB compaction), not through the job queue. This provides:

- **Natural debouncing**: 100 document changes result in 1 recomputation when the tick runs
- **No queue congestion**: Large graph computations don't block other jobs
- **Efficient invalidation**: TTL + relation change triggers mark caches stale

### How It Works

1. **Background tick** runs periodically (default: every 60 seconds)
2. Checks all enabled `GraphAlgorithmConfig` nodes
3. For each config that needs recomputation (stale, TTL expired, or branch changed):
   - Builds a graph projection from scoped nodes
   - Executes the algorithm
   - Stores results in the `GRAPH_CACHE` column family
   - Updates cache metadata

### Cache Storage

Results are stored in a dedicated RocksDB column family:

```
Key format: {tenant}:{repo}:graph_cache:{branch}:{config_id}:{node_id}
Value: MessagePack-encoded { value, computed_at, expires_at, source_revision }
```

### Query-Time Behavior

When you call `pageRank(n)` in a query:

1. **Check in-memory LRU cache** - fastest path
2. **Check RocksDB GRAPH_CACHE** - if not in LRU
3. **Live computation** - fallback if no cached value

## RELATES - Graph-Based Permissions

The `RELATES` keyword in REL (Raisin Expression Language) enables graph-based permission checks. See [REL.md](./REL.md#graph-relationship-checks-relates) for details.

Example permission rule:
```yaml
permissions:
  - path: "posts.**"
    operations: [read]
    condition: |
      node.visibility == 'public' ||
      node.created_by == auth.local_user_id ||
      node.created_by RELATES auth.local_user_id VIA 'FRIENDS_WITH' DEPTH 1..2
```

RELATES uses precomputed reachability sets when available, falling back to BFS for uncached queries.

## Performance Considerations

### Algorithm Complexity

| Algorithm | Time Complexity | Space Complexity |
|-----------|-----------------|------------------|
| PageRank | O(E × iterations) | O(V) |
| Louvain | O(V × log V) | O(V) |
| Connected Components | O(V + E) | O(V) |
| Triangle Count | O(V × d²) | O(V × d) |
| A* | O(E × log V) | O(V) |

Where V = vertices (nodes), E = edges (relationships), d = average degree.

### Best Practices

1. **Limit scope**: Use `paths`, `node_types`, or `workspaces` to reduce graph size
2. **Set appropriate TTL**: Balance freshness vs. computation cost
3. **Use branch mode**: Avoid recomputing for every revision
4. **Monitor cache hit rates**: Check if precomputation is effective
5. **Set max_nodes limit**: Prevent memory issues with very large graphs

### Scaling Guidelines

| Graph Size | Recommendation |
|------------|----------------|
| < 10K nodes | Live computation is fine |
| 10K - 100K nodes | Precomputation recommended, TTL ~5 min |
| 100K - 1M nodes | Precomputation required, TTL ~30 min |
| > 1M nodes | Consider partitioning by scope |

## Example Configurations

### Social Network Analytics

```yaml
# PageRank for user influence
node_type: raisin:GraphAlgorithmConfig
properties:
  algorithm: "pagerank"
  enabled: true
  target:
    mode: "branch"
    branches: ["main"]
  scope:
    node_types: ["raisin:User"]
    relation_types: ["FOLLOWS", "FRIENDS_WITH"]
  config:
    damping_factor: 0.85
    max_iterations: 50
  refresh:
    ttl_seconds: 600
    on_relation_change: true
```

### Community Detection

```yaml
# Louvain for friend groups
node_type: raisin:GraphAlgorithmConfig
properties:
  algorithm: "louvain"
  enabled: true
  target:
    mode: "branch"
    branches: ["main"]
  scope:
    node_types: ["raisin:User"]
    relation_types: ["FRIENDS_WITH"]
  config:
    resolution: 1.0
    max_iterations: 100
  refresh:
    ttl_seconds: 3600
    on_branch_change: true
```

### Permission-Based Reachability

```yaml
# RELATES cache for friend-of-friend permissions
node_type: raisin:GraphAlgorithmConfig
properties:
  algorithm: "relates_cache"
  enabled: true
  target:
    mode: "branch"
    branches: ["main", "staging"]
  scope:
    relation_types: ["FRIENDS_WITH", "FOLLOWS"]
  config:
    max_depth: 2
  refresh:
    ttl_seconds: 60
    on_relation_change: true
```

## Troubleshooting

### Cache Not Updating

1. Check if config is `enabled: true`
2. Verify target branches exist
3. Check background task logs for errors
4. Verify scope matches actual nodes

### Slow Queries

1. Enable precomputation for frequently-used algorithms
2. Reduce scope to relevant nodes
3. Increase TTL to reduce recomputation frequency
4. Check if graph is too large for live computation

### Incorrect Results

1. Verify scope includes all relevant nodes
2. Check relation_types includes all edge types
3. Ensure algorithm parameters are appropriate
4. Verify branch HEAD hasn't changed since computation

## API Reference

### SQL Functions

| Function | Returns | Description |
|----------|---------|-------------|
| `CARDINALITY(rel)` | INTEGER | Path length (hops) for variable-length paths |
| `pageRank(node)` | FLOAT | PageRank score (0.0-1.0) |
| `louvain(node)` | INTEGER | Community ID |
| `connectedComponents(node)` | INTEGER | Component ID |
| `triangleCount(node)` | INTEGER | Number of triangles |

Shortest-path search is a MATCH-clause **selector** (`ANY SHORTEST`,
`ALL SHORTEST`, `ANY CHEAPEST`), not a function. Path results are read through
the path accessors below.

### Path Accessors

A path variable is not selectable on its own — there is no `PATH` column type,
and `COLUMNS (p)` is a compile error naming these accessors instead. Every
accessor lands on a type all three transports (HTTP, WS, PGWire) already carry.

| Accessor | Returns | Description |
|----------|---------|-------------|
| `path_length(p)` | INTEGER | Hop count (`= edges(p)` length) |
| `nodes(p)` | JSON array | Nodes in path order, length `path_length + 1` |
| `edges(p)` | JSON array | Edges in path order, length `path_length` |
| `element_id(p)` | TEXT | Opaque stable encoding of the whole path |
| `path_first(p)` | JSON object | First node identity |
| `path_last(p)` | JSON object | Last node identity |
| `is_trail(p)` | BOOLEAN | True when no edge repeats |
| `is_acyclic(p)` | BOOLEAN | True when no node repeats |

Names are lowercase; uppercase spellings are accepted too. `nodes(p)` is spelled
"nodes" rather than DuckPGQ's `vertices(p)` because every other surface in
RaisinDB says *node*.

### Cache Status Endpoint

```
GET /api/v1/repos/{repo}/graph-cache/status

Response:
{
  "configs": [
    {
      "id": "pagerank-social",
      "algorithm": "pagerank",
      "status": "ready",
      "node_count": 15234,
      "last_computed_at": "2024-01-15T10:30:00Z",
      "next_scheduled_at": "2024-01-15T10:35:00Z"
    }
  ]
}
```

### Manual Recomputation

```
POST /api/v1/repos/{repo}/graph-cache/{config_id}/recompute

Response:
{
  "status": "scheduled",
  "estimated_completion": "2024-01-15T10:32:00Z"
}
```
