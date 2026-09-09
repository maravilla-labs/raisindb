# Type membership (`IS_A` / `HAS_MIXIN`) and compound-index ownership

**Measured 2026-09-09 against `raisin-server 0.5.3`** (the current release binary at
`~/.raisindb/bin/raisindb`), repo `studio`, branch `main`.

Inheritance and mixins are advertised as a first-class modelling concept: `extends`,
`mixins` and `overrides` resolve correctly, membership is materialized, and
`IS_A` / `HAS_MIXIN` exist in SQL. In practice the concept is **not usable as a
foundation**, for three separate reasons, plus one adjacent index bug that produces silent
wrong answers on ordinary queries.

This note states what was measured and proposes the fix.

---

## Measured

### A. Membership is stored as ordinary JSON properties, and it SURFACES

`Node::set_effective_types` (`raisin-models/src/nodes/node/core/definition.rs:477`) writes
`$mixins` and `$supertypes` into the node's `properties` map as a
`PropertyValue::Array`. They come straight back out of a `SELECT`:

```sql
SELECT properties FROM stories WHERE path = '/blueprints/starter/home'
-- keys: ['$mixins', '$supertypes', 'content', 'description', 'slug', 'title', …]
```

So engine-internal type bookkeeping is shipped to every SQL consumer, every REST response
and every function payload, on every node, forever. It is also client-writable-looking (it
is stripped on write, correctly — but only because someone remembered to).

### B. `IS_A` / `HAS_MIXIN` are full scans, and cannot ever be otherwise

`physical_plan/eval/functions/type_check/mod.rs` documents them as pure array-membership
tests over the materialized sets — "these functions never resolve NodeTypes at query time".
That makes them cheap *per row* and unusable *per query*: there is no index on
`$supertypes`, so every call scans.

On ~2 300 commerce nodes:

| query | time |
|---|---|
| `WHERE node_type = 'studio:Folder'` | **7 ms** |
| `WHERE IS_A(properties, 'studio:Folder')` | **227 ms** |
| `WHERE IS_A(properties, 'raisin:Folder')` | **160 ms** |

~25× slower at 2 300 rows, and the gap widens linearly.

### C. Membership was not stamped at all by the transaction layer (fixed on main)

**Corrected 2026-09-09.** The first version of this note blamed
`is_validate_schema_enabled()`. That was wrong, and the correction matters
because it changes what still needs doing.

What v0.5.3 actually did (`git show v0.5.3:…/add_node.rs`):

```rust
// 5a. Schema validation
if tx.is_validate_schema_enabled() {
    let validator = tx.create_validator();
    validator.validate_node(workspace, &normalized_node).await?;   // validate ONLY
}
```

`validate_and_stamp` did not exist in v0.5.3 — the transaction layer validated
and never stamped, so every node written through SQL DML, the WebSocket create
handler or a child POST carried no membership. That is the whole explanation for
the coverage measured on the live server:

| workspace | type | `node_type =` | `IS_A()` |
|---|---|---:|---:|
| commerce | `studio:Folder` | 67 | 13 |
| commerce | `commerce:ProductVariant` | 107 | 1 |
| commerce | `commerce:Order` | 160 | 1 |
| commerce | `commerce:Product` | 27 | **0** |
| stories | `studio:Page` | 67 | 5 |
| people | `studio:Contact` | 62 | **0** |
| events | `studio:Event` | 20 | **0** |

It was **already fixed on `main`** by the `validate_and_stamp` unification,
before this work. Verified on a current build: schema validation IS on for SQL
DML (a missing required property is refused), and an INSERT now stamps.

What remained, and what this change adds, is narrower: the call was still gated,
so the paths that deliberately turn validation OFF — `engine/acl.rs`, the Cypher
executor, bulk import, replication — still produced unstamped nodes. Membership
is engine metadata, not user schema, so it is now stamped either way via
`stamp_effective_types`, which fails open.

The `allowed_children` consequence is likewise narrower than first stated: it
matches the family through `$supertypes`, so it was unenforced only on those
validation-off paths, not on ordinary SQL DML.

**And it is slow.** On ~2,300 commerce nodes, `IS_A` took **160–227ms** against
**7ms** for the equivalent `node_type` predicate — roughly 25x, and a full scan by
construction. That half of the problem was untouched by the stamping fix and is
what the membership index addresses.

### D. Compound-index selection ignores which NodeType owns the index

Independent of the above, and the most dangerous of the four because it corrupts ordinary
queries.

`CompoundIndexDefinition` (`raisin-models/src/nodes/properties/schema.rs:185`) stores
`name`, `columns`, `has_order_column` — **not the NodeType that declared it**. So
`try_match_compound_index` (`planner/compound_index.rs:102`) matches on property NAME alone
and cannot check ownership. The other half, `engine/mod.rs:604`:

```rust
// No `node_type =` in the WHERE clause. A hierarchy query is
// usually written without one, so fall back to every compound
// index on the branch rather than planning as if none existed.
None => helpers::load_all_compound_indexes(…)
```

