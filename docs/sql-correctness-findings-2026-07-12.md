# SQL Correctness Findings — 2026-07-12

Investigation and fixes for three families of SQL-layer correctness defects observed
against a live workload (server 0.1.40-source, RocksDB backend, single-node dev mode).
All fixes are implemented and verified; nothing here is design-only unless explicitly
marked. All paths are relative to the repository root.

Build status: `cargo build --workspace` clean (pre-existing warnings only).
Test status: all new suites pass; `raisin-sql` lib 757/757, `raisin-sql-execution`
lib 453/453, `raisin-rocksdb` lib 482/482, `raisin-functions` lib 141/141; regression
spot-checks (`branch_fork_publish_test` 14/14, `scheduled_invocation_test` 4/4,
`dml_index_and_predicate_maintenance` 3/3, `references_integration_tests` 4/4,
`rocksdb_integration_tests` 25/25) all pass.

---

## Bug Family A — GROUP BY over JSON extraction returns NULL group keys

### Symptom

`SELECT properties ->> 'status'::String AS status, COUNT(*) FROM ws GROUP BY
properties ->> 'status'` returned the correct per-group counts but **every group key
was NULL**. Reproduced identically with and without a `Property` index declared on
the property — a genuine planner/executor bug, not an index-declaration issue (GROUP
BY key materialization never depends on the property index; the scan feeds full
`properties` rows either way).

### Root cause

Three cooperating defects sharing one mechanism. The engine wires
`SELECT <expr> … GROUP BY <expr>` by rewriting the SELECT expression into a column
reference to a canonical group-key column that the aggregate executor materializes.
`properties ->> 'status'::String` is analyzed as `Cast { expr: JsonExtractText{…} }`
— the cast wraps the whole extraction
(`crates/raisin-sql/src/analyzer/semantic/json_ops.rs:88-113`). Then:

1. `exprs_match` had no `Cast` arm
   (`crates/raisin-sql/src/logical_plan/builder/expr_helpers.rs:222`, old
   `_ => false`) → the SELECT expr was never rewritten to the group-key column.
2. `extract_column_name` in the aggregate executor had no `Cast` arm
   (`crates/raisin-sql-execution/src/physical_plan/hash_aggregate.rs:185`, old) — a
   hand-synced duplicate of the builder's `generate_groupby_column_name` that had
   drifted — so the (correctly computed) group key was stored under the fallback
   name `"group_0"`.
3. The Project above the aggregate therefore re-evaluated the raw
   `Cast{JsonExtractText}` against the aggregated row, which has no `properties`
   column → `eval_column` returns NULL
   (`crates/raisin-sql-execution/src/physical_plan/eval/core/mod.rs:176`) → `->>` of
   NULL is NULL (`eval/core/json_eval.rs:121`) → every group key silently NULL,
   while COUNTs (evaluated against input rows) stayed correct.

### Fix (single source of truth for group-key naming)

