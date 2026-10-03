# RaisinDB read/index/resolve plan

**Status (2026-10-03):** Phases 0, 1, 2, 3, 5 and 6 DONE (uncommitted). Everything else PLANNED. Phase 2 still owes its operational gate: a prod-snapshot dry run of both repairs, and a replica compound latency measurement (item 5 makes replica compound queries scan-only until Phase 8 step 3). The binding amendments from `read-index-resolve-plan-critique.md` (three critics: replication, ops/migration, history/branch) are folded into the phases below; the critique file stays as the record of why.

This document combines nine research dossiers: propindex, haschildren, readpath, translation, linkrewrite, coreindex, resolve, invariants and sota. I re-read the code wherever two dossiers disagreed.

## 0. Conflicts I resolved by reading the code

| Question | Dossiers disagreed | Verified fact | Decision |
|---|---|---|---|
| Does HTTP compute `has_children` itself, so that storage `get` can stop? | readpath said yes (via `util.rs`); haschildren said `util.rs` is dead | `raisin-transport-http/src/util.rs::populate_has_children{,_batch}` has **no callers**. HTTP and WS responses serialize whatever storage `get` returned. | **Superseded by the owner (D4).** `has_children` stays populated on all storage reads. Phase 1 made it cheap instead (a limit-1 probe), so the ~33 internal callers no longer pay O(children × revisions) and no consumer has to change. |
| Does the delete tombstone of ORDERED_CHILDREN mask the live entry? | Only haschildren flagged it | `core_tombstones.rs:76-89` keys the tombstone by `node.parent`. `definition.rs:232-243` documents `parent` as "ONLY the parent node's name". The index is keyed by parent **id**. | Confirmed bug. Fixed in Phase 2. |
| Merge PATH_INDEX tombstone value | Only coreindex flagged it | `merge/apply.rs:29`: `const INDEX_TOMBSTONE: &[u8] = b"\x00";`. Every reader compares against `b"T"`. | Confirmed bug. Fixed in Phase 2 by making readers accept both, with the data repair as cleanup only. |
| Localized lookup: full-path index (sota #7) or per-parent segment index (translation, coreindex)? | | A full-path index needs O(subtree × locales) rewrites on every move, rename or ancestor slug edit, and it would mirror about 30 PATH_INDEX write sites. | **Segment index** (one localized segment per node, per parent). The full-path index is rejected. |
| How RESOLVE should "know what is there before descending" | sota: stamp `has_refs` on the node; coreindex: probe the forward reference index; resolve: frontier walk | The resolver must decode every target anyway in order to inline it. | **No new stored state.** Compute each target's outgoing references once, at fetch time, with the one walker, and recurse only along that frontier. RLS and existence are decided per target *before* its references are collected. A path-only RLS check runs before the blob is even read, except under graph RLS. |
| Property-index cleanup: compaction filter or history_gc? | sota #5 and propindex (c): filter; propindex (d): run-collapse | A compaction filter cannot see lookahead or a causal-stability watermark. | **Run-collapse inside history_gc**, but only below a cluster-wide watermark and only after the Phase 2 repairs (see Phase 9; the earlier "lossless, pins irrelevant" claim was wrong). The lossy filter is deferred. |
| Rewriting links on plain reads | | Studio and the CLI read properties over SQL and write them back, so read-time rewriting would permanently replace internal links with public URLs. | **Rejected.** Use an explicit `REWRITE_LINKS()` only. |

## Guiding rules

These come from CLAUDE.md, project memory and the critique, and every phase follows them:

- **One writer, one reader, one walker per concern.** The largest bug class in this codebase is mirrored paths that drift apart, and several phases below exist only to collapse duplicates. Merge apply is one of those mirrored funnels (Phase 2.10).
- **Nothing derived replicates as an op, but checkpoints ship every CF.** Every new or changed index is maintained inline by the replication apply path and rebuilt locally on each node. Checkpoint bootstrap, however, is a put-merge of *every* CF from the peer, derived indexes and INDEX_STATUS included, and emits no events. So for every repair and every new CF the phase must say what checkpoint transfer does to it.
- **Rebuild leases.** Local index builds must not hold a *cluster-wide* lease: a lease held by the node that does not need the build leaves the node that does need it unbuilt. The compound build handler takes one today; Phase 2.11 scopes it per node. (The earlier rule "rebuild jobs take no lease" contradicted that existing code.)
- **New derived state ships behind a fail-closed state record**, with a row-level fallback that keeps results correct during the migration window (the spatial/compound template). State records use **tenant-first** keys that include repo, branch and workspace, and are revalidated after checkpoint ingest.
- **No change to the on-disk key format of an existing CF.** Every phase leaves existing data readable as-is. Migrations are additive (new CF, or new tombstones written at historically correct revisions).
- **New CFs ship registration-first.** A release that only declares the CF (empty, in `all_column_families`) ships one release before any writer, and the checkpoint ingestor skips and warns on unknown CFs. Downgrading below the registration release is unsupported.
- **Historical-revision writers.** Any writer that inserts below existing entries (repairs, rebuilds, out-of-order replication, `versionable=false`) must be accounted for by every reader and by Phase 9. A derived-index writer tombstones at `max(R, newest existing entry revision for that group)`, or allocates a fresh revision when the group's newest entry is above R.
- **Concurrency.** Cargo runs from the coordinator only, one crate at a time, using the narrow filters listed in the invariants dossier. Every new integration test is a module under `tests/all/`, never a new binary.

### Repair and rebuild discipline (applies to every repair, backfill and rebuild job below)

- **Reader-tolerant first.** A repair is cleanup, never the fix. Either readers tolerate the corrupt data permanently, or the repair is re-triggered by the checkpoint ingest hook (the place that already calls `invalidate_all_derived_caches`). Otherwise a later checkpoint from an unrepaired peer silently undoes it.
- **Detect from the data, not from "done" flags.** Every repair is idempotent and finds its targets by inspecting the data, so it is correct after a checkpoint ingest and after a crash.
- **Streaming and bounded.** Never the existing rebuild routine that loads a workspace into memory and commits one WriteBatch. Stream the CF, commit bounded batches (4–16 MB), persist a resumable cursor, rate-limit (or low IO priority), report progress per branch.
- **Disk precheck.** Refuse to start without headroom of at least 2× the affected CF on the data volume. Compaction afterwards runs by key range in bounded slices with a rate limiter, outside the backup window. Reclaimed space only appears after backup retention rotates, because checkpoints hard-link SSTs (prod holds ~207 GB of backups beside the data dir). The dry run reports the bytes it expects to write or reclaim.
- **Admin-triggered, never at boot, never on all nodes at once (D11).**
- **Fan-out mechanism.** Admin-triggered repairs and rebuilds use **(b) an admin endpoint that fans out to every peer** and reports per-node status; each node writes a per-node state record (`{tenant}\0{repo}\0{branch}\0repair_state\0{repair}\0{node_id}`) so the console can show which nodes have not repaired. Job dedup is per-process and there is no replicated "rebuild requested" op, so without this the result is N-1 nodes that stay broken and report nothing. Rebuilds that are *caused by a replicated config change* (Phase 12 default language and `localized_slug.property_pattern`) are instead enqueued by the `UpdateRepository` apply arm on every node, local and remote, i.e. on the "local + remote" side of `node_handlers.rs:133/162`.

---

## Phase 0 — Measurement harness — DONE

**Goal.** Give every later phase a reproducible before/after number and an operation-count assertion, so improvements are measured rather than reasoned.

**Files.**
- New `crates/raisin-sql-execution/tests/all/index_read_bench.rs`, added with `mod index_read_bench;` in `tests/all/main.rs`, all `#[ignore]`. It reuses `bootstrap()` and `engine()` from `throughput_sql.rs:102/210`.
- New `crates/raisin-rocksdb/tests/all/perf_counters.rs`, a helper that wraps a closure in RocksDB `PerfContext` (`set_perf_stats(EnableCount)`) and returns `{seek, next, get, internal_key_skipped, block_cache_hit/miss, bytes_written}`.
- `crates/raisin-rocksdb/benches/rocksdb_benchmarks.rs`: add `get_by_id/{revs}`, `get_at_old_revision/{revs}`, `get_by_path/{children}`, `append_children/{N}` (exposes the O(N²) parent lookup) and `update_unchanged_props/{P}` (write amplification: count PROPERTY_INDEX keys per update).
- A counting `Storage` wrapper in a test helper that counts `nodes().get*` calls (RESOLVE fetch counts).
- `QueryEngine` phase timers (parse / analyze / plan / execute) behind a `RAISIN_SQL_PHASE_TIMING` env flag in `raisin-sql-execution/src/engine/mod.rs`. This is the only product-code touch; it is a no-op when the flag is unset.

**Scenarios.** Results are written as JSON lines to `target/bench/*.jsonl`, so before and after can be diffed.
- **A.** Point lookup by an unchanged text property (`properties->>'slug'::String = $1`) on a node edited N ∈ {1, 10, 100, 1000} times, among M = 10k distractor nodes.
- **B.** The same query at `__revision = early`.
- **C.** `SELECT RESOLVE(properties, 2)` over a header/footer/settings fixture (about 25 references, 3 shared), as one statement and as N = 50 rows that share targets.
- **D.** `SELECT *` with embeddings configured.
- **E.** `get_by_path` on a parent with 1k children, each edited 10 times.
- **F.** `CHILD_OF` / `DESCENDANT_OF` over 3 levels × 20 children.
- **G.** Append 1k children under one parent, through the repository path and the transaction path.
- **H.** Update one property on a node with P = 30 properties; count index puts and bytes.
- **I.** Baseline for localized lookup: `properties->>'url_fr'::String = $1`, plus a per-segment walk.
- **J.** Parse/plan share of a sub-millisecond point query (decides whether Phase 14 is worth doing).

**Index advisor (report-only).** One `debug!` counter per predicate shape that falls back to a table scan, aggregated in the harness output. This is the only adaptive-indexing work justified now; see §6.

**Migration / replication / branch.** None.
**Rollback.** Delete the files.
**Expected gain.** None directly. It is the baseline for everything else.
**Risk.** None. Tests are ignored and run on demand.

## Phase 0b — MVCC index oracle (the guard for Phases 5–9) — PLANNED

**Goal.** One property test that turns any index or MVCC regression into a shrunk, minimal failing history.

**Files.**
- New `crates/raisin-sql-execution/tests/all/mvcc_index_oracle.rs` (proptest is already a dev-dependency there).

**Design.**
- Generate random histories of 20–60 operations: create, update (string, number, reference, nested Element/Composite), delete (with and without cascade), move subtree, rename, reorder, copy tree, set translation, fork followed by edits, **merge with resolutions (KeepOurs / KeepTheirs)**, **`versionable=false` updates**, and restore.
- Record HEAD after every commit.
- **The oracle is an independent in-memory reference model** driven by the generated operation sequence. It tracks tree and order from its own op log (parent **ids** and explicit sibling order), never from `cf::NODES`, NODE_PATH or ORDERED_CHILDREN. (The earlier design reconstructed the tree from `Node.parent`, which is the parent NAME and therefore ambiguous; read paths from NODE_PATH, which Phase 10 puts under test; and computed `__order` from ORDERED_CHILDREN, checking editorial order against itself. A NODES-derived oracle is also blind to bugs that distort NODES history itself, such as merge replaying source NODES at original revisions.)
- Compare **both** `cf::NODES` (decoded directly through `storage.db()`) and every index template against the model. Never call a repository read API from the model side.
- Assert the invariant `Node.order_key == ORDERED_CHILDREN label` for every live child.
- `versionable=false` is modelled explicitly as "no history": those nodes are excluded from historical-revision assertions rather than making them pass vacuously. Merge conflict detection does not see `versionable=false` edits (no revision is minted); the model encodes that as documented behaviour.
- For every recorded revision and every template, assert that the index-backed result equals the model. Templates: `get_by_path`, `list_by_parent` + `has_children`, `CHILD_OF ORDER BY __order`, `DESCENDANT_OF ORDER BY __tree_order` (keyset pages), property equality, `node_type =`, `REFERENCES`, compound `CHILD_OF + ORDER BY`, `ORDER BY created_at/updated_at ASC/DESC LIMIT k`, timestamp ranges, `COUNT(*)`, `RESOLVE(properties, 2)`, **`get_last_order_label` and the next appended label** (Phase 7 prerequisite), and the localized lookup (Phase 12).
- Stage 2: after `run_history_gc` with a random cutoff plus a tag pin, results at every retained revision are unchanged.
- Stage 3: replay into a second storage and check parity at HEAD **and** at every revision. Stage 3 uses **shuffled, interleaved two-origin oplogs with permuted apply order**, not a single-origin in-order replay, because `apply_replicated_upsert` has no newer-than-stored guard and its own comment names out-of-order catch-up.
- Case count comes from `PROPTEST_CASES` (default 16).

**Expected outcome.** Several templates **fail today** (see Phases 2 and 5). They go on a **named expected-failure list** whose expected (wrong) results are asserted, and the build **fails if a listed template unexpectedly passes**, so the list is pruned the day its fixing phase lands. `#[should_panic]` is not used: it passes on any panic and would mask new regressions in exactly those templates.

**Risk.** Low (test only). The reference model makes it 4–5 days rather than 2–3.

---

## Phase 1 — Stop paying for work nobody reads (has_children, logging, revision seeks) — DONE

**Goal.** Remove the largest per-row and per-lookup waste with no change to on-disk state or to results.

**Implemented (uncommitted, 2026-10-03).**
1. **has_children stays on all reads (owner decision, supersedes the earlier "None on the storage path" text and D4).** Instead of moving population to the API boundary, the computation was made cheap. `util.rs`'s dead helpers and the per-caller `populate` flags are therefore not needed.
2. **has_children is a limit-1 existence probe** (`ordering/child_probe.rs::probe_has_children`). It takes the caller's path, reads ORDERED_CHILDREN with limit 1 bounded by `max_revision`, and confirms each hit against NODES liveness. After **64 dead hits** it answers `true` and warns once, so a parent with many stale entries (Studio per-run job nodes) costs O(64), not O(deleted children), until the Phase 2.1 repair runs. `delete_without_cascade` (`cascade/single.rs`) uses the same probe.
3. **Seek instead of walk for time travel.** `get_revision_at_or_before` (`crud/helpers.rs`), `materialize_path` and the `lookup.rs` time-travel loop seek to `prefix ++ encode_descending(target)`. The seek helper **advances past a key whose revision fails to parse** (today's skip-and-continue at `crud/helpers.rs:141-153`), never returns None on it. The two `materialize_path` copies are merged into one.
4. **Log levels.** Hot-path `info!` sites became `debug!`/`trace!`; the `first_bytes` allocation moved into the error branch.
5. **Embedding column.** `warn!` once per query; "newest ≤ max_revision" instead of exact revision equality; one seek per `(embedder_hash, kind)` partition instead of the whole-workspace v2 fallback.

**Remaining items (critique, not yet done).**
6. **Moved-child check behind a stale entry.** NODES liveness alone cannot detect a child that is live but was MOVED to another parent behind a stale ORDERED_CHILDREN entry. Confirm with NODE_PATH (or a head decode) that the child's parent at ≤ rev equals the probed parent, and pass the service's `max_revision` through `GetOptions` / `ListOptions::for_api` so `rev/{n}` API reads are bounded. Test: `has_children_false_when_only_child_moved_behind_stale_entry`.
7. **package_builder's has_children consumer.** `jobs/handlers/package_create_from_selection/package_builder.rs:109` picks the folder layout (`.node.yaml`) vs the file layout from `node.has_children`. It is correct while D4 keeps has_children populated, but it is a behavioural consumer of a derived field: switch it to an explicit probe (or collected-set membership) so a future change to D4 cannot silently change the archive layout. Grep every `.has_children` read, including the functions callbacks (`raisin-functions/.../callbacks/nodes.rs` uses `storage.nodes()` directly) and MCP surfaces, and record which keep it.
8. **Golden package-export test** for a non-folder node with children: `package_export_non_folder_with_children_golden`.
9. **Path rule** for the merged `materialize_path`: adopt the Phase 10 rule (newer of NODE_PATH and embedded blob path, by revision) when Phase 10 lands. Until then the put_node-rename stale path described in Phase 10 is present on this reader too.

**Files.** `ordering/child_probe.rs`, `queries/lookup.rs`, `queries/listing.rs`, `crud/batch/has_children.rs`, `trait_impl/has_children_ext.rs`, `crud/helpers.rs`, `crud/read/path_materialization.rs`, `transaction/context/nodes/read.rs`, `crud/cascade/single.rs`, the scan executors, `embedding_storage/storage.rs`, `node_to_row/embedding.rs`; remaining: `package_builder.rs`.

**Migration.** None.

**Replication / branch / copy / move.** None; these are read-side changes. The probe reads the same CF that fork, merge and replication already maintain.

**Tests.**
- `has_children_consistency_test` in raisin-rocksdb `tests/all`: false→true→false on first create and last delete, at HEAD and every revision in between; move-out and move-in; copy and cross-branch copy; fork and merge; replication apply; ignores a client-supplied `Some(true)`; root `/`; unchanged after history GC; **dead-hit cap answers true and warns once**.
- A seek-vs-walk unit test over random HLCs, including encodings with `0x00` and `0xFF` bytes **and an unparseable-revision key the seek must advance past**.
- Remaining: items 6–8 above.
- Existing guards: `ordered_children_keyset_test`, `child_listing_keyset_test`, `integration_tests::{node_repository,mvcc_time_travel}`, `editorial_order_tests`.

**Rollback.** Revert the commit. No data is involved.

**Expected gain** (to be confirmed by harness E, A, C and G).
- `get_by_path` drops from about 7 seeks plus an O(children × revisions) scan to about 3 seeks plus an O(1) probe.
- An SQL index-scan row loses one node reload, one BRANCHES read and one child-range scan.
- Child appends through the repository path go from O(N²) to O(N).
- Time-travel reads go from O(newer revisions) to O(1) seeks.

**Risk.** Low. No API-visible change; the probe's dead-hit cap trades an occasional false `true` (warned) for bounded cost.

---

## Phase 2 — Correctness bugs the research proved, plus their repairs — DONE

**Goal.** Fix verified defects that would otherwise corrupt every number taken afterwards. Each item is small and independently revertible. **Order inside the phase: writer fixes (items 1, 2, 7, 10) first, then the repairs.** Repairing while a funnel still produces the corruption means the repair has to run forever.

**Prerequisites.** Prod-snapshot dry run of every repair (see Operational test gates).

1. **ORDERED_CHILDREN delete tombstone keyed by parent NAME** (`tombstones/core_tombstones.rs:76-89`).
   - Change the tombstoner signature to take an explicit `parent_index_id: Option<&str>`. The replicated delete passes its `parent_id` (`crdt_ops.rs:457-461`) **unconditionally**; today the shared tombstoner is fed `node.parent` (the NAME) whenever the peer's node carries one, so the fix would never reach replicas.
   - Callers with no id resolve it: `/` for root children, otherwise a PATH_INDEX point read of `node.parent_path()`.
   - Tombstone the **stored** label from `get_order_label_for_child`, falling back to `node.order_key`.
   - Fix the "insurance" lookup in `delete/tombstone.rs:68-70` the same way. `cross_branch/prune.rs:153-157` is the reference implementation.
   - **Repair** (streaming, per the discipline above; not an addition to `rebuild_order_indexes`, which loads a workspace into memory):
     - *NODES-tombstone pass.* Iterate **every** NODES tombstone of a child, not only the latest, so a node deleted and re-created more than once gets a tombstone per delete revision. Resolve the parent id and the label from the node version **just before each delete revision** (PATH_INDEX and the ordered entry at that revision), never at HEAD: in a cascade delete the parent is gone, or its path now belongs to another node. Write the tombstone at the child's delete revision on the exact stored `(label, child)` key, which can differ from the deleted node's `order_key` (legacy drift, merge verbatim copies).
     - *ORDERED_CHILDREN pass.* Iterate ORDERED_CHILDREN per parent and verify each `(label, child)` against the child's NODES state at HEAD. For a live entry whose child has no live NODES version ≤ HEAD, tombstone at the child's delete revision if one is known, otherwise at the GC cutoff, and report it. *As built:* the repair has no reliable GC cutoff to read, so a child with no NODES version left is tombstoned at the successor of the entry's own revision (`successor(entry_rev)`) — just after the entry, which hides it at every revision where the child had no readable version anyway — and counted as `without_delete_revision`. A known delete revision older than the entry is likewise replaced by `successor(entry_rev)`, so the tombstone always lands above the entry it masks. This catches entries whose NODES versions `history_gc` with `drop_orphan_tombstones` (`history_gc/mod.rs:226-235`) already removed.
     - Covers root parents, which the existing job skips (`async_indexing/rebuild.rs:843-845`).
     - Runs per branch, including forks taken after the delete that copied the untombstoned entry.
     - Reader-tolerant already: the Phase 1 probe confirms liveness, so the repair is cleanup. Checkpoint: an ingest from an unrepaired peer re-imports stale entries; the repair is data-detected and re-run from the ingest hook.
2. **Merge PATH_INDEX tombstone** `b"\x00"` → `b"T"` (`branches/merge/apply.rs:29`).
   - **The fix is in the readers:** one shared `is_tombstone(value)` that treats both `b"T"` and a one-byte `b"\x00"` as a tombstone, used by **every** PATH_INDEX reader permanently. A checkpoint from an unrepaired or older peer re-imports `\x00` values, and nothing would re-run a one-shot repair.
   - **Repair (cleanup only):** rewrite exact one-byte `\x00` values in PATH_INDEX (and any CF that merge wrote to) as `T` at the same key. Idempotent, streaming.
3. **Bulk descendants at a past revision** (`bulk_descendants.rs:127-130` vs `:163-171`): apply the `max_revision` filter before recording tombstones.
4. **`find_child_id_by_name`** (`ordering/queries.rs:254-288`): add per-(label, child) newest-wins deduplication, the same way the label lookup does.
5. **Replica compound fail-closed.** Until Phase 8 step 3 writes compound and unique entries on replicas, the apply path marks a workspace's compound state `NotBuilt` whenever it applies an upsert to a workspace that has compound indexes.
   - Find the indexes via `list_for_workspace` (the state records), **never** via a NodeType read in the apply path (the deadlock rule).
   - The marker carries a **monotonic generation** (the applying revision). The rebuild CASes `Ready` only if no newer marker exists; otherwise a rebuild's final `Ready` overwrites a `NotBuilt` set by an upsert it never saw.
   - **Consequence, stated plainly:** every replicated upsert after a rebuild flips the replica back to `NotBuilt`, so **replica compound queries are scan-only until Phase 8 step 3**. Measure replica query latency under this item. Preferred alternative: pull Phase 8 step 3 forward and ship it with this phase rather than a permanent replica scan fallback.
   - Merge apply writes no compound entries either, so the same marker applies there until Phase 2.10.
   - Checkpoint: state records arrive from the peer; revalidate after ingest (a peer's `Ready` says nothing about this node's apply history).
6. **Tenant wipe of INDEX_STATUS** (`storage/tenant_wipe.rs`): reuse `repo_purge`'s `MixedLayout` logic so that `spatial_index\0{t}`, `compound_index\0{t}` and `prop_index\0{t}` records are deleted.
7. **One reference walker.** Replace `tombstones/helpers.rs:25-76`, `async_indexing/helpers.rs:327-365`, `repositories/reference_index.rs:22-60` and the dead `transaction/types.rs:17-50` with `walk_references`. That fixes Composite references never being tombstoned on delete, and Element/Composite references being dropped by REBUILD. Add the Composite arm to the write-side `collect_references` (`transaction/.../create/references.rs:121-169`), with a log-only mode for dangling path-form references in the first release.
8. **Replication `load_latest_node`** (`replication/.../db_lookups.rs:13-56`). **Lands before Phase 7 and before Phase 10.**
   - *Seek:* a workspace-scoped seek of `{t}\0{r}\0{b}\0{ws}\0nodes\0{id}\0` instead of scanning the whole branch NODES CF.
   - *Explicit vs defaulted workspace:* the workspace is always present today because it is defaulted, so a "slow path for callers with no workspace" would never run, and a node whose op carries no workspace (older peers in a mixed-version cluster) would get `old_node = None`, leaving stale path/property/spatial entries untombstoned. Distinguish an explicit workspace from a defaulted one; on a scoped miss, fall back to the branch scan with a `warn!` counter (a miss cannot be told apart from a wrong workspace). Remove the fallback only after a release in which the counter stayed at zero cluster-wide.
   - *Decode:* `load_latest_node` decodes NODES blobs as `Node`; repository-written blobs are `StorageNode`, which has no `path`, so `path` silently becomes `""`. Route it through the shared `deserialize_node_with_path` (materializing NODE_PATH at the baseline's revision). That function becomes the **only** baseline reader for Phase 7, Phase 8 and Phase 12's `sync_node`.
   - *Bounded variant:* add `load_node_before(id, rev)` (newest version strictly below `rev`), the baseline Phase 7 and Phase 12 need.
9. **Copy, promote and deep-create write spatial entries.** `crud/batch/write_batch.rs::add_node_to_batch_with_parent_id` calls the shared spatial writer. Also, `spatial_reconcile.rs:62-67` uses `indexed_geometry_paths` instead of top-level iteration.
10. **Merge apply is a mirrored write funnel; route it through the shared writers** (new; lands before any repair). `write_resolved_node` and `write_resolved_deletion` (`merge/apply.rs:90-230`) write no ORDERED_CHILDREN entries or tombstones, no NODE_PATH tombstone on delete, and no UNIQUE, COMPOUND or translation entries, and do not call `tombstones::tombstone_node`. A merge-created or merge-moved node is missing from CHILD_OF; a merge-deleted node stays listed and keeps `has_children` true.
    - Route merge deletion through `tombstones::tombstone_node`.
    - Route merge upsert and move through the shared ordered-children and NODE_PATH writers.
    - At the merge revision M, a resolution does a **full put of every index, ORDERED_CHILDREN included**, and tombstones the **union** of the values indexed by base, target head and source head. Today's `add_stale_*` calls tombstone only target-old vs new, which leaves source-side values live (an existing bug).
    - **Known gap — translations.** Merge apply writes no TRANSLATION_DATA / TRANSLATION_INDEX entries: the overlays the copy brings over stay as each side left them, with no union tombstone at M. Resolving this needs the revision-correct translation reader and the translation write substrate of Phase 11, and lands there. COMPOUND entries are likewise not written at M; item 5's `NotBuilt` marker covers them until Phase 8 step 3.
11. **Compound build lease** (new). The compound build handler takes a cluster-wide lease and skips with "being built elsewhere", so a replica marked `NotBuilt` by item 5 may never rebuild locally. Remove the cluster lease, or scope its key per node (include the node id).
12. **Cross-branch stage leaks removed values** (new, existing bug). `stage_cross_branch_entry` (`stage.rs:114-145,163-185`) is an upsert with `old_dst` but tombstones only compound and unique entries from it; PROPERTY_INDEX and REFERENCE_INDEX values removed on the source stay live on the publish branch. Tombstone them from `old_dst`. (Phase 7 then passes `old_dst` as the baseline.)

**Files.** As listed above, plus a new streaming repair module `management/async_indexing/repair/{cursor,ordered_children,path_tombstone}.rs` with the admin fan-out endpoint and per-node state record, registered in the job registry.

**Migration.** Two idempotent, streaming repair jobs (order tombstones, merge `\x00` cleanup) and a REFERENCE REBUILD for Composite and Element references. All admin-triggered, fanned out per the discipline above, each node running its own build under a per-node (not cluster) lease.

**Replication / branch.** Repairs run per node and per branch. Delete-path fixes reach the replicated delete through the explicit `parent_index_id`. Checkpoint: readers tolerate both corruptions (items 1 and 2), so an ingest from an unrepaired peer cannot make results wrong, only re-dirty the data; the ingest hook re-enqueues the data-detected repairs.

**Tests to add.**
- `has_children_after_last_nested_child_deleted`: false at HEAD, true at the pre-delete revision, and `list_ordered_children_page` empty.
- `replicated_delete_tombstones_by_parent_id`.
- Order repair: `order_repair_cascade_delete`, `order_repair_delete_then_recreate`, `order_repair_after_gc_dropped_nodes_versions`, `order_repair_fork_after_delete`.
- `merge_resolution_vacates_path`: `get_node_id_by_path` returns None after a merge delete or rename resolution.
- `path_reader_treats_nul_byte_as_tombstone`.
- `merge_resolution_keeps_child_listing_consistent` (create, move and delete).
- `merge_keep_ours_source_only_value_not_matched`.
- `bulk_descendants_at_past_revision_includes_later_deleted`.
- `find_child_by_name_ignores_renamed_entry`.
- `replica_compound_query_falls_back_until_rebuilt`, `compound_ready_cas_loses_to_newer_marker`.
- `compound_build_runs_on_replica_while_origin_holds_lease`.
- `tenant_wipe_clears_index_state`.
- `reference_walkers_agree` (writer, delete tombstoner and REBUILD produce the same `(path, ref)` set for Element and Composite input).
- `copied_geometry_is_spatially_indexed`.
- `promote_removed_property_not_matched_on_publish_branch`.
- `replicated_upsert_finds_previous_version_without_branch_scan` (a perf-counter assertion), `replicated_upsert_without_workspace_falls_back_and_counts`, `load_latest_node_materializes_path_for_storage_node_blob`.
- `repair_streams_within_memory_bound` over a large synthetic workspace; `repair_resumes_after_crash` (see Operational test gates).

**Rollback.** Per item. The repair jobs only add tombstones at correct revisions (or rewrite `\x00` to `T`, which readers already treat identically), so they never need undoing.

**Expected gain.** Correctness. Replication apply cost per op goes from O(branch NODES) to O(1) seeks.

**Risk.** Medium (was "low"): item 10 changes the merge write path, and item 5 makes replica compound queries scan-only until Phase 8 step 3. Item 7 can surface previously silent dangling path references in Composite blocks, which is why it starts in log-only mode.

---

## Phase 3 — RESOLVE: correct, unified, and "know before descending" — DONE

This is the core of the user's request. It only touches code; batching is Phase 4.

**Goal.** Make `RESOLVE()` the single resolution engine. It must be RLS-safe, read at a single snapshot revision, deduplicate across all rows of a statement, and decide for each target whether it exists and is readable before it walks into that target.

**Implemented (uncommitted, 2026-10-03).**
1. **Dead resolver copies deleted.** `reference_resolver.rs` is split into `reference_resolver/{mod, walk, frontier, fetch, memo, budget}.rs`.
2. **One snapshot.** The resolver takes `at: HLC`, which is `ctx.max_revision` or else HEAD read **once per statement**. Node reads, the translation overlay **and graph RLS** (`filter_node_with_graph(..., &self.snapshot)`, `fetch.rs:113-120`) all use it, so `rls_filter_node_graph`'s `HLC::now` default no longer tears the snapshot. Path references are HEAD-bounded, matching id references.
3. **RLS always enforced on targets** (decision D1; no legacy flag). Every target passes through the RLS filter. A denied, missing or hidden-in-locale target produces identical output: the bare reference. The path-only pre-check (`can_read_in_path` before the blob is read) is **disabled when `auth.uses_graph_rls()`** (`fetch.rs:76`), because a path-only check cannot see RELATES.
4. **Per-statement memo** keyed `(ws, locator, snapshot, locale, fields_sig)`, held on `ExecutionContext`, never shared across statements.
5. **Frontier BFS.** Level N+1 consists only of references found in nodes first fetched at level N, minus anything already in the memo. One final substitution pass with depth tracking.
6. **Translation presence probe.** One iterator pass per node over its overlays; one resolver per statement.
7. **Budget.** 5,000 distinct targets, 50k inlined occurrences, 32 MB per statement; fails loudly.
8. **Embedding hidden from `SELECT *`** (D5): fetched only when named.

**Known limitation (critique).** Historical-translation behaviour under D2 is only trustworthy on the origin. On replicas, overlays do not replicate until Phase 11, and Phase 11's resync must preserve overlay history; until **Phase 11.1** (revision-correct overlay reader), 11.4 (overlay replication) and 11.5 (history-preserving resync) land, a locale-scoped `rev/{n}` RESOLVE can differ between origin and replica. Do not advertise historical translations in RESOLVE before then.

**Remaining.** Verify, then fix if confirmed: `RESOLVE(...)->>'k'` fails because `JsonExtractText` is not in the async dispatch (`async_eval.rs:44-74`).

**Files.** `raisin-core/src/services/reference_resolver/*`, `raisin-core/src/services/translation_resolver/mod.rs`, `raisin-core/src/services/rls_filter/mod.rs`, `raisin-sql-execution/src/physical_plan/eval/async_eval.rs`, `executor/context.rs`, `scan_executors/helpers.rs`.

**Migration.** None.

**Replication / branch.** None; this is read-only. On a fork, RESOLVE reads the fork.

**Tests** (in `raisin-sql-execution/tests/all/resolve_json_tests.rs` and `resolve_rls_tests.rs`):
- `resolve_respects_rls_on_referenced_nodes`, `resolve_literal_jsonb_ref_cannot_read_unauthorized_node`.
- `resolve_graph_rls_target_denied` (RELATES rule denies a target; the pre-check must not let the blob through, and the snapshot must be the statement's).
- `resolve_at_revision_inlines_historical_target`.
- `resolve_same_path_ref_in_two_workspaces_does_not_collide`.
- `resolve_cycle_and_shared_dag_output_unchanged` (golden output).
- `resolve_hidden_in_locale_is_bare_ref`.
- `resolve_budget_errors_loudly`.
- `resolve_json_extract_works` (remaining).
- A fetch-count assertion through the counting wrapper: 50 rows sharing 3 targets perform 3 fetches.
- Existing guards: `translation_roundtrip`, `resolve_inlines_nested_and_shared_references`, `resolve_fields_trims_inlined_nodes`.

**Rollback.** Revert. There is deliberately no legacy-semantics flag: RLS on targets was a security hole, not a behaviour preference.

**Expected gain** (harness C). A target shared by N rows is fetched once instead of N times; translation cost per node drops from 1 + |chain| seeks to 1; about 5× fewer storage ops on the chrome query.

**Risk.** Medium, because two behaviour changes are deliberate: RLS is enforced and historical targets are returned.

## Phase 4 — Batched snapshot reads (`get_many_for_read`), shared by RESOLVE and scans — PLANNED

**Goal.** Turn per-id awaits into one blocking batch per level or chunk.

**Prerequisites.** **Phase 10 lands before or together with this phase**, because the path rule below depends on Phase 10's backfill; adopting a NODE_PATH-only rule first returns stale paths for put_node-written nodes.

**Changes.**
- New trait method `NodeRepository::get_many_for_read(branch_scope, items: &[(ws, Locator{Id|Path})], snapshot, ReadOpts{properties: Load|Skip|Only(&[..]), has_children})`. The trait default loops `get`, which keeps the deprecated memory backend correct.
- RocksDB implementation in the new file `crud/read/batch_get.rs`. Inside **one `spawn_blocking`**:
  - **one RocksDB snapshot held for the whole statement**, alongside the single HEAD read, not one per chunk. A per-chunk snapshot can see a `versionable=false` in-place overwrite at R ≤ HEAD in one chunk and not the next; the HLC bound does not protect in-place rewrites;
  - sort the items;
  - use one raw iterator each on PATH_INDEX, NODES and NODE_PATH, seeking per item to the newest entry ≤ snapshot;
  - honour tombstones through the shared `is_tombstone` (Phase 2.2);
  - **path rule** (shared with Phase 1's `materialize_path`): take the newer of (NODE_PATH newest ≤ snapshot) and (the embedded blob path at the blob's revision), comparing revisions. NODE_PATH wins only when its revision is ≥ the blob's, which covers ancestor moves. (The earlier "read NODE_PATH at the snapshot, never at the blob's revision" rule returned the stale path for a repo-created node later renamed through put_node, because a full Node blob also parses as StorageNode and NODE_PATH then wins.)
  - decode with a projected decode when RLS needs no property conditions.
- The RESOLVE frontier calls it once per level, across a chunk of rows. `project.rs` and `batch_execution/project/mod.rs` buffer 64 rows when a projection contains RESOLVE or REWRITE_LINKS; the chunk size is 1 under `LIMIT 1` or a point lookup.
- `PropertyIndexScan`, `CompoundScan` and `ReferenceScan` fetch rows in chunks of 256 through the same primitive. One locale clone per row is removed from `property_index_scan.rs:167`, and `node_to_row` moves properties instead of cloning them (`fields.rs:162-167`).

**Files.** `raisin-storage/src/traits/node/mod.rs`, `node_operations.rs`, `raisin-rocksdb/.../trait_impl/mod.rs`, new `crud/read/batch_get.rs`, `storage_node.rs` (projected decode), `physical_plan/project.rs`, `batch_execution/project/mod.rs`, the scan executors, `node_to_row/{mod,fields}.rs`, `executor/context.rs` (statement snapshot).

**Migration.** None.

**Replication / branch.** None; this is read-only.

**Tests to add.**
- `batch_get_honours_tombstones_above_head_and_legacy_blobs`.
- `batch_get_equals_get_for_random_ids_and_revisions`, extended into the 0b oracle.
- `move_then_descendant_of_returns_new_path` (if it fails, fix `scanning/mod.rs:412-413`).
- `repo_create_then_put_node_rename_reads_new_path_at_r2` (shared with Phase 10).
- `statement_snapshot_spans_chunks_with_versionable_false_overwrite`.
- A perf assertion: RESOLVE seeks ≤ 2·distinct + c.

**Rollback.** A config flag `sql.batched_fetch=false` falls back to the per-row path for one release.

**Expected gain.** Iterator and superversion setup is amortized, and tokio workers are no longer blocked on cold reads. More than 2× on harness C/A at N = 50 rows on top of Phase 3 (to be measured).

**Risk.** Medium (a new storage primitive, and a statement-long snapshot pins memtables/SSTs for long statements; the RESOLVE budget bounds that). The mitigation is the equivalence property test.

---

## Phase 5 — Tree readers: know before recursing — DONE

**Goal.** Deep readers stop scanning ORDERED_CHILDREN for nodes the bulk set already shows to be leaves.

**Changes.**
- `deep/nested.rs` and `deep/array.rs` build `children_by_parent` once from `get_descendants_bulk_impl`.
  - A node with 0 children is a leaf: `has_children = false` and no scan.
  - A node with 1 child needs no ordering scan.
  - A node with 2 or more children gets an editorial-order scan (never sort by `order_key`, because of legacy drift).
- Fix `array.rs`'s double scan (`:99` and `:114`).
- `flat.rs` prunes ids with no descendants.
- Apply the same treatment to `count_scan`'s per-node child traversal and `NodeService::list_all`.
- At the `max_depth` boundary, use the Phase 1 probe.

**Files.** `repositories/nodes/queries/deep/{nested,array,flat}.rs`, `count_scan.rs`, `node_service` list_all.

**Migration / replication.** None.

**Tests to add.** Deep nested and array output at HEAD and at a past revision is unchanged (golden tests); `subtree_document_order_test` and `editorial_order_tests` must still pass, so DESCENDANT_OF stays pre-order.

**Rollback.** Revert.

**Expected gain** (harness F). Eliminates one iterator per leaf, which is the majority of nodes in content trees.

**Risk.** Low.

---

## Phase 6 — One revision-bounded PROPERTY_INDEX reader (correctness + speed) — DONE

**Goal.** Replace the five reader loops in `property_index/query.rs`, `scan.rs` and `listing.rs` with one primitive.

**Prerequisites.** Prod-snapshot dry run of the orphan detector (item below) before readers switch.

**The primitive.**
- Seek to `value_prefix ++ encode_desc(at)`, where `at` is `ctx.max_revision`, or else the branch HEAD (which also excludes entries stranded above HEAD).
- Decide per `(value, node_id)`: the first entry seen is the newest ≤ `at`. A live entry is a match; `T` is not.
- Stop early under LIMIT.
- Remove the node-wide cross-value tombstone set.

**Keeping orphans masked (critique).** Removing the cross-value tombstone set exposes orphan live entries left by historically buggy writers (the replica writer missing membership entries, merge apply, the old in-place writer). Custom JSON properties stay masked by the residual filter, but pseudo-properties (`node_type`, `__name`, `created_at`/`updated_at`) are exempt from it, so their orphans would become phantom rows at HEAD and in COUNT. Therefore:
- **Residual pseudo-property re-check stays** on the new reader: each candidate row's pseudo-property is re-checked against the decoded node until the per-workspace PROPERTY_INDEX state record says a rebuild or verify has reported clean. Only then may the reader drop the re-check for that workspace.
- **Report-only orphan detector** (`management/async_indexing/property_orphan_report.rs`): compares the HEAD index against node blobs for pseudo-properties and reports counts per workspace. Run it on a prod snapshot before switching readers. It writes nothing.

**Bugs this fixes** (each gets a failing-first test):
- (a) `ORDER BY updated_at ASC` drops updated nodes.
- (b) `created_at`/`updated_at` range scans. Use one encoder for bound and key (8-byte big-endian micros, `index_keys.rs:86`). Parse the value by its fixed width instead of stopping at `0x00`. Shadow older live rows.
- (c) `list_by_type` returns nodes whose type changed; re-check `node_type`.
- (d) A `__revision = N` query with a property or pseudo-property equality misses or invents rows.
- `COUNT` pushdown (`count_scan.rs:150`) uses the same primitive and the same residual re-check.

**Files.** `raisin-storage` `PropertyIndexRepository` trait (gains `at: &HLC`; memory backend stubbed), `raisin-rocksdb/.../property_index/{query,scan,helpers}.rs`, `queries/listing.rs`, executors `property_index_scan.rs`, `property_range_scan.rs`, `property_order_scan/index_order.rs`, `count_scan.rs`, planner `scan_planning/build_scan.rs` (bound encoding at `:696-704`; keep the custom JSON residual), new `property_orphan_report.rs`.

**Migration.** None. The key format is unchanged, and existing bloated data reads correctly. Orphans are masked by the residual filter (custom JSON) and the residual pseudo-property re-check.

**Replication / branch.** None. Fork-copied history reads the same way. Checkpoint: the per-workspace "clean" state record is revalidated after ingest, since ingest can import a peer's orphans.

**Tests to add.**
- `order_by_updated_at_asc_includes_updated_nodes`.
- `updated_at_range_lower_and_upper_bounds`.
- `list_by_type_after_type_change`.
- `property_eq_at_historical_revision`, `name_eq_at_historical_revision`, `count_at_historical_revision`.
- `pseudo_property_orphan_not_returned_at_head_or_counted`.
- `orphan_report_counts_pseudo_property_orphans`.
- Remove the matching templates from the 0b expected-failure list.
- Existing guards: `dml_index_and_predicate_maintenance`, `limit_pushdown_tests`, `pagination_navigation_tests`, `references_compose_tests`.

**Rollback.** Revert. No data is involved.

**Expected gain** (harness A/B). Lookup of an unchanged value on a node edited N times goes from O(N) to O(1) per node. Early exit under LIMIT. Four correctness bugs closed. The pseudo-property re-check costs one decode per candidate until a workspace is certified clean.

**Risk.** Medium, because the trait signature change touches six executors. It is guarded by the oracle and the residual re-check.

## Phase 7 — One delta writer: skip unchanged index writes (the PostgreSQL HOT idea) — PLANNED

**Prerequisites.**
- Phase 6 and Phase 0b (including the interleaved two-origin stage 3).
- **Phase 2.8** (bounded, decode-correct baseline reader) and **Phase 2.10** (merge through shared writers).
- **`get_last_order_label` rewrite** (item 6 below) before this phase touches ORDERED_CHILDREN.
- **A full PROPERTY_INDEX rebuild on every node** (via the fan-out endpoint) before `index.skip_unchanged` is enabled anywhere, because the replica writer historically omitted IS_A/HAS_MIXIN membership and skip-unchanged would never rewrite it. Alternative: the delta writer does a full put whenever the per-node state record says "built by pre-Phase-7 writer".

**Goal.** Replace the three mirrored property-index writers with one function, and stop re-putting unchanged entries in every index that shares the pattern.

**Changes.**
1. New `indexing/property_delta.rs::write_property_index_delta(batch, cfs, ctx, baseline: Baseline, new: &Node, rev)`.
   - It emits tombstones for removed or changed `(name, value, tag)` triples, and live puts **only** for added or changed ones, pseudo-properties and IS_A/HAS_MIXIN membership included.
   - Create and a tag flip behave as today.
   - It replaces `transaction/.../create/indexing.rs:54-216`, `crud/indexing/property_indexes.rs` and `replication/application/index_writers.rs:19-108`. This also fixes the replica writer that omits membership entries.
2. **The baseline is the newest version strictly below the write revision, read bounded by `rev`, never "latest".** `Baseline` is one of:
   - `Predecessor(node)` — from `load_node_before(id, rev)` (Phase 2.8). Skip is allowed **only** when `rev` is strictly greater than the baseline's revision *and* no version newer than `rev` exists.
   - `Full` — a newer version already exists (out-of-order or LWW-applied replication), or the per-node state says "pre-Phase-7 writer". Do a full put at `rev`, emitting tombstones relative to the predecessor and leaving the successor's entries alone. Otherwise a delta against the latest version writes live puts at `rev` that nothing later tombstones (phantom HEAD matches), or writes nothing at `rev` and time travel shows the old value.
   - `NoPrior` — legal **only** when the target id provably has no live version on the target branch.
   - The replication apply path **keeps full puts** (`Full` unconditionally) until oracle stage 3 with permuted two-origin oplogs proves the delta there.
3. **Callers and their baselines** (the write-funnel matrix):
   - `put_node.rs:423-444` — bounded predecessor. **put_node is also the real restore funnel**: single-node RESTORE goes through `NodeService::restore_version` → put_node (`node_service/versioning.rs:227-264`). `jobs/handlers/restore_tree.rs` only delegates to an executor callback that the server wires as `None` (`main.rs:954-958`, `init_system/mod.rs:58-60`), so RESTORE TREE currently fails; if it is wired later it must go through put_node or this writer, never a hand-rolled one.
   - `crud/update.rs:218-233` — bounded predecessor.
   - Replication applicator (`crdt_ops.rs:206-219`, `index_writers.rs`) — `Full` until proven, then bounded predecessor.
   - **Merge resolution (`merge/apply.rs:135,268`) never uses the delta or skip writer.** At M it does a full put of every index, ORDERED_CHILDREN included, and tombstones the union of base, target-head and source-head values (Phase 2.10). Merge replays the source's index entries at their ORIGINAL revisions after writing M (`resolution.rs:173-209`, then `copy_branch_indexes` at `:388-392`); a skipped KeepOurs leaves the target's live entry at its creation revision under the source's later tombstone, so the node vanishes from `slug = A` at HEAD, and the target loses its order label.
   - **Cross-branch stage passes `old_dst` as the baseline** (`stage.rs:114-145,163-185`). It is an upsert, not a create; the earlier "copy and stage go through create (old = None)" claim was wrong for stage. Copy (`copy/{tree,single}.rs`) is a create with `NoPrior`.
4. **UNIQUE:** tombstone and re-put only the properties whose value hash changed (`put_node.rs:474-489`).
5. **REFERENCE:** skip `(path, target)` pairs present in both old and new (`reference_indexes.rs:55-96`).
6. **`get_last_order_label` and the fallback label scan** are rewritten first: `(label, child)` newest-wins, tombstones honoured, returning the lexicographically max **live** label. Today the fallback picks the label with the highest revision seen and skips tombstones without recording the label, so an older live entry of a deleted child counts again; skip-unchanged and run-collapse both shift those maxima, and on a metadata-cache miss the next append can mint a label that sorts before or collides with a sibling.
7. **ORDERED_CHILDREN** (`transaction/.../create/ordering.rs:245-294`, the repository `batch/ordered_children.rs`):
   - take the label from `existing.order_key`, verified by a **`(parent, label)` prefix seek filtered by child id** (an exact-key point read is impossible: the key carries the entry's write revision too), and fall back to the sibling scan on a miss;
   - skip the re-put when parent, label and name are all unchanged (the value stores the name, so a rename still rewrites).
8. **`versionable=false`.** The earlier claim "the reused-revision tombstone overwrites the original key, which is fine" holds only when the superseded entry sits at exactly R. Any index fed by a second revision stream (Phase 12 slugs from a translation overlay at `r_t > R`, any repair or rebuild entry above R) is not masked by a tombstone at R. So every derived-index writer in Phases 7, 8 and 12 writes its tombstone at `max(R, newest existing entry revision for that group)`, or allocates a fresh revision when the group's newest entry is above R. Document that `versionable=false` nodes are excluded from merge conflict detection (no revision is minted, so `RevisionMeta.changed_nodes` never sees them; `conflict/detection.rs`, `resolution.rs:414-460`), and both branches' writes land on the same R0 keys.
9. **Verify job:** `management/async_indexing/verify_property_index.rs` samples nodes and checks that their HEAD entries exist, then enqueues the existing rebuild for any misses. Streaming and fanned out like every repair. It replaces the self-healing that every update provided by accident, but sampling alone does not close pre-existing holes, which is why the full rebuild is a prerequisite.

**Migration.** None on disk. Precondition: the per-node full rebuild above.

**Replication.** The apply path writes indexes through the same function, with `Full` baselines until proven. Derived state is still rebuilt locally. Vault-before-index ordering is unchanged, because the delta writer runs after vaulting.

**Branch / copy / move / restore.** Merge: full put at M (item 3). Stage: `old_dst` baseline. Copy: `NoPrior`. Restore: put_node with a bounded predecessor. On a move, the ordered entry for the new parent is a change and is written. A reorder changes the label and is written.

**Tests to add.**
- `unchanged_update_writes_no_entries_for_unchanged_props` (perf-counter count).
- `is_a_and_has_mixin_on_replica`.
- `time_travel_between_two_writes_with_skip`.
- `replicated_op_older_than_head_does_not_leave_live_entries`.
- Oracle stage 3 with shuffled, interleaved two-origin oplogs and permuted apply order.
- `merge_keep_ours_after_skip_unchanged_property_still_matches`.
- `merge_keep_ours_preserves_target_order_label`.
- `merge_keep_ours_source_only_value_not_matched`.
- `promote_removed_property_not_matched_on_publish_branch`.
- `unique_violation_still_detected_after_unchanged_update`.
- `reference_removed_is_tombstoned_unchanged_is_not_rewritten`.
- `reorder_and_rename_still_rewrite_ordered_entry`.
- `last_order_label_ignores_deleted_child_and_returns_max_live` (also a 0b template).
- `versionable_false_update_with_skip`, `versionable_false_with_merge`.
- `gc_then_restore` through both SQL RESTORE and `restore_version`, asserting that the index state after the restore equals the index state of the restored revision.
- Existing guards: `index_parity_test`, `merge_resolution_applies_test`, `reorder_persists_order_key_test`, `apply_revision*`.

**Rollback.** Config flag `index.skip_unchanged=false` re-enables full re-puts, which is always safe because old behaviour produced a superset.

**Expected gain** (harness H).
- PROPERTY_INDEX puts per typical update drop from about P + 7 + members to 2–4 (`__updated_at` / `__updated_by` plus whatever changed) on the origin. Replicas keep full puts until stage 3 proves the delta.
- Similar reductions for UNIQUE, REFERENCE and ORDERED_CHILDREN.
- Fork cost and GC work shrink accordingly. This directly serves the 5k writes/sec goal.

**Risk.** Medium-high. An incorrect baseline loses or invents entries (mitigated by the bounded baseline, `Full` on any doubt, and oracle stage 3). Self-healing is lost (mitigated by the prerequisite rebuild and the verify job).

## Phase 8 — COMPOUND: reader first, then derived tombstones, then replica writes — PLANNED

1. **Reader:** newest-per-`(tuple, node)` shadowing. Today the reader relies on the writer overwriting keys in place.
2. **Writer:** on UPDATE, derive the old tuple from the bounded baseline (Phase 2.8's reader) with the shared `extract_compound_column_value`, write `T` at `max(new revision, newest existing entry revision for that group)` (Phase 7 item 8), and skip unchanged tuples. This removes the workspace-wide prefix scan and the in-place overwrite in `tombstones/index_tombstones.rs:417-456`. If a NodeType's compound definition changed between old and new, fall back to the scan for that update only.
3. **Replica and merge:** write compound and unique entries on replicated upserts and on merge apply. Definitions are resolved **off** the apply hot path and cached; the cache is invalidated on `Event::Schema` and registered with `derived_cache_registry`, so there is no NodeType read inside the batch (the deadlock rule). **Cold-cache rule:** when a definition is unresolved at apply time, mark `NotBuilt` (with the Phase 2.5 generation) and enqueue a local build; never skip silently. Then remove Phase 2's `NotBuilt`-on-every-upsert marker. **Prefer pulling this step forward into Phase 2** over shipping a long-lived replica scan fallback.

**Migration.** History destroyed by the old in-place overwrite cannot be recovered, and HEAD stays correct. A compound rebuild re-derives HEAD (per-node lease, Phase 2.11). Do not run Phase 9's run-collapse on COMPOUND before step 2.

**Tests to add.** `compound_time_travel_after_update`, `compound_update_cost_independent_of_workspace_size` (perf counter), `replica_compound_query_correct_without_rebuild`, `replica_cold_definition_cache_marks_not_built`, `move_tree_compound_reindex_test` (existing), `branch_fork_index_copies_test` (existing).

**Rollback.** Per step. Step 1 is safe on its own.

**Expected gain.** Removes an O(workspace) read from every update of a node type that has compound indexes. Replicas serve compound queries from the index.

**Risk.** Medium. The order must be reader, then writer.

## Phase 9 — Run-collapse GC (the migration for existing bloat) — PLANNED

**Goal.** Shrink existing redundant index history without changing any answer at any revision.

**Prerequisites (hard).**
- **Every Phase 2 repair for that CF and branch has completed**, recorded in the per-node repair state record; collapse refuses to run on a branch where a repair is pending. The Phase 10 backfill likewise, for NODE_PATH-dependent groups.
- **A cluster-wide causal-stability watermark** exists: the minimum HLC every peer has applied and acknowledged.
- Phase 7 item 6 (`get_last_order_label` rewrite) before ORDERED_CHILDREN is collapsed.
- Phase 8 step 2 before COMPOUND is collapsed.
- Prod-snapshot dry run (Operational test gates).

**Why the earlier "lossless; pins irrelevant" claim was wrong.** Collapse assumes nothing will ever be inserted below existing entries. Out-of-order replication, the Phase 2 repairs (tombstones at historical delete revisions) and the Phase 12 rebuild all do exactly that. Concrete case: a child is deleted at `rdel` with a mis-keyed (missing) tombstone, then re-created at r3 with the same label and name. Collapse sees live(r1), live(r3) as identical and deletes r3; the repair then inserts T(rdel), and the re-created child disappears at HEAD. A collapse decision is also invalidated by any concurrent historical-revision insert between its read and its delete.

**Changes.**
- New history_gc mode `collapse_runs` (`management/history_gc/{mod,layout}.rs`). Within each existing GC group (PROPERTY_INDEX `(tag, name, value, node)`, and the analogous groups for REFERENCE, ORDERED_CHILDREN and UNIQUE), walk from newest to oldest. Delete a version whose next-older version has an identical state (live with the same bytes, or `T` after `T`), keeping the **oldest** entry of each run.
- **Only entries strictly below the causal-stability watermark** are eligible, never "per node at will".
- Takes a **per-(branch, CF) exclusion** against repair, rebuild and merge copy for the duration of each slice.
- Streams per value prefix, keeping a per-node "last state" map, so hot values over 2M entries are no longer skipped. Bounded batches, resumable cursor, rate limiter, per the repair discipline.
- **Disk:** RocksDB deletes add markers, so disk grows until compaction. Free-space precheck of at least 2× the CF size; compaction by key range in bounded slices with a rate limiter, outside the backup window. Reclaimed space only appears after backup retention rotates (checkpoints hard-link SSTs). The dry run reports the bytes it expects to reclaim.
- **Merge-base pin:** pin merge revisions in `history_gc/mod.rs:368-376` (the divergence base can be a merge revision; see invariants dossier). This applies to retention GC.

**Migration.** Admin job, fanned out per node, run per branch. Excludes COMPOUND until Phase 8 step 2.

**Replication.** Local only; derived data. The watermark is the replication-facing part.

**Checkpoint.** A checkpoint from an uncollapsed peer re-imports redundant entries; harmless (a superset), collapsed again on the next run.

**Fork.** Shrinks the cost of future forks. Forks copied earlier hold their own copies.

**Tests to add** (in `history_gc_test`):
- `collapse_preserves_every_index_read_at_every_revision` (oracle stage 2);
- `repair_then_collapse_equals_collapse_then_repair`;
- `collapse_refuses_while_repair_pending`;
- `collapse_ignores_entries_above_watermark`;
- `gc_keeps_reordered_child_hidden_in_old_group`;
- `gc_keeps_merge_base_after_second_merge`;
- `gc_then_cross_branch_copy_at_pinned_revision`;
- `gc_then_restore_to_retained_revision`;
- `gc_on_fork_independent`;
- `collapse_idempotent`;
- `collapse_disk_precheck_refuses_without_headroom`.

**Rollback.** Disable the mode in config. Entries deleted below the watermark after all repairs were redundant.

**Expected gain.** Proportional to past edit counts. Expect PROPERTY_INDEX size to drop by roughly the average number of edits per node, for unchanged properties. Measured on a production snapshot copy with the dry run; the space shows up only after backup retention rotates.

**Risk.** Medium-high. It deletes data. Mitigated by the watermark, the repair dependency, the exclusion, the dry run and oracle stage 2.

## Phase 10 — Transaction write path writes NODE_PATH + StorageNode through one writer — PLANNED

**Prerequisites.** Phase 2.8 (decode-correct baseline reader). **Lands before or together with Phase 4.** The read-path half (item 1) ships **one release before** the write-path half.

**Goal.** The SQL/WS write path (`put_node`/`add_node`) stores a full Node blob with its path (`create/storage.rs:54`) and never writes NODE_PATH, so every read of such a node decodes twice. A node created via the repository (NODE_PATH p1 at r1) and then renamed via put_node (blob path p2 at r2) reads back p1, because `deserialize_node_with_path` parses the full Node as StorageNode and NODE_PATH wins (`path_materialization.rs:138-150`).

**Changes.**
1. **Read rule first (release N).** `materialize_path` and Phase 4's `batch_get` take the newer of (NODE_PATH newest ≤ snapshot) and (the embedded blob path at the blob's revision), by revision. This fixes the stale path immediately and makes a later downgrade safe: a downgraded binary's put_node writes full-Node blobs and no NODE_PATH, and this rule still reads the blob's path.
2. Write the test first: `repo_created_then_put_node_rename_reads_new_path` (at HEAD and at r2).
3. **Writer (release N+1).** Route both layers through one `write_node_record(batch, …)` (the existing `crud/indexing/mod.rs:80-98`), placed next to `write_path_index` at `put_node.rs:393`.
4. **Backfill** (streaming, admin-triggered, fanned out): for every full-blob revision whose embedded path differs from NODE_PATH newest ≤ that revision, write NODE_PATH **at that revision**. The earlier "nodes with no NODE_PATH entry, at their latest revision, no history rewritten" scope missed exactly the affected population and left time travel to r2 wrong forever. Idempotent and data-detected.

**Files.** `transaction/context/nodes/create/{storage.rs, core/put_node.rs, core/add_node.rs}`, `crud/indexing/mod.rs`, `path_materialization.rs`, `crud/read/batch_get.rs`, new `management/async_indexing/repair/node_path_backfill.rs`.

**Migration.** Backfill as above. The reader keeps its legacy full-Node fallback forever.

**Replication.** Origin now matches replicas, which already write NODE_PATH. Checkpoint: NODE_PATH arrives from the peer; the read rule tolerates a missing or older entry, and the backfill is data-detected and re-run from the ingest hook.

**Tests to add.** The stale-path test (HEAD and r2), `put_node_writes_node_path`, a round-trip of a legacy full-Node blob, `node_path_backfill_writes_at_each_divergent_revision`, and **`downgrade_rename_through_old_put_node_reads_new_path`** (open with the old writer after the new one has run, rename through the old put_node, read back).

**Rollback.** The writer (release N+1) can be reverted to release N safely because release N's read rule prefers the newer embedded path. Downgrading below release N after the writer has run is unsupported (stale NODE_PATH would win again).

**Expected gain.** One fewer full decode per read of every SQL/WS-written node.

**Risk.** Medium (write path).

---

## Phase 11 — Translation substrate (prerequisite for Phase 12) — PLANNED

**Changes.**
1. **Revision-correct overlay reads.** `translations/{nodes,blocks}.rs` share one reader for "newest ≤ rev, `T` means absent", and the batch reader gains the `T` check (`queries.rs:115`). This is a correctness fix for time-travel reads with a locale, and the prerequisite for Phase 3's historical-translation behaviour.
2. **Atomic writes.** The repository `store_translation` writes **one WriteBatch** (`nodes.rs:102-130`). The TRANSLATION_INDEX value is unified with the transaction writer's.
3. **One key builder.** Every translation key comes from `repositories/translations/keys.rs` (`pub(crate)`), replacing about 15 `format!` copies.
4. **Overlay replication.**
   - New op `UpsertTranslationOverlay { workspace, node_id, locale, overlay | Hidden | Deleted, revision }` in `raisin-replication/src/operation/op_type.rs`, plus an applicator arm in `applicator/mod.rs`.
   - **Release 1 also adds an `Unknown` catch-all OpType variant** so that future ops degrade to skip-and-warn (or are buffered) instead of stalling the applier. The OpType enum has no catch-all today, so an older binary cannot decode a new op.
   - Two-release rollout: release 1 *applies* both the new and the legacy op, and still *emits* the legacy op. Release 2 emits the new op, gated by `RAISIN_REPLICATION_EMIT_TRANSLATION_V2`, the same pattern as envelope v2. The flag must not be set until every node, and every persisted oplog, is on release 1.
   - Hidden is captured as Hidden, not as Delete.
5. **Convergence of existing replicas.** An admin "resync translations" job, fanned out per the discipline above.
   - It **emits the legacy op while `RAISIN_REPLICATION_EMIT_TRANSLATION_V2` is unset.**
   - It **re-emits the full overlay history**: every TRANSLATION_DATA version and its tombstones at their original revisions. Re-emitting only the current overlay falsifies replica history (today's translation stamped at an old revision, or a change that never happened at a new revision).
   - Where full history is unavailable, record a per-repo `translation_history_complete_from` revision on the replica. Below it, a locale-scoped time-travel read **fails loudly** (or falls back to the default language with an explicit marker), never serves a fabricated version.

**Tests to add.**
- `overlay_read_at_revision`;
- `batch_overlay_read_with_tombstone`;
- `translation_write_is_atomic` (fault injection between puts is not possible, so assert a single batch);
- `translation_replicates_to_peer`, `hidden_replicates_as_hidden`;
- `resync_translations_preserves_overlay_history`;
- `locale_time_travel_below_history_floor_fails_loudly`;
- `unknown_optype_is_skipped_not_stalled` (mixed-version, see Operational test gates);
- `default_language_change_test` (existing);
- `cross_branch_copy_carries_translation_overlay` (existing).

**Rollback.** Release 1 is fully backward compatible. **Once the V2 flag is set, downgrading below release 1 is unsupported**: persisted oplog entries become undecodable to the older binary.

**Expected gain.** Correctness: replicas serve translated content, and time travel shows historical translations.

**Risk.** Medium. It changes the replication protocol, which is why it is gated.

## Phase 12 — Core LOCALIZED_NAME_INDEX (built like the internal derived indexes) — PLANNED

No functions, triggers or workflow engine are involved. It is maintained inline in the write batch at every funnel and rebuilt by a native job.

**Prerequisites.**
- Phase 11, Phase 2.8 (baseline reader for `sync_node`), Phase 7 item 8 (tombstone at `max(R, …)`).
- **Phase 12.0 — CF registration release.** One release **before** any writer: declare `localized_name_index` as an empty CF in `all_column_families`, and make the checkpoint ingestor skip and warn on unknown CFs. Alternatively that release opens every CF from `DB::list_cf` generically. An older binary cannot open a DB containing an unknown CF and cannot ingest a checkpoint carrying one. **Downgrading below the registration release is unsupported**; downgrading to it from the writer release is supported.
- Prod-snapshot dry run of the rebuild.

### Shape

New CF `localized_name_index`, branch-scoped, with two key families:
- forward `{t}\0{r}\0{b}\0{ws}\0lname\0{locale}\0{parent_id}\0{slug}\0{~rev}` → `node_id | T` (parent_id is `/` at the root, the ORDERED_CHILDREN convention);
- reverse `{t}\0{r}\0{b}\0{ws}\0lname_of\0{node_id}\0{locale}\0{~rev}` → `(parent_id, slug) | T`.

The reverse key lets any writer tombstone the previous forward key without reading the old overlay. Keys have fixed-tail parsers and are never split on `\0`.

### Slug sources

One selector, `localized_slugs(node, overlays_by_locale, repo_cfg) -> Vec<(locale, slug)>`, placed next to the walker in `indexing/`. It is used by every writer and by the rebuild, and it has two sources:
- (a) a reserved overlay pointer `/__slug` in TRANSLATION_DATA, which gets history, fork, copy and delete for free;
- (b) `RepositoryConfig.localized_slug.property_pattern`, e.g. `url_{locale}`, which serves today's `url_fr` data without rewriting any node.

Selection is structural (RepositoryConfig is already loaded and replicated), with no NodeType resolution on the write path.

### Lookup

`/a/b/c` in locale L walks the segments with **one seek per segment** and stops at the first missing one. Per segment:
1. Try each locale in `get_fallback_chain(L)`.
2. If none matches, try the base name through PATH_INDEX, but only if that node has no slug of its own in L (one reverse probe).
3. Otherwise answer `canonical_localized_path`, so the caller can issue a 301.

RLS is applied to the resolved node. Missing, forbidden and hidden-in-locale all return the same 404.

Moving or renaming an ancestor writes nothing here. Moving a node writes one forward and one reverse row per locale for the moved root only.

### Uniqueness

Per `(branch, ws, parent, locale)` over effective names, checked by a forward-key probe in the same batch, the way UNIQUE_INDEX does it.
- Enforcement is **off** per repository until a clean rebuild has reported zero collisions.
- Until then, collisions resolve deterministically (lowest `created_at`, then id) and are listed in the admin UI.
- Concurrent replicated claims: the highest `~rev` wins on every replica.

### Registration checklist

- `lib.rs` (`mod cf`, `all_column_families`) — **in the 12.0 release**;
- checkpoint ingestor: skip-and-warn on unknown CFs — **in the 12.0 release**;
- `branches/cf_registry.rs` (Copied, `RevisionLocator::Tail`);
- `storage/tenant_wipe.rs` (`TENANT_PREFIXED_CFS`; not test-enforced, so add a test that it is listed);
- `storage/repo_purge.rs`;
- `history_gc/layout.rs`;
- `tombstones/mod.rs`;
- optional `prefix_transform.rs` extractor, only after the Phase 15 iterator audit.

### Write funnels

All of them call one `localized_name::sync_node(batch, scope, node_id, parent_id, new_selector_output, rev)`:
- **create/update:** `put_node.rs`, `add_node.rs`, `crud/create/add.rs`, `crud/update.rs`, `batch/write_batch.rs`, `queries/property.rs` (pattern source), `deep_create.rs`;
- **translation:** `transaction/context/translations/write.rs`, `repositories/translations/nodes.rs`;
- **move:** both `move_tree.rs` (root only);
- **delete:** `transaction/.../delete.rs`, `crud/delete/tombstone.rs`, `cascade/{single,tree}.rs`, `tombstones/index_tombstones.rs`;
- **copy:** `copy/{tree,single}.rs`, `cross_branch/{stage,translations,prune}.rs`;
- **merge:** `merge/apply.rs` (full put at M, never a delta; Phase 7 item 3);
- **replication apply:** `node_operations/{create_node,move_rename}.rs`, `applicator/{crdt_ops,legacy_node_ops,move_node_ops}.rs`, plus the Phase 11 translation arm;
- **restore:** **put_node** (`NodeService::restore_version` → put_node, `node_service/versioning.rs:227-264`). `jobs/handlers/restore_tree.rs` is not a funnel: it delegates to an executor the server wires as `None` (`main.rs:954-958`), so RESTORE TREE currently fails. If it is wired later, it must call `sync_node`;
- **config change:** see Rebuild and state.

**`sync_node` semantics under out-of-order apply.** It reads the reverse rows **bounded by the incoming revision** (newest ≤ rev), not the newest row. When it applies below the latest reverse row, it also writes a tombstone for its own forward key at the **next-newer reverse row's revision**, so the newer state stays authoritative. Otherwise an older replicated slug write leaves a live forward key that no newer revision ever tombstones (a phantom slug). Tombstones follow `max(R, newest existing entry revision for that group)` for `versionable=false` nodes, because a slug from an overlay at `r_t > R` is not masked by a tombstone at R.

### Rebuild and state

- Job `RebuildLocalizedNameIndex` per `(repo, branch, ws)`, through `JobRegistry` + `JobDataStore`, streaming per the repair discipline.
- It walks pre-order and writes at each node's current revision.
- **State key** `{tenant}\0{repo}\0{branch}\0{ws}\0lname_state` (tenant-first, and including repo, branch and workspace: INDEX_STATUS is `SkippedOnPurpose` on fork, `cf_registry.rs:266-267`, so a shorter key would let a fork or publish branch alias another branch's Ready flag). It records `built_from_rev` **and a fingerprint of `(property_pattern, default_language, fallback chain)`**. Lookups fail closed on a fingerprint mismatch (the compound template).
- **Config changes flip the state to `NotBuilt` in the same write batch as the config change**, on the origin and in the replication apply arm for `UpdateRepository`, and enqueue the rebuild from both (the "local + remote" side of `node_handlers.rs:133/162`). A default-language or `localized_slug.property_pattern` change replicated from a peer otherwise leaves the state Ready and lookups serve stale slugs until the rebuild drains.
- **Per-branch rebuild required** after fork, after cross-branch publish target creation, and after a merge into a branch that is not Ready.
- The planner and the lookup are gated fail-closed: until the state is Ready with a matching fingerprint, and for any `max_revision < built_from_rev`, they **fall back to the `url_{locale}` property lookup**. That is today's behaviour, so the migration window is never empty.
- Any in-process state cache registers with `derived_cache_registry`.
- Every node runs its own rebuild under a per-node lease at most; admin-triggered rebuilds use the fan-out endpoint.
- **Checkpoint.** The state record and the CF arrive from the peer; revalidate the state after ingest (fingerprint against the local RepositoryConfig, `built_from_rev` against local history), and treat a mismatch as `NotBuilt`. Across mixed versions, exclude the CF from checkpoints or version-gate it.

### Surfaces

All of them call **one** core function, `NodeService::resolve_localized_path(ws, locale, path, at, auth)`.
- **SQL:**
  - virtual columns `__slug` and `__localized_path`, populated only when a locale is in scope and only when projected;
  - a new physical operator `LocalizedPathLookup` (in the style of `point_lookup.rs`) for `WHERE locale = 'fr' AND __localized_path = $1`;
  - a scalar `RESOLVE_PATH(workspace, locale, path) -> id`;
  - `extract_locale_predicate` accepts bound parameters (`predicates.rs:308`);
  - an optional guarded rewrite of `properties->>'url_fr'::String = $1` to the index when the pattern matches and the state is Ready.
- **HTTP:** `GET /api/repository/{repo}/{branch}/head/{ws}/by-localized-path/{locale}/{*path}`, returning the translated node plus `canonical_path`, `canonical_localized_path` and `alternates{locale: path}` for hreflang. **`alternates` is filtered by per-locale visibility and RLS**: a locale where the node is hidden, or a path the caller cannot read, is omitted, so alternates cannot leak paths. Short-circuit the default language and cache RepositoryConfig.
- **WS:** `node_get_by_localized_path { locale, path }`, plus an optional `locale` on `node_get`/`list`.
- **SDK:** `ws.nodes().getByLocalizedPath(locale, path)` and `localizedPaths(id)`.
- **Functions:** `raisin.nodes.getByLocalizedPath`.
- **Docs:** `docs/website/docs/access/sql/localized-paths.md`.

### Tests

One per row of the maintenance matrix, asserting lookup and absence after the operation:
- `lookup_at_every_revision`;
- `ancestor_move_and_rename_needs_no_rewrite`;
- `carried_by_copy_cross_branch_copy_and_fork`;
- `pruned_by_cross_branch_prune`;
- `removed_on_delete_and_replicated_delete`;
- `rebuilt_on_replica`;
- `default_language_change_rekeys`, `default_language_change_falls_back_until_rebuilt`;
- `pattern_change_replicated_from_peer_falls_back`;
- `fork_is_not_ready_until_rebuilt`;
- `hidden_locale_hides`, `localized_alternates_hide_hidden_locales`;
- `sibling_uniqueness_when_enforced`;
- `fallback_to_property_scan_until_ready`;
- `rls_404_parity`;
- `restore_version_resyncs_slug`;
- `versionable_false_with_translation_slug`;
- `out_of_order_slug_apply_leaves_no_phantom` (also in the interleaved-oplog oracle);
- `tenant_wipe_lists_cf`;
- `downgrade_to_registration_release_opens_db`.

Add the template to the 0b oracle.

### Rollback, gain, risk

**Rollback.** Turn the feature flag off and lookups use the fallback. The CF is **not** inert to older binaries: rollback is supported only down to the 12.0 registration release. Below it, the previous binary refuses to open the DB.

**Expected gain** (harness I). Localized URL resolution costs `depth × 1–2` seeks, independent of workspace size, instead of a property-index scan whose cost grows with edit history. Customers no longer need the `url_xx` hack.

**Risk.** High surface area (about 20 write sites). It is mitigated by the single `sync_node`, the per-site tests, the fail-closed fingerprinted gate and the registration-first rollout. Effort L, 2–3 weeks, plus one release of lead time for 12.0.

## Phase 13 — `REWRITE_LINKS()`: rich-text link rewriting in SQL — PLANNED

**Prerequisites.** Phases 3 and 4, which provide the shared batched, RLS-aware loader.

### Function

`REWRITE_LINKS(value [, options jsonb])` returns the same type it receives: TEXT in gives TEXT out, JSONB in gives JSONB out. It is non-deterministic.

**Accepted link forms:**
- HTML `<a href="raisin://{ws}/@{id}">`: the id form, stable across moves, preferred;
- `raisin://{ws}/{path}`: the path form, compatible with MCP;
- ProseMirror link marks carrying the existing `{"raisin:ref", "raisin:workspace"}` envelope.

All other hrefs are left byte-identical.

**Options:**
- `locale`: defaults to the row's resolution locale;
- `url`: a template over `{locale}`, `{path}`, `{rel}`, `{id}`, `{ws}`, and `{localized_path}` once Phase 12 is Ready;
- `strip`;
- `default_locale_prefix`;
- `broken`: one of `unlink | mark | keep_text`;
- `fields`;
- `keep_ref`.

### Semantics

- Missing, deleted, RLS-denied, hidden-in-locale and absent-on-this-branch targets are all "broken" and produce identical output.
- Publish awareness comes only from querying the publish branch.
- Output passes a scheme allowlist, and attributes are HTML-escaped.
- Id links resolve through the Phase 4/10 path rule at the snapshot, never through stored `raisin:path`, which goes stale after a move.

### Implementation

- New `raisin-core/src/services/link_resolver.rs` over the Phase 4 batch read, using `PropertiesMode::Skip` when RLS has no property conditions (and never the path-only skip under graph RLS, as in Phase 3).
- `raisin-sql-execution/src/physical_plan/eval/functions/links/{mod,html_scan,prosemirror,url_template}.rs`, using a minimal `<a` tokenizer (no new dependency).
- Signatures registered in `raisin-sql/src/analyzer/functions/builtins_hierarchy.rs`.
- Async dispatch in `async_eval.rs`.
- Cross-row chunking from Phase 4.
- Optional 13b: `links: Option<RichTextLinks>` on both `RichTextFieldConfig` structs (serde default), used only to select fields for `REWRITE_LINKS(properties)` through the `WalkCursor` selector. Run `cargo test -p raisin-models --lib --no-run` plus dependents.

### Tests and rollout

**Tests** (in `raisin-sql-execution/tests/all/rewrite_links_tests.rs`):
- an id link after a MOVE gets the new path;
- a path link after a move is broken;
- the locale prefix, and hidden in `fr` means broken;
- RLS-denied output equals missing output, including under graph RLS;
- deleted target;
- unpublished target is absent on the publish branch;
- `rev/{n}` uses the historical path;
- 50 rows sharing 3 targets cost 3 fetches;
- HTML edge cases: quotes, entities, uppercase `HREF`, `javascript:` untouched, malformed tags pass through;
- ProseMirror marks;
- `REWRITE_LINKS` on plain text with no links is byte-identical.

**Docs:** `docs/website/docs/access/sql/rich-text-links.md`.

**Migration.** None. It is read-only, and existing content passes through unchanged. It becomes useful once Studio's link picker writes the id form (decision D8).

**Rollback.** Remove the function.

**Expected gain.** Studio drops its client-side link resolution. One statement renders a page with its links at a fixed cost per distinct target.

**Risk.** Low.

## Phase 14 — SQL AST/plan cache (gated on harness J) — PLANNED

- **Step 1** (only if parse plus analyze exceeds about 15% of point-query latency): cache the parsed AST keyed on the template SQL **before** parameter substitution, and bind parameters as AST literals. This gives pgwire Parse/Bind real reuse.
- **Step 2** (physical plan with parameter slots): deferred. Planning depends on literal values (LIMIT pushdown, `__revision` stripping, the property literal), so a parameter-aware planner is needed first. If built, it is keyed on `(template, tenant, repo, branch, catalog epoch, availability epoch, auth shape)`, invalidated on `Event::Schema`, and registered with `derived_cache_registry`. Availability is re-checked at execution time (fail closed).

**Risk.** Medium. Step 1 is low.

## Phase 15 — Deferred, measure-gated, or rejected

- **Prefix extractors and prefix blooms** for NODES, NODE_PATH, PATH_INDEX, PROPERTY_INDEX and TRANSLATION_DATA, with an `exact_prefix_scan` helper (`prefix_same_as_start`). Only after an audit of every shorter-prefix iterator (fork copy, wipe, purge, GC, rebuild), and only if harness counters show blocks read on negative lookups. Meanwhile, drop the dead whole-key bloom on PROPERTY_INDEX (memory only; restart required, no data change).
- **Content-addressed decoded-blob cache** (key = hash of blob bytes, value = decoded StorageNode without path). Only if the post-Phase-4 harness shows decode dominating. Never a cache keyed by `(node, revision)` or by HEAD (`versionable=false`, replication below HEAD, and SST ingest all break it).
- **Lossy PROPERTY_INDEX compaction filter.** Premature; run-collapse covers the bloat. Revisit only for `__updated_at` tombstone churn, with a historical-revision planner gate and the Phase 9 watermark.
- **Materialized has_children (CHILD_PRESENCE CF).** Not recommended. It would be a 31st mirrored write path, counters do not survive merge, and a flag on the parent is not time-travel correct. Revisit only if the Phase 1 probe still profiles hot, and then only after funnelling all ORDERED_CHILDREN writes through one helper.
- **Rich-text links in the reference index** (extending `walk_references` with a memchr-gated `raisin://` parse, id form only). It needs a REFERENCE rebuild. Valuable for unpublish-impact and REFERENCES(), but not required for rewriting.
- **RESOLVE nested in CASE** (lifting async calls in the planner). Do it after Phase 4, when Studio asks for it.
- **Repository-layer translation `name_{lang}` legacy keys:** leave as they are.

## Operational test gates

These gate releases, not single phases. They cover the failure modes that unit and oracle tests cannot.

- **Downgrade open test, every release.** After the new binary has run (and written whatever it writes), open the DB with the previous release's CF list and read path. Required to pass for every release except one explicitly marked "downgrade unsupported below X" in its release notes (Phase 10 writer below its read-rule release, Phase 11 after the V2 flag, Phase 12 below 12.0). Test: `downgrade_open_previous_release` in raisin-rocksdb `tests/all`.
- **Mixed-version replication test for every new op.** An older applier receiving an unknown OpType must skip-and-warn or buffer it, never stall the applier or wedge the peer. Test: `unknown_optype_is_skipped_not_stalled` (Phase 11), and the same harness for any later op.
- **Crash-mid-repair resume.** Kill each streaming repair, rebuild and backfill at a random batch boundary, restart, and assert that the final state equals an uninterrupted run and that the cursor resumed rather than restarted. Test: `repair_resumes_after_crash` per job.
- **Checkpoint bootstrap then repair.** Bootstrap a node from an unrepaired peer's checkpoint (corrupt ORDERED_CHILDREN, `\x00` PATH_INDEX tombstones, a stale Ready state record), assert reads are already correct (reader tolerance), run the repair, and assert the data is clean and state records were revalidated. Test: `bootstrap_from_unrepaired_peer_then_repair`.
- **Prod-snapshot dry run before Phases 2, 6, 9 and 12.** On a copy of a production snapshot: run every repair, the Phase 6 orphan detector, the Phase 9 collapse and the Phase 12 rebuild in dry-run mode, and record expected writes, expected reclaimed bytes, peak memory, duration and disk headroom. A phase does not ship until its dry run is recorded in the release notes.

## 6. Adaptive / dynamic indexing: verdict

**Premature, apart from a report-only advisor.**

- Every derived index is rebuilt on **every** replica and copied on every fork.
- Every new index adds a write funnel to a codebase whose top bug class is mirrored writers that drift.
- Correctness depends on fail-closed availability gates.

Auto-creating or dropping indexes would multiply all three costs with no human in the loop. Cracking, learned indexes, DBSP/IVM, the InnoDB change buffer and migrating to RocksDB user-defined timestamps are all poor fits (sota dossier). What *is* justified is the Phase 0 advisor counter: log, per predicate shape, the queries that fell back to a scan or a row-level fallback, aggregated in the admin console. An operator then declares compound or property indexes explicitly through the existing state record. Revisit automation only when that log shows recurring, high-volume shapes that operators keep failing to index.

## 7. Decisions the product owner must make

- **D1 — RESOLVE enforces RLS on targets.** DECIDED **yes**, always enforced, no legacy flag (shipped in Phase 3). Guest Studio sites need read grants on `assets` and `tags`, or delivery functions declare `execution_context: system`.
- **D2 — RESOLVE reads targets at the query revision.** Recommend **yes**. It is a correctness fix, and `rev/{n}` pages will show historical targets. Historical *translations* are trustworthy on replicas only after Phase 11.1 and 11.5.
- **D3 — RESOLVE budget defaults** (5k targets / 50k occurrences / 32 MB, failing loudly). DECIDED **yes** (shipped).
- **D4 — `has_children` on reads.** DECIDED: **kept on all reads**, made cheap by the Phase 1 probe (supersedes the earlier "absent from internal storage reads" recommendation). Sub-decision: should the chevron respect RLS (children the caller cannot see)? Recommend: not now. Document it as existence-only.
- **D5 — `embedding` excluded from `SELECT *`** (fetched only when named). DECIDED **yes** (shipped), with a release note.
- **D6 — Skip-unchanged writers give up accidental self-healing.** Recommend **yes**, with the prerequisite full rebuild on every node and the verify job.
- **D7 — Run-collapse GC enabled by default under KEEP_ALL.** **Reversed: no.** Collapse is not lossless while historical-revision inserts are possible. It stays off by default, under KEEP_ALL and otherwise, until the causal-stability watermark exists and every Phase 2 repair has completed on that branch; then it is admin-triggered. Retention-based GC stays opt-in. Pin merge bases first.
- **D8 — Internal link grammar `raisin://{ws}/@{id}` and a Studio link-picker change.** Recommend **yes**. URL policy stays in Studio through the REWRITE_LINKS template.
- **D9 — Localized slug model.** Recommend: `/__slug` overlay plus the `url_{locale}` RepositoryConfig pattern. Per-sibling uniqueness enforced per repository only after a clean rebuild. A canonical-name URL in a locale where the node has its own slug answers with a 301 hint, not a 404.
- **D10 — Translation replication op rollout across two releases.** Recommend **yes**. Release 1 applies the new op and adds the `Unknown` OpType catch-all, release 2 emits it. Downgrade below release 1 after the flag is set is unsupported.
- **D11 — Repair jobs run automatically on upgrade, or admin-triggered?** **Admin-triggered, permanently**, with a prominent console banner and per-node status. The earlier "automatic in the next minor release" is dropped: it would make every node run streaming repairs at boot simultaneously.

## 8. Ordering summary (gain / risk)

| # | Phase | Status | Gain | Risk | Effort | Hard prerequisites |
|---|---|---|---|---|---|---|
| 0 | Harness | DONE | enabler | none | S–M | — |
| 1 | has_children / log / seek | DONE (items 6–9 open) | very high | low | M | — |
| 3 | RESOLVE correct + frontier + memo | DONE | very high (+ security) | medium (behaviour) | M | — |
| 6 | Bounded property reader + residual re-check + orphan report | IN PROGRESS | high + correctness | medium | M–L | prod-snapshot dry run (orphan report) |
| 2 | Proven bug fixes, merge funnel, compound lease, then repairs | PLANNED | correctness, replication apply O(1) | medium | M–L | writer fixes before repairs; prod-snapshot dry run |
| 0b | MVCC oracle with independent reference model | PLANNED | guard | none | L | before 7–9 |
| 5 | Tree readers know-before-recurse | PLANNED | medium | low | S–M | — |
| 10 | Path read rule (release N), NODE_PATH writer + backfill (N+1) | PLANNED | medium + correctness | medium | M | 2.8 |
| 4 | Batched snapshot reads | PLANNED | high | medium | M | 10 (before or with) |
| 7 | Delta writers (skip unchanged) | PLANNED | very high (writes) | medium-high | M–L | 6, 0b, 2.8, 2.10, `get_last_order_label` rewrite, full rebuild on every node |
| 8 | Compound | PLANNED | high (writes) | medium | M | 2.5, 2.11; step 3 ideally pulled into 2 |
| 9 | Run-collapse GC | PLANNED | high (space, fork) | medium-high | M | all Phase 2 repairs complete per CF/branch, causal-stability watermark, 7.6, 8.2; prod-snapshot dry run |
| 11 | Translation substrate + `Unknown` OpType | PLANNED | correctness | medium | M | two-release rollout |
| 13 | REWRITE_LINKS | PLANNED | feature | low | M | 3, 4 |
| 12.0 | localized_name_index CF registration + unknown-CF skip in ingest | PLANNED | enabler | low | S | one release before 12 |
| 12 | Localized name index + surfaces | PLANNED | feature + perf | high surface | L | 11, 2.8, 7.8, 12.0 shipped; prod-snapshot dry run |
| 14 | AST cache | PLANNED | measure-gated | low–medium | M | harness J |
| 15 | Deferred items | — | — | — | — | — |

Notes on the order:
- **2.8 (decode-correct, bounded `load_latest_node`) lands before Phases 7 and 10**; it is the only baseline reader for 7, 8 and 12.
- **Phase 10 lands before or together with Phase 4**, and its read rule ships one release before its writer.
- **Phase 9 hard-depends on the Phase 2 repairs** for each CF and branch, and on the watermark.
- **Phase 12's CF registration (12.0) ships one release before any writer.**
- Every repair, backfill and rebuild streams with bounded batches, a resumable cursor and a disk precheck, stays admin-triggered, and fans out through the admin endpoint with per-node state.
- Phase 13 is listed before Phase 12 because it is read-only and lower risk. Its `{localized_path}` template variable lights up once Phase 12 reaches Ready.
