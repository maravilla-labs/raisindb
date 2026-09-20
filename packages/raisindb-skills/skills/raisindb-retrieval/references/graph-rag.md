# Graph retrieval

Read this when the question is about how things **relate** — who is involved,
what belongs to what, what else mentions this — rather than what a document
says. Vector search finds documents that read like the question; it cannot tell
you two of them concern the same supplier, because neither text says so.

## Relations are not references

RaisinDB has two kinds of link, and graph retrieval uses the explicit one.

| | Reference | Relation |
|---|---|---|
| What it is | A property that points somewhere: `{"raisin:ref": "/path", "raisin:workspace": "ws"}` | `RELATE FROM … TO … TYPE … WEIGHT …` |
| Lives in | The node's own properties | Its own index, independent of both nodes |
| Carries a label | No | Yes — the type IS the label on the arrow |
| Query with | `REFERENCES(...)`, `RESOLVE(...)` | `NEIGHBORS(...)`, `GRAPH_TABLE(...)` |
| Exists because | The content says so — an author field, a hero image | Someone asserted it |

An extracted knowledge graph is the second kind. "Acme employs Dana" is a claim
about the world, not a field on a document, and `works_at` has to be the label
on the edge. Writing these as references would also bury them among the
content's own links, so a traversal could no longer tell "this document mentions
it" from "the author wrote it".

```sql
RELATE FROM path='/entities/dana-weber' IN WORKSPACE 'library'
       TO   path='/entities/acme-marine' IN WORKSPACE 'library'
       TYPE 'works_at';

SELECT path, relation_type, weight
FROM NEIGHBORS('library:/entities/dana-weber', 'BOTH', NULL);
-- /entities/acme-marine   works_at   NULL
```

`RELATE` is idempotent on `(from, to, type)`, so re-asserting an edge is a
no-op rather than a duplicate — which is what lets an extraction job re-run
without `UNRELATE`-ing first.

## Building the graph from documents

`/lib/raisin/ai/extract-entities` reads a document's text, asks a model which
entities it names and how they relate, and writes entity nodes joined by typed
relations, each linked back to the document with a `mentioned_in` edge.

```json
{ "path": "/marine-handbook", "workspace": "library" }
```

Run it from a trigger on document change:

```yaml
trigger:
  kind: node_event
  workspace: assets
  event: updated
steps:
  - id: extract
    kind: function
    function: /lib/raisin/ai/extract-entities
    input:
      path: "${trigger.node.path}"
      workspace: assets
```

**It is safe on every change.** A run records a fingerprint of the text it read,
and a later run finding the same fingerprint does nothing — no model call, no
write. Without that guard every re-save pays for an extraction, mints a
revision, and triggers the reindex that follows, forever. Pass `force: true` to
override.

**Entities converge.** Two documents naming the same organisation attach to ONE
node, and that shared node is the connection neither document could make alone.

The relation type reaches a SQL literal position, so anything a model invents is
reduced to `[a-z0-9_]` before it is written. Keep that if you write your own
extractor — a quote character there would end the literal.

## Walking out from what you found

`/lib/raisin/ai/graph-context` seeds from hybrid search (or from seeds you pass)
and expands breadth-first:

```json
{ "query": "who maintains the handbook?", "hops": 2 }
```

Each returned node carries:

- `hop` — distance from a seed, so a caller can prefer the near ones
- `via` — the edge's type, so an answer can say not just *that* two things are
  connected but *how*
- `truncated: true` on the result when the walk hit its budget

**Report truncation to the user or the model.** A neighbourhood that looks
complete but was cut short is how an assistant concludes something is
unconnected when it was simply never visited.

`ask` can use this as a second retrieval leg with `use_graph: true`. It is
opt-in because it answers a different question and costs a walk.

## Writing your own walk

```sql
SELECT id, path, name, node_type, relation_type, weight
FROM NEIGHBORS($1, 'BOTH', NULL)
LIMIT 25
```

Three things to get right, each of which fails silently:

1. **Address by `id` on hops after the first.** A neighbour row does not say
   which workspace it came from, and a bare path resolves in the default
   workspace only — so continuing by path searches the wrong workspace and
   returns nothing, which reads as "the graph ends here". The
   `'workspace:/path'` form works for a seed, from v0.6.34.
2. **Dedupe on every identity a node answers to** — its id *and* its path. A
   seed addressed by path comes back as a neighbour under its id, and keyed on
   one of those it is emitted as its own neighbour.
3. **Budget the walk.** One popular node — a tag, a shared contact — can have
   thousands of edges, and at two hops that is a query storm and a context
   window nothing can use. Stop, and say that you stopped.

`BOTH` is usually right: a contract that references a party and a party that
references a contract are equally worth reading, and which way the edge points
is a modelling accident.

## Algorithms

For ranking and clustering over a projected subgraph — PageRank, Louvain
communities, betweenness, shortest paths — see `GRAPH_TABLE` and the
`raisin:GraphAlgorithmConfig` node type. Those run over an in-memory projection
and write results back to node properties; they are a different tool from the
per-question walk above.
