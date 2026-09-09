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
