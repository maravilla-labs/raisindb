# HYBRID_SEARCH / FULLTEXT_SEARCH / KNN — SQL semantics

Status: implementable specification. Supersedes the three design proposals.
Every code claim below was re-verified against this worktree before adoption;
where a proposal misdescribed the tree, the correction is noted inline.

---

## 0. Verdict

**Proposal 2 (least surprise / least privilege) is the base.** It is the only
proposal whose description of the tree survived checking end to end, and its two
decisive arguments hold up:

- **the shortfall must go to the operator log, never to the result set** — a
  `dropped_by_permission` column or a short-result error is a differential
  oracle: hold the scope, vary the query, count rows, enumerate documents you
  may not read. That is a worse leak than the one `rls_filter_search_hit`
  just closed, delivered through the fix's own diagnostics.
- **push-down is a rank-correctness argument, not a performance one** — RRF
  consumes *ranks*. A rank computed over a pool containing rows the caller can
  never see is a wrong rank, printed in a column named `fulltext_rank`. No
  amount of over-fetching repairs it; only narrowing the pool before ranking
  does.

Grafted in:

- from **Proposal 1**: the `WorkspaceSet` enum where `Empty ≠ All` (a one-keystroke
  RLS bypass otherwise), the unified column set across all three functions, one
  shared fetch/fuse/emit loop, two independent leg weights where `0` *skips the
  leg including embedder resolution*, `max_distance` as a first-class argument,
  `EXPLAIN` printing the resolved corpus, and implementing `KNN` rather than
  deleting it.