Falling back to every index is sound only if matching then verifies ownership. It does not.
Measured consequence — a `commerce:StockReservation` index answering a `studio:Event` query
in a different workspace:

| query | plan | rows |
|---|---|---:|
| `events`, `node_type` + `status` | `CompoundIndexScan: event_status_start` | 3 ✔ |
| `events`, `status` only | `CompoundIndexScan: reservation_status_expires` | **0** ✘ |
| `stories`, `status` only | `CompoundIndexScan: reservation_status_expires` | **0** ✘ |
| `events`, `node_type::String` + `status::String` | `CompoundIndexScan: reservation_status_expires` | **0** ✘ |

One index is currently capturing every unqualified `status` query in the repository. Any
common property name (`status`, `code`, `email`, `slug`) is the same landmine: the first
NodeType to index it wins every unqualified query on it, across all workspaces.

---

## Proposed fix

### 1. Membership becomes an INDEX, not a property

Stop writing `$supertypes` / `$mixins` into the properties bag. Maintain the membership set
as a **system index** written on the same path that already computes it, keyed for lookup:

```
(supertype_name, node_id) -> ()
(mixin_name,     node_id) -> ()
```

Then:

- `IS_A(...)` / `HAS_MIXIN(...)` plan as an **index lookup**, not a scan — the property
  becomes first-class and fast, which is the whole point of offering it;
- membership **never surfaces** in `SELECT properties`, REST payloads or function inputs;
- it cannot drift from the NodeType definition through a stray property write, and it
  cannot be spoofed from input;
- the reserved-key stripping on write becomes unnecessary rather than load-bearing.

Keep the SQL functions as the read surface — they are the right API; only the storage
changes. A `$supertypes`-shaped view can remain available to the function runtime if
`api_wrapper.js:263` needs it, but read from the index rather than the row.

### 2. Stamping must not be optional

Membership is engine metadata, not user schema. Compute and index it on **every** node
write regardless of `validate_schema`. Validation of user-supplied properties can stay
toggleable; type membership should not, because turning it off silently produces rows that
lie to `IS_A` and bypass `allowed_children`.

### 3. Backfill

Existing rows carry no membership. Ship a migration that walks each branch and writes the
index from each node's `node_type` + resolved NodeType. Without it the feature stays
0–19% correct on every existing installation.

### 4. Compound indexes learn their owner

1. Add the owning node type to `CompoundIndexDefinition`, populated when read off the
   NodeType schema.
2. In `try_match_compound_index`, skip an index whose owner is not implied by the query —
   require a `__node_type` equality naming the owner, or a subtype of it (which the new
   membership index can answer).
3. A type-owned index must **never** serve a query with no `node_type` predicate. Falling
   back to a scan is correct; answering from one arbitrary type's index is not.
4. Bump the definition version constant so entries written under the old reading are
   invalidated.

Regression test: a `status`-leading index on type A must not be selected for a query on
type B, nor for an untyped query.

### 5. While the cast is index-eligible, it selects the wrong index

`f1d61a51` made `::String` index-eligible. Combined with (D) that is worse than before: the
cast defeats the `node_type` predicate and routes the query to a foreign index. Either the
cast must preserve node-type qualification, or it must not be index-eligible. Note the
downstream docs still recommend adding `::String` as a workaround from an earlier era —
those recommendations are now actively harmful and should be retracted with the fix.

---

# Addendum: what a row actually costs (measured 2026-09-09)

Prompted by "why does a 20k-row listing take seconds". Two findings, one of them
a correction to numbers reported earlier in this note's lifetime.

## The earlier numbers were a DEBUG build

Everything measured before this section came from `target/debug`. Release is
~6x faster on the read path, and it changes the conclusions:

| query (16,700 matching rows) | debug | release |
|---|---:|---:|
| `COUNT(*)` | 33 ms | **8 ms** |
| `SELECT path` LIMIT 5000 | 1265 ms | **204 ms** |
| `SELECT path` (all) | truncated at ~12,999 in 3.2s | **16,700 rows in 672 ms** |
| `SELECT path, properties` (all) | truncated at 7,999 | **16,700 rows in 1061 ms** |

So the truncation that prompted this was largely a debug artifact: in release a
20k-row listing lands around 0.8–1.3s and never reaches the budget. The budget
still fires around 50–80k rows, which is why the silent-subset problem below was
worth fixing on its own terms rather than dismissing.

## Where the time goes

Per 5000 nodes, release, instrumented inside `get_at_revision_impl`:

| step | per 5000 | per node |
|---|---:|---:|
| `get_revision_at_or_before` | 18–45 ms | ~4–9 µs |
| RocksDB point get | 4–6 ms | ~1 µs |
| **msgpack deserialize** | **57–66 ms** | **~12 µs** |
| `materialize_path` | 0 ms | free* |
| `populate_has_children` | 0 ms | free |