- New `crates/raisin-sql/src/logical_plan/group_key.rs` —
  `group_key_column_name(expr)`: Column → `table.column`; `properties->>'k'` →
  `table.properties_k`; Function → `NAME(arg)`; **Cast → transparent** (inner
  expr's name, so an uncast projection of the same extraction still finds the key);
  anything else → deterministic structural-hash name (`group_<hash>`) instead of an
  un-lookup-able `"group_N"`. Exported via `logical_plan/mod.rs`.
- `expr_helpers.rs`: `exprs_match` gains a `Cast` arm plus a structural-equality
  fallback so CASE / IS NULL / other group-by shapes are wired instead of silently
  nulling; `generate_groupby_column_name` delegates to the shared function.
- `hash_aggregate.rs`: group columns stored under the shared
  `group_key_column_name`; the drifted local duplicate deleted.
- `logical_plan/operators/plan_impl.rs`: `LogicalPlan::schema()` for Aggregate
  reports the same canonical names (display-only).

### Indexed vs unindexed verdict

No divergence; the bug affected both equally. No index declaration is required for
GROUP BY key correctness.

### Tests

`crates/raisin-sql-execution/tests/group_by_json_extraction.rs` (new, 2 tests
covering both index variants, with cast/alias permutations and controls; failed
before the fix, pass after) plus 4 unit tests in `group_key.rs` pinning the naming
contract including cast-transparency.

---

## Bug Family B — `path LIKE` scan anomalies

### Symptoms and root causes

**(a) `path LIKE '/parent/name-%'` matched nothing.** The planner correctly rewrote
the LIKE to a `PrefixRange`
(`crates/raisin-sql-execution/src/physical_plan/planner/filter_analysis.rs:525`),
but the executor
(`crates/raisin-sql-execution/src/physical_plan/scan_executors/prefix_scan.rs`)
treated the string prefix as a **parent node path**: it trimmed the trailing
fragment, did `get_by_path('/parent/name-')`, found no node there, and returned 0
rows. The scan was a tree traversal, not a string-prefix scan, so any prefix not
ending on a `/` boundary was unanchorable.

**(b) `path LIKE '/parent/%'` included `/parent` itself.**
`scan_descendants_ordered_impl`
(`crates/raisin-rocksdb/src/repositories/nodes/queries/scanning/mod.rs:66-76`)
pushed the traversal root itself into the result; the residual LIKE was never
re-applied (see (c)), so the parent leaked out. The same defect made
`DESCENDANT_OF('/x')` include `/x`, contradicting the row-eval semantics in
`eval/functions/hierarchy/descendant_of.rs:146-149`.

**(c) `path LIKE '/jobA/%' AND node_type = 'X'` returned rows from other
subtrees.** `combine_canonical_predicates`
(`crates/raisin-sql-execution/src/physical_plan/planner/predicate_ops.rs`, old
lines 279-290) unconditionally **dropped** `PrefixRange` / `ChildOf` /
`DescendantOf` / `References` from every residual filter. Whenever a different
access path won the scan choice (a PropertyIndexScan for `node_type` when equality
is estimated ≥10× more selective — `index_selection.rs:107` — or NodeIdScan /
PathIndexScan / CompoundIndexScan, which win unconditionally at
`scan_planning/mod.rs:108/139`), the hierarchy predicate was silently discarded and
every `X` node in the workspace was returned.

**(d) Rows for long-updated/deleted values lingered; occasional apparent
duplicates.** Confirmed sub-defect: neither write path tombstoned stale
**property-index** old-value entries on UPDATE
(`repositories/nodes/crud/update.rs` and
`transaction/context/nodes/create/core/put_node.rs` tombstoned compound / unique /
spatial old values but not `PROPERTY_INDEX`), and the SQL `PropertyIndexScan`
executor never re-verified the property on the fetched row (unlike the repository's
own `find_by_property_impl`, `queries/property.rs:75`, which re-checks because index
keys are hashed). After a property changed `held → confirmed`, a query for
`'held'` still returned the node; the orphaned entries survived restarts and
inflated index-backed COUNTs. Duplicate physical rows for one node were **not
reproducible**; the most parsimonious explanation is defect (c) itself (cross-subtree
leakage rendering as duplicates), with per-locale row expansion as a secondary
candidate.

### Fixes

1. `planner/predicate_ops.rs:224` — residual filters no longer drop hierarchy
   predicates; every canonical predicate round-trips via `to_expr()` into
   row-evaluable `PATH_STARTS_WITH` / `CHILD_OF` / `DESCENDANT_OF` / `REFERENCES`
   filters (all have row-level eval implementations).
2. `planner/scan_planning/build_scan.rs` — each scan builder removes only the
   predicate its scan *guarantees*: prefix and property-prefix scans keep everything
   as residual (scan = superset access path, filter = source of truth);
   `build_property_index_scan` keeps `JsonPropertyEq` residuals (hashed keys →
   collisions and legacy orphans) while still consuming `__` pseudo-property
   equalities to preserve the COUNT(*) index pushdown.
3. `scan_executors/prefix_scan.rs:102` — distinguishes directory vs name-fragment
   prefixes: name-fragment scans use the path-index string-prefix scan
   (`scan_by_path_prefix`, exact LIKE semantics — fixes (a)); direct-children scans
   anchor at the containing directory; the tree traversal skips its own root
   (fixes (b) and DESCENDANT_OF-includes-parent).
4. `repositories/nodes/crud/indexing/property_indexes.rs:26` — new shared
   `add_stale_property_tombstones` (changed/removed values, published-tag flips,
   `__node_type` / `__name` / `__archetype` / `__created_by` / `__updated_by` and
   timestamp pseudo-props; unchanged triples skipped), wired into **both** write
   paths: `crud/update.rs` and the transactional `put_node.rs:180`, mirroring the
   existing compound/unique/spatial pattern.

### Indexed vs unindexed verdict

No divergence at this storage layer: RocksDB's `add_property_indexes` indexes
**all** properties regardless of the NodeType `index: [Property]` declaration, and
both test variants pass identically. All four symptoms were genuine
planner/executor/storage bugs; applications do not need extra index declarations for
`path LIKE` correctness.

### Documented, not fixed

- Transient duplicate physical rows: not reproducible deterministically (see above).
- `DESCENDANT_OF('/')`-style scans with prefix `/` still traverse via a stored root
  node lookup (pre-existing, out of scope).

### Tests

`crates/raisin-sql-execution/tests/path_like_prefix_scan.rs` (new, 7 tests: one per
symptom (a)-(c) including an id-lookup residual-drop repro, indexed and unindexed
property variants, an update/delete/reopen index-hygiene test for (d), and a
write-churn duplicate-row probe). All failed before the fix except the two
property-variant guards; 7/7 pass after.

---

## Bug Family C — duplicate rows at one path / silent create failures

### Symptom

Two physical rows (distinct ids) at a single path; index scans returned both;
delete-by-path removed one and left the other's index entries orphaned. Separately,
`nodes.create` failures in the embedded function runtime were silently swallowed —
the caller received a success-shaped object.

### Root causes

**Path duplication (core defect):** path uniqueness was enforced only by a
read-check separated in time from the write — a TOCTOU race with no cross-writer
synchronization:

1. Transactional path (function runtime `nodes.create` → SQL INSERT →
   `txn_ctx.add_node`; `NodeService::create`): check at
   `crates/raisin-rocksdb/src/transaction/context/nodes/create/core/add_node.rs:62-76`,
   write deferred to commit
   (`crates/raisin-rocksdb/src/transaction/commit/mod.rs:159`) — a race window
   spanning the entire transaction lifetime. The supposed guard,
   `RocksDBTransaction::check_conflicts`
   (`crates/raisin-rocksdb/src/transaction/core.rs:230-241`), is a documented no-op
   placeholder.
2. Non-transactional repository path: check at
   `crates/raisin-rocksdb/src/repositories/nodes/validation.rs:53-64`, blind write
   at `crates/raisin-rocksdb/src/repositories/nodes/crud/create/add.rs:122-124`.

Two concurrent creates both pass the check and both commit their own atomic
WriteBatch → two rows at one path.

**Silent create failure (masking bug):** the QuickJS runtime's
`__raisin_internal.nodes_create` converts a storage `Err` into an
`{"error": "…"}` JSON string
(`crates/raisin-functions/src/runtime/quickjs/api_nodes.rs:68-71`), but the JS
wrapper `raisin.nodes.create`
(`crates/raisin-functions/src/runtime/quickjs/api_wrapper.js:255-258`) returned it
wrapped as a success-shaped node without checking `.error`. `nodes.delete` /
`nodes.updateProperty` returned a bare `false` with the message lost entirely; the
same pattern existed in the `admin.nodes.*` wrappers (`api_admin.rs`).

### Fixes

**Path uniqueness — in-process CREATE path-reservation registry** on
`NodeRepositoryImpl`, shared by both write paths. A creator reserves
`(tenant, repo, branch, workspace, path)` under an owner token *before* the
committed-storage existence check and releases only after its write is durable (or
on abort). Reserve-then-check guarantees the loser either gets an immediate
`Conflict` or sees the winner's committed row.

- `repositories/nodes/mod.rs` — registry + reserve/release API
  (poison-recovering release).
- `transaction/core.rs` — per-transaction owner token + reserved-key list; release
  on `rollback`; a `Drop` impl so abandoned transactions cannot leak reservations.
- `transaction/commit/mod.rs` — release right after the durable write.
- `transaction/context/nodes/create/core/add_node.rs` — reserve before the
  existence check (covers SQL INSERT, service create, transactional deep-create,
  and upsert-create).
- `repositories/nodes/trait_impl/crud_dispatch.rs` — reserve around
  `validate_for_create` + `add_impl` with a cancellation-safe drop guard.

Cost is one short `Mutex<HashMap>` operation per create; commits are not
serialized. Sequential semantics unchanged: create at an occupied path is a hard
`Conflict` error; recreate-after-delete remains allowed.

**Error propagation** — `api_wrapper.js`: `raisin.nodes.create/update/move` now
throw `Error(message)` when the internal result carries `{error}`;
`delete`/`updateProperty` (and the `admin.nodes.*` equivalents) also throw, with
`api_nodes.rs`/`api_admin.rs` changed to return `"true"` / `{"error": …}` JSON
instead of a message-less `false`. Success shapes are unchanged.

### Registry extension: remaining path-creating writes — CLOSED (same day, follow-up pass)

The gaps listed in the first pass (`put_node` create branch, `create_deep_node_impl`,
`copy_node_impl`/`copy_node_tree_impl`) are now **CLOSED** — every path-creating write goes
through the registry, with one reservation idiom
(`repositories/nodes/mod.rs::PathReservationGuard`, an owner token + RAII release
after the durable write) shared by all non-transactional call sites:

- **`put_node` (transactional upsert-by-id)** —
  `transaction/context/nodes/create/core/put_node.rs`: when the incoming id does
  not exist (a CREATE), it reserves the path via the transaction's owner token
  *before* the existence checks and adds an explicit read-your-writes +
  committed-storage path-occupancy check (a DIFFERENT id at the path →
  `Conflict`, mirroring `add_node`). A put that updates an EXISTING id in place
  (same path or a path move) takes the update branch and is never blocked.
  `upsert_node` (by path) needs nothing of its own: its create branch delegates
  to `add_node`, its update branch to `put_node`'s update branch.
- **`create_deep_node_impl` (repo-level deep create)** —
  `crud/create/deep_create.rs`: reserves every path it may materialize, top-down
  (ancestors before descendants — consistent acquisition order, no deadlock),
  leaf last. **Intermediate folders use a WAITING reserve**
  (`try_reserve_create_path_waiting`, 5s bound): two racers creating the same
  missing parent are ensure-folder *convergence*, not conflict — the loser waits
  for the winner's reservation to clear, wins it, re-checks committed storage,
  and converges on the winner's committed folder (releasing that key
  immediately). **The leaf uses the immediate reserve** — contention there is a
  genuine `Conflict`. (The *transactional* deep path `add_deep_node` /
  `upsert_deep_node` already reserves through `add_node`; a losing transaction
  gets `Conflict` rather than convergence, since it cannot converge on another
  transaction's uncommitted folder anyway.)
- **`copy_node_impl` / `copy_node_tree_impl` (direct `add_impl` callers)** —
  `queries/copy/single.rs` + `queries/copy/tree.rs`: the destination path
  (single) / destination ROOT (tree) is reserved before the occupancy check and
  held until the write is durable. Tree copy deliberately reserves only the
  root: all descendants land strictly under it in ONE atomic WriteBatch, so
  descendant paths become visible only together with the reserved root, and a
  competing parent-validated creator cannot produce a descendant path while the
  root doesn't exist — per-descendant keys would be O(tree) mutex traffic with
  no added safety. (`copy_nodes_across_branches` is out of scope by design: it
  preserves ids and *intentionally overwrites* the target branch — destination
  collision is the point, not a conflict.)

**Bonus correctness bug found by the new tests** — `transaction/context/nodes/read.rs::get_node`
iterated the NODES column family with a prefix iterator but never verified
`key.starts_with(prefix)` (unlike `get_node_by_path` and `materialize_path`), so
looking up a NONEXISTENT id deserialized the *next node in the keyspace* and
returned a wrong node. Observed effect: `put_node` with a fresh id randomly took
the UPDATE branch against an unrelated node instead of creating. Fixed with the
standard first-non-matching-key `break`.

### Known remaining gaps (documented, not fixed)

- The registry is in-process: cross-node uniqueness under replicated multi-writer
  deployments is a separate design problem (last-writer-wins by design). The
  invariant, the replication boundary and its anomalies, and three
  clustering-roadmap designs (path-partitioned ownership / reservation via the
  replication log / accept-then-heal repair) are written up in
  [`docs/design/path-uniqueness-and-replication.md`](design/path-uniqueness-and-replication.md),
  including application guidance until a cross-replica mechanism ships.
- A parentless *transactional* `add_node` writing a blind descendant path is an
  orphan-tolerated escape hatch; the registry does not attempt prefix ownership
  for it (see the tree-copy scope note above).

### Tests

`crates/raisin-rocksdb/tests/create_path_uniqueness_test.rs` (now 10 tests; all
pass, verified 5× for race stability):
- First pass (4): concurrent transactional creates, concurrent repository creates
  — both verified to FAIL with the fix disabled — sequential recreate errors,
  reservation release on rollback/drop.
- Extension (6): `concurrent_put_new_id_vs_create_same_path_yield_single_node`,
  `sequential_put_at_occupied_path_with_new_id_errors` (also asserts a same-id
  in-place put still succeeds), `concurrent_deep_creates_sharing_parents_converge`
  (both leaves win, exactly one `/shared` and `/shared/a`),
  `concurrent_deep_creates_same_leaf_conflict` (parents converge, leaf conflicts),
  `copy_into_occupied_destination_conflicts` (single + tree copy),
  `concurrent_tree_copies_same_destination_yield_single_copy`.

`crates/raisin-functions/src/runtime/quickjs/tests.rs` gained error
injection plus 3 tests: all node write errors throw in JS with the storage message,
an uncaught create error fails the function, success shapes unchanged.

---

## Summary table

| Bug family | Root cause | Fixed? | Tests |
|---|---|---|---|
| A: GROUP BY JSON keys NULL | Group-key column naming drift between plan builder and aggregate executor; no `Cast` handling in `exprs_match`/naming → Project re-evaluated the extraction against a row without `properties` | Yes | `group_by_json_extraction.rs` (2) + 4 unit tests |
| B(a): name-fragment `path LIKE` matches nothing | Prefix scan executor anchored on a nonexistent parent node instead of a string-prefix index scan | Yes | `path_like_prefix_scan.rs` |
| B(b): directory `path LIKE` includes the parent | Descendant traversal emitted its own root; residual LIKE dropped | Yes | `path_like_prefix_scan.rs` |
| B(c): cross-subtree leakage with `AND node_type` | Residual-filter combiner unconditionally dropped hierarchy predicates when another access path won | Yes | `path_like_prefix_scan.rs` |
| B(d): stale property-index entries | UPDATE never tombstoned old property-index values; index scan never re-verified fetched rows | Yes | `path_like_prefix_scan.rs` (update/delete/reopen) |
| B(d'): transient duplicate rows | Not reproducible; most likely an artifact of B(c) | Documented | churn probe (no dupes) |
| C: duplicate rows at one path | TOCTOU between path-existence check and write; conflict checker a no-op | Yes (in-process reservation registry) | `create_path_uniqueness_test.rs` (4) |
| C'': same TOCTOU on put-new-id / deep create / copy | `put_node` create branch, `create_deep_node_impl`, and `copy_node(_tree)_impl` bypassed the registry (direct `add_impl` / batch writers) | Yes (registry extended; `PathReservationGuard` + waiting reserve for intermediate-folder convergence) | `create_path_uniqueness_test.rs` (+6) |
| D: `get_node` returns a WRONG node for a nonexistent id | NODES prefix iterator missing the `key.starts_with(prefix)` guard — ran past the prefix into the next node's keys | Yes | `sequential_put_at_occupied_path_with_new_id_errors` (regression trigger) |
| C': silent create failure in function runtime | JS wrappers swallowed `{error}` results / bare `false` | Yes | quickjs `tests.rs` (3) |

### Guidance for the application layer

- No extra `index: [Property]` declarations are required for GROUP BY keys or
  `path LIKE` correctness — none of the fixed defects were index-declaration
  behavior. Property indexes remain relevant for query *performance* and for
  index-backed COUNT pushdowns.
- Function code should be prepared for `raisin.nodes.create/update/delete/…` to
  **throw** on storage errors (previously some failures were silent); a create at an
  occupied path now reliably surfaces as a Conflict error.

---

## E: Shared test-fixture failure — `NodeType not found: raisin:Folder` (re-baseline)

**Root cause.** `WorkspaceService::put` bootstraps a ROOT node typed
`raisin:Folder` inside a transaction, and the transactional create path
schema-validates every node (`add_node` step 5a → `NodeValidator` →
`NodeTypeResolver`, strict `NotFound` on a missing type). Built-in NodeType
registration does not live in the storage layer: it is seeded by a
server-level `RepositoryCreated` event handler
(`NodeTypeInitHandler` → `raisin_core::nodetype_init::init_repository_nodetypes`).
Integration-test fixtures construct tenant/repo/branch/workspace directly on
the storage layer, so the handler never runs, `raisin:Folder` is never
registered, and `WorkspaceService::put` fails during fixture setup — masking
every test in the affected binaries. (The same ordering hazard exists in
production: a client that creates a workspace immediately after creating a
repository races the async event handler.)

**Fix (harness contract restored at one shared point).**
`crates/raisin-core/src/services/workspace_service.rs`: before creating the
ROOT node for a new workspace, check whether `raisin:Folder` resolves on the
target branch; if missing, seed the built-ins via the same idempotent
`init_repository_nodetypes` path the server handler uses. No test needed the
`raisin:Folder` seeding boilerplate anymore; two tests were re-baselined for
intended contract changes:

- `integration_tests::nodetype_repository::test_nodetype_repository_isolation`
  — exact NodeType counts became baseline-relative (built-ins are now seeded);
  the repo/tenant isolation assertions are unchanged.
- `integration_tests::tree_operations::test_copy_with_duplicate_name` — an
  occupied copy destination is now `Error::Conflict` (family C'' path-collision
  contract), previously `Validation`.

### Re-baseline after the harness fix

| Target | Result after fix | Category |
|---|---|---|
| raisin-rocksdb `test_tombstone_handling` | 9/9 pass | (i) now passing |
| raisin-rocksdb `compare_put_vs_add` | 1/1 pass | (i) now passing |
| raisin-rocksdb `rls_integration_tests` | 10/10 pass | (i) now passing |
| raisin-rocksdb `apply_revision_test` | 2/3 pass | (ii) 1 real failure |
| raisin-rocksdb `apply_revision_capture_test` | 0/2 pass | (ii) 1 real + (iii) 1 auth-context fixture |
| raisin-rocksdb `integration_tests` | 76/82 pass (5 ignored) | (ii) 2 real + (iii) 4 auth-context fixture |
| raisin-rocksdb `permission_resolution_tests` | 3/5 pass | (ii) 2 real failures |
| raisin-rocksdb `two_node_replication_test` | 5/6 pass | (ii) 1 real failure |
| raisin-rocksdb `checkpoint_network_test` | 0/2 pass | (ii) 2 real failures |
| raisin-rocksdb `cluster_integration_test` | 0/4 pass | (ii) 4 real failures |
| raisin-rocksdb `cluster_move_node_test` | 2/3 pass | (ii) 1 real failure |
| raisin-core `node_service_integration` | 10/15 pass | (ii) 1 real + (iii) 4 archetype-fixture — unchanged by the fix (InMemory backend, never hit the raisin:Folder issue) |
| Regression checks: `create_path_uniqueness_test` 10/10, `branch_fork_publish_test` 14/14, raisin-core `--lib` 166/166 | all pass | no regressions |

### (ii) Genuine masked defects surfaced (to investigate, NOT fixed here)

1. **DELETE leaves relation-index entries** —
   `apply_revision_test::apply_revision_delete_removes_relations_and_translations`
   and `integration_tests::delete_operations::test_delete_removes_outgoing_relations`:
   after deleting a node, `get_outgoing_relations` still returns its 2 relations.
2. **Transactional read-by-path misses committed nodes** —
   `integration_tests::transaction::test_transaction_can_read_existing_nodes`:
   a node visible via `nodes().get_by_path()` is `NotFound` when read through a
   transaction (`ctx.get_node_by_path`) during a copy operation.
3. **Replication op-capture gaps** —
   `apply_revision_capture_test::apply_revision_captures_node_mutations` (create
   change missing from the captured ApplyRevision op);
   `two_node_replication_test::test_user_replication` (UpdateUser op never
   captured/replicated, 0 of 1 after 5s).
4. **Cluster replication loses operations** (deterministic, reproduced twice) —
   `cluster_integration_test::test_three_node_cluster` (2 of 3 ops reach node2),
   `test_partition_recovery` / `test_admin_user_priority` (0 ops replicate),
   `test_lazy_index_trigger_after_catchup` (no PropertyIndexBuild job queued
   after catch-up); `checkpoint_network_test` both tests (post-checkpoint op
   never reaches the fresh node, n of n+1);
   `cluster_move_node_test::test_move_tree_replication` (`move_node_tree`
   fails `NotFound("Node not found")` on the source it just created).
5. **Permission resolution** —
   `permission_resolution_tests::test_group_role_aggregation` (roles granted via
   group membership resolve to `[]`) and `test_role_deduplication` (returns 1
   role where 2 distinct roles are expected).
6. **InMemory list visibility** —
   `node_service_integration::test_workspace_isolation`: a node created through
   `NodeService::add_node` is invisible to `list_all()` on the in-memory backend.

### (iii) Still-failing on OTHER fixture issues (test-code updates needed)

- **Auth-context now mandatory for transactions** (RLS hardening): fixtures
  never call `set_auth_context(AuthContext::system())` —
  `integration_tests`: `transaction::test_transaction_commit`,
  `test_initial_structure_with_transaction_api`,
  `delete_operations::test_async_snapshot_creation`,
  `delete_operations::test_concurrent_fulltext_indexing`;
  `apply_revision_capture_test::apply_revision_captures_transaction_mutations`.
- **Unregistered archetypes in initial_structure test data**: children carry
  `archetype: "text/markdown"` / `"text/rust"` as free-form strings; write-path
  validation resolves archetypes and rejects unknown ones —
  `node_service_integration`: `test_initial_structure_auto_creation`,
  `test_nested_initial_structure`, and both `*_with_transaction_api` variants.

---

## Final verification — combined tree (end of day)

Full clean build of the combined working tree (registry extension + harness fix +
all of the above) with `CARGO_PROFILE_DEV_DEBUG=false`:

- `cargo build --workspace` — **OK** in 3m 12s (warnings only, no errors).
- `cargo build -p raisin-server` — **OK** (0.5s incremental on top of the
  workspace build); binary at `target/debug/raisin-server`.

| Suite | Result |
|---|---|
| raisin-rocksdb `create_path_uniqueness_test` | 10/10 pass |
| raisin-rocksdb `--lib` | 482/482 pass |
| raisin-sql-execution `--lib` | 453/453 pass |
| raisin-functions `--lib` | 141/141 pass |
| raisin-rocksdb `test_tombstone_handling` (re-baselined) | 9/9 pass |
| raisin-rocksdb `compare_put_vs_add` (re-baselined) | 1/1 pass |
| raisin-rocksdb `rls_integration_tests` (re-baselined) | 10/10 pass |
| raisin-rocksdb `branch_fork_publish_test` (spot-check) | 14/14 pass |
| raisin-rocksdb `scheduled_invocation_test` (spot-check) | 4/4 pass |
| raisin-sql-execution `path_like_prefix_scan` (spot-check) | 7/7 pass |
| raisin-sql-execution `group_by_json_extraction` (spot-check) | 2/2 pass |

The remaining failures documented in section E — categories (ii) genuine masked
defects and (iii) fixture updates — are unchanged and out of scope for this pass.

---

## Runtime binding consolidation (2026-07-13)

The QuickJS runtime now consumes the same shared binding registry
(`runtime/bindings/methods/*`) as Starlark. What moved:

- **One gateway instead of per-method host functions.** A single host fn
  `__raisin_call(method, argsJson)` (`runtime/quickjs/gateway.rs`) resolves the
  internal method name in the shared registry and runs the invoker
  (`block_in_place` + `block_on`), returning JSON with the Starlark-identical
  `{"error": true, "message": …}` envelope on `Err`. `api_wrapper.js` dispatches
  every `raisin.*` / `asAdmin().*` / tx method through it, keeping each method's
  frozen JS error convention (throw / null / [] / sentinel) in one place.
- **Deleted ~2,900 LOC of duplication**: the hand-written QuickJS hosts
  (`api_nodes.rs`, `api_misc.rs`, `api_admin.rs`, `api_transaction.rs`,
  `api_locks.rs`, `api_integrations.rs`, `api_imap.rs`, `api_resources.rs`) and
  the dead registry artifacts (`bindings/adapters/*`, `bindings/wrappers/*`,
  `bindings/macros.rs`, `bindings/methods/common.rs`). Only per-execution-state
  bindings remain runtime-local (`api_temp.rs`, `api_fetch.rs`, timers).
- **One-definition rule now enforced by tests**:
  `test_quickjs_wrapper_matches_registry` (bindings/methods/mod.rs) requires
  every wrapper `__call('name')` to exist in the registry AND every registry
  method to be either exposed or on an explicit exclusion list;
  `test_raisin_api_surface_snapshot` (quickjs/tests.rs) pins the full JS-visible
  surface (Object.keys of `raisin`, all 16 namespaces, tx, asAdmin/admin,
  `Resource`) against expected lists, so accidental surface drift fails CI.
  Registry `internal_name`s were also verified duplicate-free.
- `pdf_extractText` / `pdf_getPageCount` / `pdf_ocr` ported into the registry
  (Starlark gains them additively); the 120s HTTP timeout moved into the shared
  invoker.

Verification of the combined tree (`CARGO_PROFILE_DEV_DEBUG=false`):
`cargo build --workspace` OK; `raisin-functions` 136/136;
`raisin-rocksdb branch_fork_publish_test` 14/14 and
`scheduled_invocation_test` 4/4; `raisin-flow-runtime` 167/167 lib +
38/38 `e2e_flows`; `raisin-transport-ws` 11+1 pass.

One fix landed during verification, in the *other* workstream's uncommitted
job-deps wiring: `raisin-transport-ws` called
`state.storage.job_registry()`/`.job_data_store()` on the generic `Arc<S>`
(methods exist only on the concrete `RocksDBStorage`), breaking the whole
workspace build. Rewired both call sites (`handlers/functions.rs`,
`handlers/nodes/sql_query.rs`) to
`state.rocksdb_storage.as_ref().map(|s| s.job_registry().clone())` — the same
feature-gated pattern the HTTP transport uses; degrades to `None` on
non-RocksDB storages.
