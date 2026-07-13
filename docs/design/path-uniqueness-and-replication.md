# Path Uniqueness and Replication

Status: design note, 2026-07-12. Companion to
`docs/sql-correctness-findings-2026-07-12.md` (finding: concurrent creates at
the same path) and `docs/REPLICATION.md` (CRDT replication architecture).

## 1. The single-node invariant (guaranteed today)

On a single database process, a scoped path
`(tenant, repo, branch, workspace, path)` identifies **at most one live node**,
even under concurrent writers. This is enforced by an in-process **create
path-reservation registry** on the node repository
(`crates/raisin-rocksdb/src/repositories/nodes/mod.rs`):

- A shared `Mutex<HashMap<reservation_key, owner_token>>` holds all in-flight
  creates. The key is the scoped path (`\u{1}`-joined); the value is an opaque
  owner token from `new_path_reservation_owner()` — one token per transaction
  or per non-transactional create call, so an owner may re-reserve its own
  path idempotently while a different owner is rejected.
- **Ordering contract:** a creator must `try_reserve_create_path` FIRST, then
  perform the committed-storage existence check, then write. Reservations are
  released (owner-checked, so a stale release can never drop a newer
  reservation) only after the write is durable or the operation aborts. The
  registry therefore covers the check-then-write race window: at any moment a
  path is either unreserved (and the storage check is authoritative), or
  reserved by exactly one owner whose committed row will be visible to the
  next reserver's existence check.
- **Conflict contract:** the loser of a race gets a deterministic outcome —
  either `try_reserve_create_path` fails with a Conflict error (a concurrent
  in-flight create holds the key), or its post-reservation existence check
  sees the winner's committed row and fails with the ordinary
  "already exists" error. Never a silent duplicate.

All create-shaped write paths reserve through this registry:

| Path | Wiring |
| --- | --- |
| Transactional create (incl. deep-create, copy — each materialized child path) | `transaction/core.rs` (`reserve_create_path`, per-transaction owner, release on commit/rollback) via `transaction/context/nodes/create/core/add_node.rs` |
| Non-transactional create / put-that-creates | `repositories/nodes/validation.rs` + `crud/create/add.rs`, dispatched through `trait_impl/crud_dispatch.rs` (guard releases after the write) |

Regression coverage: `crates/raisin-rocksdb/tests/create_path_uniqueness_test.rs`
(sequential duplicates, transactional vs. non-transactional interleavings, and
multi-threaded races asserting exactly one winner).

## 2. The boundary: replication does not share the registry

The registry is **in-process state**. Under the multi-writer CRDT replication
described in `docs/REPLICATION.md` (masterless, operation-based, LWW merge
with vector-clock + timestamp + node-id tie-breaking), two cluster nodes can
each locally satisfy the invariant, yet both accept a create at the same
scoped path in the same causal window. Replication then converges by
last-writer-wins. **This is by design today**: availability is preferred over
cross-node uniqueness, and no coordination round-trip is paid per create.

### Concrete anomalies for path-keyed data

1. **Duplicate-then-converge window.** Between the two commits and
   convergence, readers on different nodes see *different* nodes (different
   ids, different properties) at the same path. Anything captured during the
   window — a reference resolved to the loser's id, an export, a cache — may
   point at a node that convergence subsequently discards.
2. **Silent loss of the loser's write.** LWW keeps one row; the other
   creator's content vanishes without an error ever having been returned to
   that client. From the application's perspective a successful create was
   later un-created.
3. **Orphaned secondary state keyed by the losing id.** If convergence keeps
   one id, everything the loser's commit produced under its id can dangle:
   property/fulltext/reverse-reference index entries, children created under
   the losing node before convergence, graph relations, emitted events, and
   version history. Index repair must treat "row replaced by a same-path,
   different-id sibling" as a delete+insert, not an update.
4. **Divergent subtree grafting.** Creates *under* the two competitors during
   the window attach children to different parent ids; after convergence one
   subtree's parent no longer exists, which is a stronger corruption than a
   single lost row.

## 3. Candidate designs for the clustering roadmap

| Design | Sketch | Trade-offs |
| --- | --- | --- |
| **Path-partitioned ownership** | Hash/range-partition the scoped-path keyspace; each partition has a single writer (lease-based home node). A create is forwarded to the path's owner, whose local registry then IS the global registry. | Strong uniqueness with no per-create consensus; but adds a forwarding hop, requires lease management + failover, and cross-partition operations (deep-create, copy, move spanning partitions) need multi-owner coordination. Unavailable for a partition during owner failover. |
| **Reservation via the replication log** | A create appends a *reserve* operation to the op-log and only acknowledges after a quorum (or the causal set of peers) confirms no competing reservation orders before it. Effectively per-path consensus piggybacked on existing replication transport. | Preserves masterless topology and reuses the OpLog; but turns create latency from local-disk into round-trip, and degenerates under partition (must choose: block creates or fall back to today's LWW). Careful not to serialize unrelated creates behind one slow peer. |
| **Convergence repair job (accept-then-heal)** | Keep optimistic local creates; a deterministic background reconciler detects same-path/different-id collisions in the merged state and repairs them: merge properties when types match, otherwise rename the loser to a deterministic sibling path (e.g. `page~2`) and emit a conflict event for the application/UI. | No write-latency cost, fully partition-tolerant; but duplicates are *visible* until repair runs, renames can break external URLs, and merge semantics are type-specific policy the database must expose (merge vs. rename vs. tombstone). Best paired with anomaly-tolerant applications (§4). |

These compose: partitioned ownership for the common case, log-ordered
reservation for cross-partition creates, and the repair job as the safety net
for partition-mode writes.

## 4. Guidance for application layers (until cluster-wide uniqueness lands)

- **Treat the node id, not the path, as the stable identity.** Store and
  resolve references by id where possible; re-resolve path-derived references
  after convergence-sensitive operations.
- **Idempotent upserts keyed by a stable id.** For "ensure this node exists"
  flows (folders, singletons, keyed job/state nodes), derive the identity from
  a stable business key and upsert: create, and on Conflict / already-exists
  re-read and update the existing row. Never `check-then-create` in
  application code — the error path IS the API.
- **Per-key inventory claims for mutual exclusion.** Where the application
  needs *at most one* of something across the whole deployment (counters,
  capacity, locks, "first writer wins" semantics), use the atomic inventory
  primitive keyed by the business key rather than encoding exclusivity in a
  path. Inventory claims are backed by a coordination store and hold across
  replicas; path creation is not.
- **Design for the duplicate window.** Consumers of path-keyed listings
  should tolerate transiently seeing two candidates for one logical key
  (pick deterministically, e.g. lowest id) and re-read after conflict events.

Single-process deployments (the common case today) are fully covered by §1
and need none of the §4 mitigations for correctness — they are cheap
insurance for a later move to multi-writer clustering.