\* free *here* because the fixture is root-level. Path is NOT stored in the blob
(`StorageNode` omits it deliberately, for O(1) moves) and is walked from the
parent chain, so a deep tree pays for it.

Above the storage layer the scan adds ~22 µs/row of async dispatch and per-row
overhead; RLS is 0.4 µs, locale 0.3 µs and row building 0.7 µs — all noise.

**The single removable cost is the msgpack deserialize.** `SELECT path` decodes
every property of every node and throws them away.

Two things were tried and did NOT help, recorded so they are not retried:

- **Hoisting the branch-head resolution out of the scan loop.** `NodeRepository::get`
  with `max_revision: None` resolves the head per call, so a 16,700-row scan
  resolved it 16,700 times — but it is cached and the change measured flat
  (207 ms vs 204 ms). It was kept anyway, for a different reason: per-call
  resolution means a head that advances mid-scan makes later rows come from a
  newer snapshot than earlier ones. One revision for the whole scan is a
  consistent read; that is a correctness fix, not a speed one.
- **`populate_has_children`**, suspected because `dispatch_get` passes `true`
  unconditionally. It measured at zero.

## The optimisation NOT taken, and why

Projection pushdown — skipping the node fetch when the projection needs only
`id`/`path` — is the real fix and would take a path-only scan close to the
`COUNT(*)` number. It was deliberately not built here because it is only sound
under four conditions that must ALL hold, and getting any of them wrong is a
correctness or security bug rather than a slow query:

1. **The projection must need no properties.** A residual filter is the trap:
   `IS_A(x) AND status='y'` evaluates `properties->>'status'` on the ROW, so a
   row without properties silently matches nothing.
2. **RLS must not inspect the node.** `rls_filter::filter_node` returns early on
   `auth.is_system` or `permissions.is_system_admin` — both node-independent, so
   this IS decidable once per scan. For any other principal RLS reads the node's
   path, workspace and properties, and skipping the fetch would be an RLS
   BYPASS, not a speedup.
3. **Head queries only.** The `node_path` column family (id → path) is
   maintained for O(1) moves and is not revision-scoped, so a point-in-time query
   cannot read paths from it.
4. **Tombstones.** The fast path must still see a deletion that the MVCC walk
   would have found.

A narrower version is safe and probably worth doing first: decode only the
header fields when no properties are projected. `StorageNode` is a serde struct
over msgpack, so this needs care about field order rather than a new keyspace,
and it helps every query rather than only path-only ones.

---

# Addendum: why a SUBTYPE must not borrow its ancestor's compound index

Tried, measured, and reverted 2026-09-09. Recording it because the idea is
intuitive, it is asked for, and it is a **1000x pessimisation** rather than the
speedup it sounds like.

The premise is reasonable: `extends` merges `compound_indexes`, so an index
declared on `party:Person` is inherited by `party:VipPerson`, and the planner
refusing to use it for a VipPerson query looks like a bug. It is not.

**An index NAME is a workspace-global keyspace.** Every member of the family
writes into it, so `person_status_name` holds all 16,700 Persons AND the 10
VipPersons. `node_type` is not one of its columns, so a VipPerson query served
from it must scan the whole family and narrow with a residual filter. Measured
on that fixture:

| plan | rows | time |
|---|---:|---:|
| `PropertyIndexScan __node_type=party:VipPerson` + filter | 10 | **3 ms** |
| `CompoundIndexScan person_status_name` + residual `node_type` | 10 | **budget exceeded (400)** |

The type-scoped property index goes straight to the 10 rows. The ancestor's
compound index reads 16,700 to find them — and on this fixture it ran out of the
scan budget before finishing, which is the only reason the regression was
visible at all rather than merely slow.

The general rule: borrowing an ancestor's index is only a win when the subtype is
a LARGE FRACTION of the family, or when the index's leading columns are selective
enough that the family scan is small. Here `status='active'` matched ~16,700 of
16,760 — the index narrowed nothing, and then the residual threw away 99.94% of
what it read.

**Doing it properly needs costing, and the statistics do not exist.** The choice
is between (index selectivity x subtype fraction) and (1 / node_type count), and
`SchemaStats` carries the NUMBER of distinct node types, not the row count per
type. Without per-type cardinality the planner cannot tell the good case from the
catastrophic one, so it declines uniformly.

**The modelling answer needs no engine change: declare the index on the type you
actually query.** A name is a keyspace, so `ALTER NODETYPE 'party:VipPerson' ADD
COMPOUND_INDEX 'vip_status_name' ON (status, display_name)` creates a keyspace
holding only VipPersons. Inheritance of a declaration is a convenience for
sharing a definition; it is not a promise that a query on a subtype is targeted.

What IS fixed and should stay: the write path and the rebuild both resolve the
`extends` chain, so a subtype's rows do land in an inherited index's keyspace.
That matters for the family-level query (`IS_A(party:Person) AND …`), which is
exactly the case the keyspace suits.
