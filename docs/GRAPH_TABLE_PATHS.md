# GRAPH_TABLE path accessors

`GRAPH_TABLE` used to return **scalars only**. `sssp`, `bfs`, `pageRank`, `wcc`
and friends each answer one number per row, and there was no way to ask for the
ordered list of nodes on a path — the variable-length matcher computed the full
path and threw it away, keeping only the first and last node and smuggling the
hop count out by rewriting a relation type to `"knows[3]"`.

A path variable now binds the whole ordered path, and the accessors below read
it. For the grammar (`MATCH p = …`, selectors, restrictors, quantifiers) see
[`GRAPH_TABLE.md`](GRAPH_TABLE.md); this page is the accessor reference.

## There is no PATH column type

A path variable is **not selectable on its own**. `COLUMNS (p)` is a compile
error naming the accessors:

```
'p' is a path and has no SQL column type, so it cannot be selected directly.
Use a path accessor: path_length, nodes, edges, element_id, path_first,
path_last, is_trail, is_acyclic.
```

This is deliberate. The PGQ value type has `Null`, `Boolean`, `Integer`,
`Float`, `String`, `Array` and `Json` — and no `PATH`. Adding one would mean
teaching three transports a new type; PGWire in particular has nothing to
borrow, because PostgreSQL OID 602 `path` is a *geometric* type and would be
actively wrong. Every accessor instead lands on a value that already crosses
HTTP, WS and PGWire, so path support needed **zero transport changes**.

## The accessors

Names are DuckPGQ-style lowercase, plus the four Spanner names DuckPGQ has no
equivalent for (`path_first`, `path_last`, `is_trail`, `is_acyclic`). Path
accessors are not standardised — vendors disagree — so this is a choice, not a
conformance claim. Uppercase spellings are accepted too; dispatch lowercases
first.

| accessor | returns | SQL type | PGWire | JSON |
|---|---|---|---|---|
| `path_length(p)` | hop count (`edges.len()`) | `Integer` | `INT8` | number |
| `nodes(p)` | nodes in path order | `Json` array | `json` text | array |
| `edges(p)` | edges in path order | `Json` array | `json` text | array |
| `element_id(p)` | opaque path identity | `String` | `TEXT` | string |
| `path_first(p)` | first node identity | `Json` object | `json` text | object |
| `path_last(p)` | last node identity | `Json` object | `json` text | object |
| `is_trail(p)` | every edge distinct | `Boolean` | `BOOL` | bool |
| `is_acyclic(p)` | every node distinct | `Boolean` | `BOOL` | bool |

`nodes(p)` always has `path_length(p) + 1` entries; `edges(p)` always has
`path_length(p)`.

### `nodes(p)` shape

```json
[
  { "id": "a", "workspace": "graph", "node_type": "PgqCity" },
  { "id": "b", "workspace": "graph", "node_type": "PgqCity" }
]
```

`nodes`, not DuckPGQ's `vertices`: the model type is `raisin_models::nodes::Node`
and every other surface in the product says "node".

### `edges(p)` shape

```json
[
  {
    "relation_type": "road",
    "source_id": "a",
    "target_id": "b",
    "source_workspace": "graph",
    "target_workspace": "graph",
    "weight": 5.0
  }
]
```

`weight` is `RelationRef.weight` **verbatim**, and `null` when unset. It is never
defaulted to `1.0`.

`relation_type` is verbatim too. It used to be rewritten to `"road[2]"` on
variable-length matches so `CARDINALITY(r)` could parse the hop count back out;
that encoding is gone.

### `element_id(p)` shape

```
graph:a|road|graph:b|road|graph:d
```

Pipe-joined, node and edge alternating. Usable as an equality or dedup key.
**Not a parseable contract** — treat it as opaque.

## Worked example

```sql
SELECT *
FROM GRAPH_TABLE(
  MATCH ANY SHORTEST p = (a:PgqCity)-[r:road]->{1,4}(b:PgqCity)
  WHERE a.id = 'a' AND b.id = 'd'
  COLUMNS (
    path_length(p) AS hops,
    nodes(p)       AS ns,
    edges(p)       AS es,
    element_id(p)  AS eid,
    is_acyclic(p)  AS acyclic
  )
);
```

## `CARDINALITY(r)`

`CARDINALITY(r)` still works and still returns the hop count. It now reads the
path bound under `r` instead of parsing a mangled relation type. A single-hop
relationship has no bound path and is cardinality 1.

## Selectors, briefly

The selector chooses among the paths that match; the restrictor decides which
walks are paths at all. See [`GRAPH_TABLE.md`](GRAPH_TABLE.md) for the grammar.

| selector | meaning |
|---|---|
| *(none)* | every path |
| `ANY` | one arbitrary path per endpoint pair — **not** minimum-hop |
| `ANY SHORTEST` | one minimum-hop path per endpoint pair |
| `ALL SHORTEST` | every minimum-hop path per endpoint pair |
| `ANY CHEAPEST` | one minimum-total-weight path (RaisinDB extension) |

> **`ANY CHEAPEST` and `COST` are a RaisinDB extension.** Neither GQL nor
> SQL/PGQ standardises weighted path search; the committee lists "cheapest path
> search, by adding weights to edges" among features not ready for the current
> drafts. The spelling follows Google Spanner Graph. Portable queries should use
> `ANY SHORTEST`, which is hop count only.

Under `ANY CHEAPEST`, a traversed edge with no weight — or a weight that is not
positive and finite — is a **runtime error**:

```
edge road from graph:d to graph:k has no weight; ANY CHEAPEST requires every
traversed edge to carry a positive finite weight
```

It is never `unwrap_or(1.0)`. A routing query that silently answers with a
shortest-*hop* path while claiming to be cheapest is exactly the class of
silent-wrong-results this engine has been eliminating.

## Limits, and why they are errors

Two caps bound a variable-length match:

* **Depth** — an unbounded quantifier is capped at 10 hops.
* **Paths** — a single pattern may enumerate at most **10,000** paths.

Exceeding the path cap is an **error**, not a shorter answer:

```
variable-length path match exceeded the 10000 path cap. Partial results are not
returned because they are indistinguishable from a complete answer. Narrow the
quantifier (for example ->{1,2}), add a selector such as ANY SHORTEST, or filter
the endpoints with a label or WHERE clause.
```

This is a behaviour change: the matcher used to stop at the cap and return what
it had, which looked like a complete result set.

## Honest performance note

A path query materialises the adjacency for **every relation in the branch**, in
memory, per query. On a branch with millions of relations a path query is slow
and memory-hungry regardless of which selector is used. When both endpoints of a
path are bound to concrete nodes and the default `ACYCLIC` restrictor is in
force, the query is answered directly by BFS (or Dijkstra under `ANY CHEAPEST`)
and nothing is enumerated; otherwise paths are enumerated and then selected,
because that is the only formulation that composes with a restrictor and with a
free target.

## Proof

`crates/raisin-server/tests/all/pgq_path_e2e_test.rs` starts a real server,
writes real nodes and relations, and asserts every claim on this page through
`POST /api/sql/{repo}` — including that `ANY SHORTEST` (2 hops, weight 10) and
`ANY CHEAPEST` (3 hops, weight 3) return *different* routes, which is the only
way to prove the weights are actually being read.