- from **Proposal 3**: the *universe vs. filter* distinction stated exactly
  ("`WORKSPACES => ('a','b') LIMIT 10` can return rows where `ALL … WHERE
  workspace_id IN ('a','b') LIMIT 10` returns none"), the CLAUDE.md `REFERENCES`
  discipline — **push-down is an optimisation, the residual filter is the
  correctness** — RLS drops and residual-predicate drops sharing ONE loop, the
  rule that ranks must never be merged across two runs at different `k`, and
  skipping the over-fetch entirely for callers who cannot be RLS-filtered.

Where the tree settled a dispute:

| Claim | Verdict |
|---|---|
| P1: "a `WHERE` above a table function becomes a generic `PhysicalPlan::Filter`" | **False.** `build_table_source` (`builder/table_source.rs:46-61`) returns `LogicalPlan::TableFunction` *without using* `filter`; only the `Scan` arm consumes it. `split_predicates_by_table` routed it there because `identifiers.rs:136` qualifies even a bare column with `TableRef::name()`. The predicate is **silently discarded**. P1's whole §3 ("`WHERE` keeps working, it is just post-truncation") rests on a mechanism that does not exist. |
| P1: `filter_node` returns everything for `permissions() == None` | **False.** `rls_filter/mod.rs:38-46` **denies**. `is_system` and `is_system_admin` allow. The distinction is load-bearing for the readable-set resolver (§5.2). |
| P1: `DEFAULT_MAX_DISTANCE` is declared twice | **True** — `raisin-hnsw/src/engine/search.rs:114` and `:212`. (P3 and the session brief both cite `:101`, which is the doc comment.) |
| P3: "there is no workspace catalog table to select from; `pg_class` is a stub" | **False.** `Workspaces` is a live, read-only, RLS-scanned catalog table (`catalog/schema_tables/schema_object_tables.rs:431`, executed by `scan_executors/schema_table_scan.rs:82`) and carries `allowed_node_types` JSONB. This kills P3's rejection of the two-step form and removes the need for an `ACCEPTING` keyword. |
| P2: named args parse and their names are thrown away | **True.** `sqlparser 0.59` defaults `supports_named_fn_args_with_rarrow_operator() -> true`; `analyze_table_function_args` (`from_clause.rs:462-470`) matches `FunctionArg::Named { arg, .. }` and pushes only the expr. So `HYBRID_SEARCH('q', workspaces => 'library')` today means *limit 10, every workspace*. |
| P2: `:=` is a synonym | **False** — `supports_named_fn_args_with_assignment_operator()` defaults to `false`. Only `=>`. |
| P2/P3: `KNN` has no executor | **True.** `execute_table_function` has no arm; it falls to `"Unsupported table function: KNN"` (`table_function.rs:706`) while `ddl_keywords/functions.rs:70-77` ships a worked example. |
| P2: the language argument must be ISO 639-1 | **True and worse than stated.** `tantivy_engine/search.rs:31-33` builds an exact `TermQuery` on the stored `language` field, and `tantivy_engine/language.rs:10` stamps ISO 639-1. The shipped help example `FULLTEXT_SEARCH('content management', 'english')` matches **zero documents, forever**. |
| P2/P3: `node_types` can be pushed into the fulltext leg via `shape_type` | **True but not equivalent.** `shape_types` is multi-valued and holds `node_type` ∪ `archetype` ∪ nested `element_type` (`tantivy_engine/properties.rs:28-35`). A `shape_type` filter therefore **over-matches** a `node_type` predicate. Safe as an optimisation under a residual filter; a lie if exposed as an argument. Decides §6.2. |

---

## 1. The surface

```
HYBRID_SEARCH  ( query [, limit]    [, workspace] [, named ...] )
FULLTEXT_SEARCH( query ,  language               [, named ...] )
KNN            ( query [, limit]                 [, named ...] )
```

### 1.1 Positionals

| # | HYBRID_SEARCH | FULLTEXT_SEARCH | KNN |
|---|---|---|---|
| 1 | `query` TEXT — **required** | `query` TEXT — **required** | `query` TEXT, or `EMBEDDING('…')`, or a `Literal::Vector` — **required** |
| 2 | `limit` INT — optional, default 10 | `language` TEXT — **required** (unchanged) | `limit` INT — optional, default 10 |
| 3 | `workspace` TEXT — optional, **kept forever**, exactly equivalent to `workspaces => '<name>'` | — | — |

- A **4th positional on any function is an error.** Today extras are silently
  ignored (`FULLTEXT_SEARCH` reads only args 0–1; `HYBRID_SEARCH` only 0–2).
- **`FULLTEXT_SEARCH` positional #3 is an error**, naming `workspaces =>`. It is
  ignored today, so giving it a meaning silently is exactly the drift being
  banned. `HYBRID_SEARCH` positional #3 keeps working because it *already has*
  that meaning. The asymmetry is deliberate; document it or someone will
  "make it consistent" and break one of the two.
- A **non-INT `limit` is a hard error**, never a silent 10. Today
  `HYBRID_SEARCH('q','library')` is limit-10-cross-workspace with no complaint
  (`table_function.rs:437-445`).
- Positionals must precede named args. Supplying a value both positionally and
  by name is an error naming both. Never a silent precedence.

### 1.2 Named arguments (`name => value`, `=>` only)

No parser work: `sqlparser 0.59` already parses these into `FunctionArg::Named`.
The one required change is that `analyze_table_function_args` must **stop
discarding the name**.

| name | type | default | applies to |
|---|---|---|---|
| `workspaces` | TEXT (grammar §1.3) | **none — REQUIRED** | all three |
| `limit` | INT, `1..=1000` | 10 / 100 (fulltext) / 10 | all three |
| `language` | TEXT, ISO 639-1 (`^[a-z]{2}$`) | `ctx.default_language` | hybrid, fulltext |
| `vector_weight` | DOUBLE, `>= 0.0` | 1.0 | hybrid (`KNN` forces 1.0) |
| `fulltext_weight` | DOUBLE, `>= 0.0` | 1.0 | hybrid (`KNN` forces 0.0) |
| `max_distance` | DOUBLE, `(0.0, 2.0]` | 0.6 | hybrid, knn |

- Every value also accepts `Literal::Parameter` (`$1`), so an agent building a
  workspace list in the host language binds it.
- An unrecognised name is a hard error listing the valid names.
- A `language` value that is not two lowercase letters is a hard error naming
  the correct spelling (`'en'`, not `'english'`) — see §0.
- Both weights zero is an error.

### 1.3 The `workspaces` value grammar

One parser, one place — `raisin_sql_execution::search::scope::parse_workspace_scope`.
Exactly five productions:

```
exact := <name>                       'library'
set   := <name> ("," <name>)+         'library, handbook, policies'
glob  := token containing '*' or '?'  'content-*'
all   := 'ALL READABLE'               case-insensitive, exactly those two words
error := '' | '*' | 'ALL' | anything else
```

- Whitespace around commas is trimmed. A name containing a comma is rejected
  *here*, with the name quoted, rather than silently becoming two names that do
  not exist (which would be zero rows and no error).
- `'*'` and bare `'ALL'` are **rejected**, with an error naming `'ALL READABLE'`.
  `'*'` reads as "unscoped" and the eye skips it; `'ALL READABLE'` is two
  uppercase words, unique in any corpus, and is the grep target that makes
  breadth auditable. **Do not add a shorter alias later** — that is how this
  becomes `'*'` again.
- `'ALL READABLE'` never means "every workspace in the repo". There is no
  spelling for that; a system caller gets it implicitly (§5.2) and nobody else
  needs it.

### 1.4 Errors, verbatim

```
HYBRID_SEARCH('q', 10)
-- ERROR: HYBRID_SEARCH requires an explicit workspace scope. Add
--        workspaces => '<workspace>' to search one, 'a, b, c' for several,
--        'content-*' for a family, or workspaces => 'ALL READABLE' for every
--        workspace you may read (which is what this call used to do).

HYBRID_SEARCH('q', 10, workspaces => '*')
-- ERROR: workspaces => '*' is not a scope. Use 'ALL READABLE' for every
--        workspace you may read, or a glob such as 'content-*'.

FULLTEXT_SEARCH('q', 'english', workspaces => 'library')
-- ERROR: language must be an ISO 639-1 code. Use 'en'; the index stores
--        two-letter codes, so 'english' matches no documents.

HYBRID_SEARCH('q', 10, workspaces => 'library', k => 60)
-- ERROR: unknown argument 'k' for HYBRID_SEARCH. Valid: workspaces, limit,
--        language, vector_weight, fulltext_weight, max_distance.

HYBRID_SEARCH('q', 10, workspaces => 'library, payroll')
-- ERROR: workspace 'payroll' is not available to this query.
```

That last message is **identical for "does not exist" and "you may not read
it"**, deliberately — the same 404-not-403 stance this repo already takes under
RLS. The distinction goes to a `debug` log (`reason=not_in_catalog` /
`reason=not_readable`), never to the caller.

---

## 2. The default (2-arg form): a hard error, in one release

`HYBRID_SEARCH('q', 10)` and `FULLTEXT_SEARCH('q','en')` currently search every
workspace in the repo. As of this change they **fail with the message in §1.4**.

Three options were on the table; two are wrong.

- **Silently re-point at the session workspace** (P3's release N+1) is the worst.
  For pgwire and the WS handler the "session workspace" is whatever the
  *connection* opened on — a value the caller never chose and usually cannot
  see, and `planner/mod.rs` falls back to the literal `"default"`, frequently not
  a real workspace. A RAG agent whose corpus spans two workspaces quietly loses
  half of it and answers confidently from the remainder. Nothing in the query,
  the logs or the row count says anything changed. That is the codebase's named
  dominant bug class, delivered on purpose.
- **Keep it meaning "everything"** (P1) is defensible on retrieval grounds and
  P1's asymmetry argument is right about *which* mistakes RAG punishes — a
  confident false negative has no artifact to notice. But it leaves intact the
  exact property that produced the incident: you cannot tell from the query text
  what corpus it searched, and an operator asked "which of our queries go
  repo-wide?" has no answer, because the broad form and the narrow form are the
  same two tokens and the broad one is the shorter string. RLS makes a PNG in
  `packages` *authorised*; it does not make it *wanted*.
- **Refuse it.** The error names both migrations, so the fix is one argument and
  never a guess. A caller who genuinely wanted breadth restores it in fourteen
  characters *and records that decision in the query text*. A caller who never
  wanted it finds out at the first run instead of after shipping.

P1's real objection — that nagging correct usage trains people to ignore
warnings — is answered by making the broad spelling a first-class, documented,
non-deprecated value rather than a grudging escape hatch. `'ALL READABLE'` is
the recommended RAG form and the book says so.

**No config escape hatch.** A `search_default_workspaces` setting or an
`allow_implicit_search_scope` flag moves the blast radius out of the query and
into a TOML file nobody re-reads, and a flag left on in production is invisible
in review. The whole design is one sentence — *the scope is in the query text* —
and a flag negates it.

**One release, not two.** P3's two-step (error in N, new default in N+1) makes
callers reason about the surface twice for no gain: after N every call names a
scope, so the N+1 default is unreachable. Ship the error and leave it.

---

## 3. Returned columns — one set, all three functions

Today `FULLTEXT_SEARCH` emits timestamps but no ranks, and `HYBRID_SEARCH` emits
ranks but no timestamps, so a retriever that reranks by recency cannot get
`updated_at` out of the hybrid function. Three near-identical hand-maintained
column lists is the same drift class as two fetch loops.

Declare it **once** in `analyzer/semantic/mod.rs` as
`search_result_table_def(name)`, called by `fulltext_search_table_def`,
`hybrid_search_table_def` and `knn_table_def`:

| column | type | null | note |
|---|---|---|---|
| `node_id` | TEXT | no | |
| `workspace_id` | TEXT | no | the workspace the hit came from |
| `name` | TEXT | no | |
| `path` | TEXT | no | |
| `node_type` | TEXT | no | the node's own type, not its shape identity |
| `score` | DOUBLE | no | fused RRF score |
| `fulltext_rank` | BIGINT | yes | NULL when the leg did not contribute |
| `vector_rank` | BIGINT | yes | NULL when the leg did not contribute |
| `vector_distance` | DOUBLE | yes | |
| `chunk_index` | BIGINT | yes | NULL = no vector hit; 0 = unchunked document |
| `revision` | BIGINT | no | |
| `created_at` | TEXT | yes | RFC3339 |
| `updated_at` | TEXT | yes | RFC3339 |
| `properties` | JSONB | no | post-RLS field-filtered bag |

Order is fixed as listed. `knn_table_def`'s single opaque `result JSONB` column
is deleted.

**No `truncated` / `dropped_by_permission` column, ever.** See §7.5.

### 3.1 Deterministic order

Keep `all_hits.sort(); all_hits.dedup();` before fusion, and harden the final
sort from `partial_cmp(...).unwrap_or(Equal)` to an explicit total order:
**`(score DESC, workspace_id ASC, node_id ASC)`**. A caching agent depends on
reproducibility under RRF ties.

Document the consequence: `ORDER BY updated_at DESC` above a search function
orders *the top-k*; it does not retrieve the k most recent matches. That is
correct and must be said out loud.

---

## 4. Worked SQL

```sql
-- (a) one workspace
SELECT node_id, path, score
FROM   HYBRID_SEARCH('vector index rebuild', 10, workspaces => 'library');

-- (a') identical, and kept working forever
SELECT node_id, path, score
FROM   HYBRID_SEARCH('vector index rebuild', 10, 'library');

-- (b) three named workspaces
SELECT node_id, workspace_id, path, score, vector_distance, chunk_index
FROM   HYBRID_SEARCH('vector index rebuild', 10,
                     workspaces => 'library, handbook, policies')
ORDER BY score DESC;

-- (c) every workspace this caller may read -- the RAG form, said out loud
SELECT node_id, workspace_id, path, properties, score
FROM   HYBRID_SEARCH('Wie baue ich einen Vektorindex neu auf?', 20,
                     workspaces => 'ALL READABLE',
                     language   => 'de');

-- (c') cross-lingual: the query shares no lexical surface with the corpus, so
--      the fulltext leg contributes only noise ranks. Skip it entirely.
SELECT node_id, workspace_id, path, vector_distance
FROM   HYBRID_SEARCH('Wie baue ich einen Vektorindex neu auf?', 20,
                     workspaces      => 'ALL READABLE',
                     fulltext_weight => 0,
                     max_distance    => 0.9);

-- (d) a family of workspaces by name
SELECT * FROM FULLTEXT_SEARCH('rollback', 'en',
                              workspaces => 'content-*', limit => 50);

-- (e) "every workspace of a given nodetype" -- two statements, no new syntax
SELECT name FROM Workspaces WHERE allowed_node_types @> '["raisin:Document"]';
SELECT * FROM HYBRID_SEARCH('retention policy', 10,
                            workspaces => 'library, handbook');

-- (e') usually what people actually mean: filter the ROWS, across everything
SELECT node_id, workspace_id, path, score
FROM   HYBRID_SEARCH('retention policy', 10, workspaces => 'ALL READABLE')
WHERE  node_type = 'raisin:Document';

-- (f) pure vector, no fulltext index required
SELECT node_id, path, vector_distance
FROM   KNN(EMBEDDING('quarterly revenue'), 10, workspaces => 'ALL READABLE');

-- (g) keyword-leaning corpus (identifiers, SKUs, error codes)
SELECT * FROM HYBRID_SEARCH('ST_DWITHIN', 10,
                            workspaces      => 'ALL READABLE',
                            vector_weight   => 0.3,
                            fulltext_weight => 1.0);
```

---

## 5. Scope resolution

### 5.1 The type — `Empty` is not `All`

```rust
pub enum WorkspaceSet {
    All,               // push NO filter into the legs (system callers only)
    Only(Vec<String>), // push exactly these
    Empty,             // return zero rows WITHOUT touching either index
}
```

**Never model this as `Option<Vec<String>>`.** Collapsing an empty vec to `None`
turns "may read nothing" into "search everything" — a full RLS bypass on the
read path, one keystroke away, in the same file that just closed one. A test
asserts a caller with no Read grant anywhere gets zero rows and that neither
engine's `search` was called.

### 5.2 Resolution, in order

`raisin_sql_execution::search::scope::resolve_scope(spec, catalog, auth, branch)`:

1. **Expand** against `storage.workspaces().list(RepoScope::new(tenant, repo))`
   (repo-scoped — workspaces have no revision history). `exact`/`set`: each name
   must exist. `glob`: `glob::Pattern`, the same crate and semantics
   `ScopeMatcher` uses. `ALL READABLE`: every catalog name.
2. **Intersect with readable**, via a new sibling of the existing helpers,
   `rls_filter::readable_workspaces(auth, candidates, branch)`. It must mirror
   `filter_node`'s early returns **exactly**, including the one P1 got wrong:

   | caller | result |
   |---|---|
   | `auth_context == None` (no identity at all) | `All` |
   | `Some(a)` with `a.is_system` | `All` |
   | `Some(a)`, `a.permissions() == None` | **`Empty`** — `filter_node` *denies* here (`rls_filter/mod.rs:38-46`) |
   | `Some(a)`, `permissions.is_system_admin` | `All` |
   | otherwise | `{ ws : ∃p ∈ permissions, Read ∈ p.operations ∧ p.scope_matcher().matches(PermissionScope::new(ws, branch)) }` |

   Pure CPU, no I/O, `O(workspaces × permissions)`, matchers pre-compiled.
   Permission scope is a **glob**, so this is *test each candidate*, never
   *enumerate the grants*.
3. **Report vs. narrow.**
   - `exact` / `set`: a name lost at step 1 **or** step 2 is an **error** (§1.4).
     Naming a workspace is an assertion; a typo must not silently shrink a RAG
     corpus forever.
   - `glob` / `ALL READABLE`: names lost at either step are dropped silently.
     Matching a pattern is a query, and a query returning fewer matches is not
     an error.

   That asymmetry is deliberate. Put it in the docs.
4. Sort, dedup. Resolving to nothing yields `Empty` → zero rows, one INFO line,
   **no error**.

### 5.3 Caching

Resolution runs once per statement, never once per leg. The catalog list is a
storage read on the hot path of an agent's RAG loop, so cache it per
`(tenant, repo)`, **event-invalidated on `Event::Workspace` and registered with
`derived_cache_registry`** — checkpoint SST ingest emits no events, and that trap
is already written down in CLAUDE.md.

**Bias the cache toward including, and write the reason in the code.** A stale
set that *includes* a workspace the caller has lost is harmless — per-node RLS
still drops the rows, you only wasted candidates. A stale set that *excludes* a
workspace the caller just gained is a **silent recall loss**. Same shape as the
`has encrypted fields` gate, opposite direction, identical reasoning; someone
will "optimise" it the wrong way unless the comment says so.

---

## 6. Composition: WHERE / ORDER BY / LIMIT

### 6.1 Prerequisite: stop discarding predicates (shared layer, own commit)

Verified: `SELECT * FROM HYBRID_SEARCH('x',10,'library') WHERE node_type='X'`
plans to `Project { TableFunction }` — no `Filter` node anywhere. The published
book ships exactly this pattern (`book/src/api/sql-reference.md:549-553`) and it
has never run.

**Do not fix this by enumerating exclusions in `split_predicates_by_table`** —
that leaves the next `TableRef` kind silently dropping predicates again. Fix it
so no arm *can*:

- `build_table_source` returns `(LogicalPlan, Option<TypedExpr> /* unconsumed */)`.
  Only the `Scan` arm consumes `filter`; every other arm returns it unconsumed.
- `build_query` folds **every** unconsumed filter back into the top-level
  `remaining_predicates`, producing one `LogicalPlan::Filter` above the whole
  FROM/JOIN tree.
- Fold it into the **top-level** filter, not above the individual relation. For
  the right side of an outer join those are not equivalent, and today's code
  already pushes right-side predicates into the right `Scan` — do not propagate
  that bug into the new path.

This one edit fixes `HYBRID_SEARCH`, `FULLTEXT_SEARCH`, `KNN`, `GRAPH_TABLE`,
`NEIGHBORS`, derived tables and CTE scans together. It changes result sets for
anyone who unknowingly relied on the drop, so it ships as its own commit, with
its own tests and its own release-note line.

### 6.2 The rule: universe is an argument, everything else is WHERE

- **`workspaces` is *only* an argument**, because the universe changes what
  top-k *means*. `WORKSPACES => ('a','b') LIMIT 10` returns the 10 best rows in
  a and b; `'ALL READABLE' … WHERE workspace_id IN ('a','b') LIMIT 10` returns
  whichever of a and b's rows survived the *global* best-N — which can be empty
  while matching documents exist.
- **`node_type`, `path`, `name`, `properties->>'k'`, `workspace_id`, and
  `vector_distance` are columns, and they are filtered with `WHERE`.** There is
  no `node_types =>` and no `path_prefix =>` argument.
- **A `WHERE workspace_id = …` never narrows the universe.** P3's
  "two spellings coincide" equivalence is elegant and it is the thing that
  breaks quietly the first time someone adds a predicate form that looks
  pushable and is not. With `workspaces` required, the universe is always
  already named; a `workspace_id` predicate is an honest sub-filter of it, and
  `EXPLAIN` says so. When the planner sees one, log INFO naming the argument
  form the caller probably wanted. **Advise, never rewrite.**

### 6.3 Push-down is an optimisation; the residual filter is the correctness

Straight from the `REFERENCES(...)` discipline already mandated in CLAUDE.md.

`LogicalPlan::TableFunction` and `PhysicalPlan::TableFunction` gain
`filter: Option<TypedExpr>` — an **advisory copy** of the conjuncts that
reference only this function. The authoritative `Filter` above stays. A
push-down that under-narrows costs performance; it can never cost correctness,
and a new predicate form needs no executor work to be *correct*.

| predicate | fulltext leg | vector leg |
|---|---|---|
| workspace set (argument) | native — `FullTextSearchQuery::workspace_ids` (`raisin-storage/src/fulltext.rs:133`), single → `TermQuery`, many → `Should` boolean (`tantivy_engine/search.rs:121-166`). **Zero new indexer code.** | set filter inside the engine (§8) |
| `node_type = 'X'` / `node_type IN (…)` | advisory, as `shape_types` | **impossible** — a `SearchResult` carries only `node_id`, `chunk_id`, `chunk_index`, `workspace_id`, `revision`, `distance` |
| `path LIKE`, property predicates, `vector_distance <` | residual | residual |

**The `shape_types` push-down over-matches, on purpose.** The index field is
multi-valued and holds `node_type` ∪ `archetype` ∪ nested `element_type`
(`tantivy_engine/properties.rs:28-35`), so a `shape_type` term also admits nodes
whose *archetype* is `'X'`. That is sound only because the residual filter is
authoritative — it widens the candidate pool and never drops a needed row.

> This is precisely why there is **no `node_types =>` argument**. An argument
> would have to mean what its name says; `shape_types` does not. Exposing it
> would ship a documented lie.

Widen `FullTextSearchQuery::shape_type: Option<String>` to
`shape_types: Option<Vec<String>>`, mirroring `add_workspace_filter`'s
`Should`-boolean shape. Recognise **only** this predicate shape: a top-level
conjunct that is `Expr::BinaryOp{Eq}` over `Column{name:"node_type"}` and a
`Literal::Text`, or a non-negated `Expr::InList` of `Literal::Text`. Anything
disjunctive, negated, or referencing a joined table → no push-down, residual
only. `fields.shape_types` is `Option<Field>` (absent on a pre-v2 index) — the
push-down must degrade to no filter, as `search.rs:54` already does.

### 6.4 ORDER BY / LIMIT

`ORDER BY` and `LIMIT` above the function compose normally, but the function's
own `limit` is **retrieval depth, not a display cap**:
`HYBRID_SEARCH('q', 10, …) LIMIT 100` returns 10. To get 100 rows, ask the
function for 100. `ORDER BY score DESC` is honoured and elided when it matches
the natural order.

---

## 7. The fetch/fuse/emit loop — one implementation, exact numbers

Today: fuse → `scored.truncate(limit)` (`table_function.rs:632`) → fetch → RLS →
drop. A caller who may read 1 workspace of 12 asks for 10 and gets 1, with
nothing to distinguish that from "only one document matches".

`FULLTEXT_SEARCH` escapes this only because it has no `limit` argument. The
moment it gains one — which this spec gives it — it inherits the identical bug.
So there is **one** loop, in `raisin_sql_execution::search::emit`, called by all
three functions. The RLS test file already states the reason in its header:
*"two separate fetch loops over two separate index legs; a fix applied to one and
not the other leaves the hole open."* That has already happened twice in this
file (the hardcoded `"default"` workspace; `language: "en"` hardcoded in the
hybrid leg while `FULLTEXT_SEARCH` takes it as an argument).

### 7.1 Constants

```rust
/// Shared with planner::plan_dispatch::vector_knn::RESIDUAL_OVERFETCH —
/// ONE constant, not two. RLS is a residual filter; same problem, same number.
pub const SEARCH_OVERFETCH: usize = 20;
pub const SEARCH_LEG_CAP:   usize = 2000;
pub const RRF_K:            f64   = 60.0;
```

`vector_knn.rs:22` is rewritten to reference `SEARCH_OVERFETCH`. Two constants
with one job in two files is how `DEFAULT_MAX_DISTANCE = 0.6` ended up declared
twice in one file.

### 7.2 Leg sizing

```rust
let filtered = !(matches!(scope, WorkspaceSet::All) && residual.is_none());
let leg_k = if filtered {
    (limit * SEARCH_OVERFETCH).min(SEARCH_LEG_CAP)
} else {
    limit * 2                      // chunk-collapse headroom only
};
```

The unfiltered fast path matters: `rls_filter_search_hit` returns everything for
`auth == None` / `is_system` / `is_system_admin`, so over-fetching for them is
pure waste. `limit` is validated `1..=1000` at analysis time, so `leg_k ≤ 2000`.
This replaces both `limit * 2` sites (`:551`, `:581`) and `FULLTEXT_SEARCH`'s
hardcoded `1000` (`:363`).

### 7.3 The loop

```
budget      = SEARCH_LEG_CAP
checks_left = 10 * limit                    // fetch + permission evaluations
decided     : HashMap<(workspace, node), bool>
out         : Vec<Row>
k           = leg_k
redraws     = 0

loop {
    ft   = fulltext_leg(k)     // workspace_ids + shape_types pushed
    vec  = vector_leg(k)       // workspace set pushed into the engine
    fused = rrf(ft, vec, w_ft, w_vec)        // BOTH rank maps recomputed from
                                             // THIS run only -- see 7.4

    for hit in fused {                       // in score order
        if out.len() == limit || checks_left == 0 { break }
        if decided.contains_key(&hit) { continue }
        if hit.workspace_id.is_empty() { warn!(); continue }   // existing guard
        let node = fetch(hit); checks_left -= 1;
        let node = rls_filter_search_hit(node, auth, ...);     // SHARED helper
        let ok   = node.is_some() && residual_matches(&node);  // SAME loop
        decided.insert(hit, ok);
        if ok { out.push(row(node)) }
    }

    if out.len() == limit { break }
    legs_exhausted = ft.len() < k && vec.len() < k
    if legs_exhausted || redraws == 1 || k >= budget || checks_left == 0 { break }
    k = (k * 4).min(budget); redraws += 1;
}
return out                                    // possibly fewer than `limit`
```

- **`scored.truncate(limit)` is deleted.** Truncation moves into the emitting
  loop: count rows actually *yielded*. The existing `try_stream!` already yields
  one at a time, so this costs no extra memory beyond the candidate id list.
- **RLS drops and residual-predicate drops are the same phenomenon and share one
  loop.** Two loops would be this codebase's signature bug.
- **At most ONE re-draw** (≤ 2 leg runs). A second search at a larger `k`
  re-returns the same prefix, so a third round buys almost nothing while costing
  a third full graph walk; memoising `decided` is what makes the second round
  cheap, and the second round is where nearly all the benefit is.
- The per-hit `rls_filter_search_hit` (`table_function.rs:36`, called at `:392`
  and `:662`) is **unchanged and stays the sole authority**. Add a comment at the
  push-down site saying so: the next reader who sees `scope=[library]` in the
  plan will be tempted to conclude the rows are already authorised. They are
  not — workspace is one of four dimensions (workspace, path, node_type, REL
  condition) plus field filtering.

### 7.4 Never merge ranks across two runs

When the loop re-draws at a larger `k`, **both** rank maps are recomputed from
that run and fusion is redone from scratch. HNSW is approximate: a wider search
can reorder, so ranks stitched from two runs would make `vector_rank` a lie in a
column that says otherwise.

### 7.5 When the legs run dry before `limit`

1. **Return what you have.** Never pad. Never error.
2. **The caller is told nothing.** `emitted < limit` because nothing matched and
   `emitted < limit` because 94 matches were unreadable **must be
   indistinguishable client-side**, or the function is a differential oracle.
   This is the one place least-privilege beats least-surprise on purpose.
3. **The operator gets everything**, in exactly one `warn!` per statement:

```rust
warn!(function = "HYBRID_SEARCH", tenant_id, repo_id, branch, user_id,
      scope_spec = "ALL READABLE", scope = ?["handbook","library"],
      catalog = 7, readable = 2,
      requested = 10, emitted = 3, leg_k = 200, redraws = 1,
      candidates = 137, dropped_permission = 94, dropped_residual = 11,
      dropped_missing_node = 2, dropped_no_workspace = 0,
      legs_exhausted = true,
      "search returned fewer rows than requested");
```

   Today the code cannot tell you which cause applied — and that is precisely
   why the dead-vector-leg bug and the RLS-truncation bug both presented to
   operators as "returns fewer rows than I asked for". Note also that a stale
   Tantivy reader (`ReloadPolicy::OnCommitWithDelay`) produces the identical
   empty-result signature, so `candidates` vs `dropped_permission` is the only
   thing that separates reader lag from RLS without a bisect.
4. **Reachable without log access:** `EXPLAIN ANALYZE` reports the same
   counters. For a programmatic caller, the documented has-more probe is *ask for
   `k+1` and check whether you got it*. No new column.

---

## 8. Push the workspace set into both legs — yes

**Fulltext leg: free.** `workspace_ids: Option<Vec<String>>` already exists and
`add_workspace_filter` already builds a `TermQuery` for one and a `Should`
boolean for many. `table_function.rs:359` passes `None`; `:547` passes at most a
one-element vec. This is a call-site change.

**Vector leg: widen the type, and be honest about what it buys.** Change
`HnswIndexingEngine::search` / `search_with_threshold` from
`workspace_id: Option<&str>` to a set, and `SearchRequest::workspace_filter:
Option<String>` → `workspace_filters: Vec<String>` **in the same commit** — they
are the two mirrored paths inside that crate and `search_with_threshold` /
`search_chunks` already carry duplicate copies of the threshold logic.

What it buys:

1. **Rank honesty** (§0) — the correctness argument.
2. **Recall.** Every candidate slot is spent on a workspace the caller can read
   instead of one RLS will delete. Over-fetch treats the symptom; push-down
   removes the cause. Do both — over-fetch still covers path-level and
   REL-condition RLS, which cannot be pushed down at all.
3. **The scope becomes a representable fact** — one resolved set, consumed by
   both legs, the fast reject and `EXPLAIN`. Today "which workspaces did this
   search cover" is the *absence* of a filter, which is exactly why it was
   discoverable only by accident.

What it costs, honestly:

- **usearch has no attribute filtering.** The engine still walks the whole graph
  and `retain`s after fetching `k * 10` (`engine/search.rs:74-86`). A set makes
  it *less wasteful*, not *selective*. A narrow scope inside a large index still
  comes back short, and the log must say `legs_exhausted=true` while the real
  cause is that the ANN walk never visited that region. **State this in the doc
  comment**, or the resulting bug gets filed against RLS.
- Replace the `k*10 if Some else k*2` guess with: `k * 10` whenever any
  workspace filter is present, capped at `SEARCH_LEG_CAP`, and let the shortfall
  surface in §7.5 rather than pretending a selectivity estimate exists.
- **Do not shard the graph per workspace.** One graph per
  `(tenant, repo, branch)` stays. Per-workspace subgraphs would multiply index
  memory and rebuild cost by the workspace count, turn the ~60 s snapshot lag
  (`engine/lifecycle.rs:24`) into N lags, and degrade cross-workspace RAG — the
  thing this is for — into searching N graphs and merging.

**Cheap pre-reject before any fetch:** drop from the universe every workspace no
Read permission's `ScopeMatcher` matches. It is the same set §5.2 computes — one
computation, three uses (legs, pre-reject, `EXPLAIN`). It is a sound *upper
bound* only; path, node-type and REL conditions still filter rows, so it can
never replace the per-row check.

---

## 9. Ranking controls

### 9.1 Two weights, and `0` skips the leg — yes

```
score = w_ft / (RRF_K + rank_ft) + w_vec / (RRF_K + rank_vec)
```

Relative and unnormalised; defaults `1.0` / `1.0` reproduce today's arithmetic
**exactly** (not merely order-preservingly — which is why two weights beat P2's
single `semantic_weight`, whose 0.5 default halves every published score).

**A weight of 0 skips the leg entirely** — no Tantivy query, no embedding round
trip. That is what makes `KNN` and `FULLTEXT_SEARCH` thin wrappers over one
branch instead of a third implementation, and it is what a cross-lingual RAG
caller needs: a German query against an English corpus has near-zero lexical
overlap, so the fulltext leg contributes *noise ranks* that RRF fuses at full
weight and that actively displace correct vector hits.

**`fulltext_weight` and `vector_weight` both zero is an error.**

**Critically: `vector_weight => 0` must skip embedding-provider *resolution*,**
i.e. it must not trip the hard error at `table_function.rs:505-524` ("this tenant
has no enabled embedding configuration"). Otherwise a tenant with no embedder
cannot run a deliberately fulltext-only hybrid query — strictly worse than
today. The existing error stays exactly as it is for `vector_weight > 0`: a
vector leg that silently does nothing is the production shape of the last bug.

### 9.2 RRF `k = 60` — not exposed. Unanimous, and correct

Nobody can tune it without a labelled relevance set, and anyone who has one is
doing offline evaluation, not writing a query. It is a *global* constant: varying
it per query makes two callers' scores incomparable — including one agent's
scores across two turns, which is what breaks a rerank-and-threshold loop. And
exposing it invites cargo-cult tuning that **hides real bugs**: someone whose
vector leg is dead will find that lowering `k` "improves" results and never find
the fault. That is not hypothetical — the dead vector leg in this very file
presented as plausible ranking with NULL `vector_rank` and nothing logged.

The HTTP handler *does* expose it (`HybridSearchQuery::k`, default 60). That is
not an argument for the SQL surface; it is an argument for deleting the second
implementation (§11). Until then the HTTP `k` parameter is accepted, ignored,
and documented as deprecated.

### 9.3 `max_distance` — yes, and this one is urgent

`DEFAULT_MAX_DISTANCE = 0.6` silently discards every candidate beyond cosine
distance 0.6 **before fusion**. For a hybrid query that is a hidden recall
cliff: a document at 0.61 contributes nothing to the vector leg and, if it was
not a lexical match either, is invisible. There is currently **no reachable way
to widen it from SQL** — this session established that
`WHERE embedding <=> EMBEDDING(…) < 0.99` arrives as `threshold=None` and 0.6
applies regardless — which makes it a hardcoded corpus boundary nobody wrote
down.

Spec: `max_distance =>` plumbed to `search_with_threshold`. Default stays 0.6,
so nothing changes unless asked.

**Separately, and not in this commit:** fix the `VectorScan` path so its
`< constant` predicate actually reaches `max_distance`. Those are the two
mirrored paths; fixing one is how you earn a third bug report. Meanwhile
`vector_distance` is emitted per row, so client-side thresholding is the honest
interim escape hatch.

### 9.4 `language` — yes, and it is a live bug

`FULLTEXT_SEARCH` requires it as argument 2; `HYBRID_SEARCH` hardcodes
`language: "en"` (`table_function.rs:549`). On a multilingual corpus the hybrid
function's fulltext leg runs an exact term filter for `"en"` against a German
query while the standalone function does it correctly. Two mirrored paths,
already drifted.

Default is `ctx.default_language` (already on `ExecutionContext`), **not** a
hardcoded `"en"` and **never** inferred from the query text — that would be a
silent behaviour change dressed as an improvement. Values are ISO 639-1 only
(§1.2); the shipped `'english'` example is corrected in the same commit.

### 9.5 Not exposed

Per-leg limits, HNSW `ef_search`, `ScoringConfig` chunk scoring, choice of
fusion algorithm. Each changes results while giving the caller no way to know
whether they improved.

---

## 10. `KNN` — implement it

It is in the analyzer's TVF list (`from_clause.rs:181`), has a `TableDef`, and
`ddl_keywords/functions.rs:70-77` ships `SELECT * FROM KNN(EMBEDDING('search
query'), 10)` as its example — and `execute_table_function` has no arm, so every
such query dies at runtime with `"Unsupported table function: KNN"`. Nothing can
regress.

Implement it as `HYBRID_SEARCH` with `fulltext_weight` forced to 0 and
`vector_weight` forced to 1 — the **same** loop, scope resolver, RLS pass,
over-fetch and column set (fulltext columns NULL). A RAG caller genuinely wants
pure vector: cross-lingual retrieval gets nothing but noise from the fulltext
leg, and forcing them through `HYBRID_SEARCH` also forces them to maintain a
fulltext index they do not need.

Argument 1 accepts, in this order:

1. `Literal::Text` — embedded with the tenant's provider, like the other functions.
2. `EMBEDDING(<text literal>)` — **unwrapped to its inner text literal** and
   treated as (1). Identical result, and it needs no scalar-expression evaluator
   at table-function bind time. This keeps the advertised example working.
3. `Literal::Vector` — used directly, no embedding call. (The variant exists at
   `typed_expr/expressions.rs:256`.) Dimension mismatch against the tenant's
   configured dimension is a hard error naming both numbers.

Anything else is an error naming the three accepted forms.

`vector_weight` / `fulltext_weight` / `language` are rejected as arguments to
`KNN` (there is no fulltext leg to weight and no analyzer to choose).

---

## 11. One implementation, not three

New module `crates/raisin-sql-execution/src/physical_plan/search/`:

| file | owns |
|---|---|
| `scope.rs` | `WorkspaceSet`, `parse_workspace_scope`, `resolve_scope`, the cache |
| `args.rs` | the positional + named argument grammar, one parser for all three functions |
| `legs.rs` | fulltext and vector leg dispatch, push-down construction |
| `fusion.rs` | `RRF_K`, weighted RRF, the total-order sort |
| `emit.rs` | the §7.3 loop: fetch → `rls_filter_search_hit` → residual → yield |
| `mod.rs` | `SEARCH_OVERFETCH`, `SEARCH_LEG_CAP`, the counters struct |

`raisin_core::services::rls_filter` gains `readable_workspaces` (§5.2) so it
lives beside `filter_node` and cannot drift from it.

**Then collapse the mirrors, in the same release.**
`raisin-transport-http/src/handlers/hybrid_search/rrf.rs::merge_with_rrf` is a
full second implementation that still keys `HashMap<String, RrfScoreEntry>` on
`node_id` alone — the exact bug fixed on the SQL side by
`type HitKey = (String, String)` (`table_function.rs:538`) — and its handler
module contains no `auth` or RLS reference at all. The MCP `search` tool
(`handlers/mcp/services/search.rs`) is a third path. Under this repo's own named
#1 bug class, shipping the SQL semantics without rewiring HTTP and MCP onto the
shared module would be correct in one of three places. `raisin-sql-execution` is
already a dependency of `raisin-transport-http` under `storage-rocksdb`, which
both surfaces require.

**Sequencing.** (1) the predicate-drop fix (§6.1), its own commit, its own
tests. (2) the `search/` module with the SQL TVFs rewired onto it, HTTP and MCP
rewired, `rrf.rs`'s mirror deleted. (3) the required `workspaces` argument, the
`KNN` executor, the unified column set, and the book rewritten — one release.
Do **not** land (3) before (2), or HTTP and MCP keep an implicit-everything
default while SQL requires an explicit one, which is the drift this exists to
close.

---

## 12. EXPLAIN

`operators/describe.rs:33-37` prints only the name and alias. Add the resolved
universe, the push-downs and the residual, so an operator can see *why* a query
returned three rows:

```
TableFunction: HYBRID_SEARCH
  scope=[handbook, library]  spec='ALL READABLE'  (catalog=7, readable=2)
  leg_k=200  weights=(ft 1.0, vec 1.0)  max_distance=0.60  language='de'
  pushed:   shape_types=['docs:Document']   (fulltext leg only; over-matches,
                                             residual filter is authoritative)
  residual: properties->>'status'::String = 'published'
```

The executor logs the same line at INFO on every search. Nobody discovers the
scope by accident twice.

---

## 13. Migration — does an already-written query change meaning?

**The chosen default does not silently change any query's meaning: the 2-argument
form becomes a hard error.** But three other changes in this release DO alter
results with no error, and one change is genuinely silent. All four must be in
the release note.

| written today | after | risk |
|---|---|---|
| `HYBRID_SEARCH('q', 10)` | **hard error** naming both migrations | loud, zero drift |
| `FULLTEXT_SEARCH('q','en')` | **hard error** for want of `workspaces` | loud |
| `HYBRID_SEARCH('q', 10, 'library')` | unchanged, forever | none |
| `FULLTEXT_SEARCH('q','en','library')` | **hard error** (today arg 3 is silently ignored and the search is repo-wide) | loud — and it repairs a silent bug |
| `HYBRID_SEARCH('q', 10, workspaces => 'library')` | now means what it says (today: limit 10, **all** workspaces) | **the one genuinely silent change** |
| `… FROM HYBRID_SEARCH('q',20,'ws') WHERE node_type='X'` | the `WHERE` **now actually runs** | fewer rows, correctly, with no error |
| an RLS-restricted caller asking for 10 | up to 10 rows instead of 3 | strictly more correct, more rows |
| `SELECT * FROM HYBRID_SEARCH(…)` | gains `created_at`, `updated_at` | breaks positional/pgwire-by-index consumers |
| `SELECT * FROM KNN(…)` | works instead of erroring | pure gain |
| `ORDER BY col <=> EMBEDDING(…) LIMIT k` | untouched | none |

**Meaning-preserving by construction:** weights default `1.0/1.0`, so scores are
numerically identical; `RRF_K` stays 60; `max_distance` stays 0.6; the third
positional keeps its meaning; workspace push-down returns the same rows from
better candidates.

### The release note must say, verbatim in substance:

> **1. Search functions now require an explicit workspace scope.**
> `HYBRID_SEARCH(query, k)` and `FULLTEXT_SEARCH(query, language)` previously
> searched **every workspace in the repository** — undocumented behaviour that
> returned build artifacts and binary assets to callers asking for documents.
> They now fail with an error naming the fix. Add `workspaces => '<name>'` for
> one, `'a, b, c'` for several, `'content-*'` for a family, or
> `workspaces => 'ALL READABLE'` for every workspace you may read — which is what
> your call used to do. The three-argument form
> `HYBRID_SEARCH(query, k, 'workspace')` is unchanged and keeps working.
>
> **2. `WHERE` over a table function was silently ignored and is now applied.**
> This affects `HYBRID_SEARCH`, `FULLTEXT_SEARCH`, `KNN`, `GRAPH_TABLE`,
> `NEIGHBORS`, derived tables and CTE scans — not just search. Queries that
> relied on the unfiltered result will return fewer rows. No error is raised;
> the rows they were getting were wrong.
>
> **3. `LIMIT k` now means k rows delivered, not k candidates budgeted.**
> Permission filtering used to run *after* the result was truncated, so a
> restricted caller asking for 10 could receive 3. They now receive up to 10.
> Anyone who compensated by over-requesting (asking for 50 to reliably get 10)
> will now get 50. Never more than the limit — but retune it.
>
> **4. `HYBRID_SEARCH(q, k, workspaces => 'x')` changes meaning silently.**
> Named table-function arguments parsed but their names were discarded, so this
> ran as *limit 10, every workspace*. It now scopes to `x`. This is the only
> query shape whose meaning changes without an error. The behaviour was
> undocumented and produced results no author would have accepted, so anyone who
> wrote it wanted the new meaning; the in-repo count of such call sites is zero.
> Grep your stored SQL, agent prompts and `raisin.sql()` calls for `=>` inside a
> table-function call before upgrading.
>
> Also: `HYBRID_SEARCH`'s full-text leg no longer hard-codes English — it uses
> the repository's default language, or `language => 'de'`. The language
> argument takes ISO 639-1 codes (`'en'`, not `'english'`); the previously
> documented `'english'` matched no documents. `HYBRID_SEARCH` and
> `FULLTEXT_SEARCH` now return the same columns; `SELECT *` gains `created_at`
> and `updated_at` on hybrid and the rank/distance columns on full-text.
> `KNN(...)` is implemented and no longer fails at execution.

Nothing here touches stored data, indexes or the replication payload. It is
entirely a query-surface change, so a rollback is a revert with no data to
repair — which is the strongest argument for taking the loud break now instead
of shipping a compatibility flag whose removal would be a second, harder
migration later.

---

## 14. Tests that must exist before this ships

1. **The system-caller regression, written first.** `auth_context == None`,
   `is_system`, and `is_system_admin` each resolve `'ALL READABLE'` to `All`.
   Miss any and every internal search silently narrows — indexing jobs, MCP
   tools, agents running as system.
2. **`Empty` is not `All`.** A caller with no Read grant anywhere returns zero
   rows and neither engine's `search` is invoked. Assert the call count.
3. **`permissions() == None` denies**, matching `filter_node` — not `All`.
4. **No truncate-before-RLS.** A caller readable in 1 of 12 workspaces asks for
   10 and receives 10 when 10 readable matches exist.
5. **`WHERE` now applies** over each of the six table functions, and over a
   derived table and a CTE scan.
6. **Push-down equivalence.** `workspaces => 'a, b'` returns byte-identical rows
   whether or not the workspace filter reaches the legs (run with push-down
   forced off). If these ever disagree, the pool and the filter have forked.
7. **`shape_types` over-match is invisible.** A node whose *archetype* is `X`
   but whose `node_type` is not must be absent from
   `… WHERE node_type = 'X'` despite being admitted by the push-down.
8. **Weight 0 skips the leg** — and `vector_weight => 0` succeeds on a tenant
   with **no** embedding configuration.
9. **Ranks are never merged across runs**: force a re-draw and assert every
   emitted `vector_rank` is consistent with the final run's ordering.
10. **The named-argument name survives** analysis (`workspaces => 'library'` is
    not a positional).
11. **Scope errors are not an existence oracle**: a non-existent workspace and an
    unreadable one produce byte-identical error text.
12. Existing `search_table_function_rls.rs`, `hybrid_search_workspace.rs` and
    `hybrid_search_query_embedder.rs` are updated to the new surface in the same
    commit.

---

## 15. Open risk

The weakest joint is the vector leg. usearch cannot filter by attribute, so a
pushed-down workspace set is still a post-filter over a fixed over-draw: a
narrow scope inside a large tenant index returns short, and the log will say
`legs_exhausted=true` truthfully while the real cause is that the ANN walk never
visited that workspace's region. No `fetch_k` heuristic fixes this — only
index-side filtering would, and that is a separate project. Combined with
`max_distance`, a narrow scope starves twice.

Second: the required-scope break has no measurable blast radius outside this
repo. Stored tenant SQL, agent prompts and `raisin.sql()` calls cannot be
surveyed from here. The mitigation is the error message, not a survey — and the
message is the only thing standing between a caller and a silent corpus change,
which is why it names all four migrations rather than just failing.
