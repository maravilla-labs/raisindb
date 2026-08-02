---
title: Tracking moving objects
description: How to configure a high-frequency position property — what it costs per update, and how the spatial compaction filter keeps a hot cell constant instead of growing with update count.
---

# Tracking moving objects

Tracking a vehicle that reports its position every few seconds is a genuinely
different workload from indexing static places, and the defaults are tuned for
the latter. This page states the costs plainly.

## Per-update write cost

Every position update writes **one index key per configured precision**, and
tombstones the superseded ones.

| profile | precisions | keys written | tombstones | total per update |
|---|---|---|---|---|
| default (`INDEX_PRECISIONS_DEFAULT`) | `2,4,6,7,8,9,10,11` | 8 | 8 | **16** |
| tracking | `6,8` | 2 | 2 | **4** |

Tombstones are bounded by `configured ∪ indexed` — the precisions that can
actually hold entries, read from the local index-state record. Where that record
is unavailable (no state record yet), tombstoning widens to all twelve
precisions: over-tombstoning costs writes, under-tombstoning leaves a stale
entry matching forever, and only one of those is acceptable.

## Configuring a tracking field

Use the per-property policy that already exists. No new subsystem, no flag.

```sql
ALTER SPATIAL INDEX FOR 'fleet' PROPERTY 'position' SET PRECISIONS = (8, 6);
```

Precision **count** is a write-cost knob; radius coverage is what it buys:

| precision | approximate cell | serves radii |
|---|---|---|
| 8 | ~38 m × 19 m | up to ~100 m — tight candidate set |
| 6 | ~1.2 km × 0.6 km | ~100 m to ~3 km |
| 4 | ~39 km | wide-area fleet queries |

Beyond ~10 km a precision-6 ring approaches the 1024-cell scan budget and the
planner declines the index, degrading to a correct-but-full scan with a warning.
If wide-radius fleet queries matter, add a coarse precision (`(8, 6, 4)`,
3 keys per update) rather than reverting to the eight-precision default.

`cover = centroid` (the default) is right for a tracked point; `extent` would
multiply cells per precision for no benefit on a point.

## Why a tracking workload used to degrade, and what bounds it now

The revision is part of the index key, so an update writes a *new* key rather
than overwriting the old one. A radius query prefix-iterates each scanned cell
and visits **every** key in it, so superseded revisions are read cost.

The distribution is counter-intuitive:

* At a **coarse** precision a vehicle circulating one airport stays inside the
  **same cell** across every update, so that one prefix accumulates roughly two
  entries per position update.
* At a **fine** precision the vehicle moves between cells and the entries spread
  thin.

**Coarse cells are where read cost concentrates.** Left unbounded, one vehicle at
1 update/second for 24 h is ~86,400 updates and on the order of 1.7 × 10⁵ entries
in its precision-6 prefix — a query that was single-digit milliseconds on day one
became seconds by day two.

### The compaction filter bounds it

A stateful RocksDB compaction filter on the spatial column family drops
superseded entries: the descending revision sits immediately after the geohash
in the key, so the filter can identify them. It is **on by default**.

Measured over 500 position updates of one vehicle:

| | before | after |
|---|---|---|
| precision-6 cell prefix | 501 entries | **1** |
| whole spatial CF | 4,122 entries | **8** |
| 1 km radius query | 1.50 ms | **0.21 ms** |

Those figures are with maximum reclamation (`keep_revisions = 1`). The shipped
default is `keep_revisions = 8`, `retention_secs = 3600`, which lands at 8 entries
in the same test — a hot cell becomes a **constant** rather than a function of
update count.

The **newest entry per node per cell is never droppable**, so a read at HEAD is
bit-identical with the filter on or off. Tombstones are dropped only during a
full compaction, once aged out of the retention window: partial visibility means
the filter can only ever keep too much, never drop a live entry, so pruning is
incremental and converges as levels merge.

Configure it via `RocksDBConfig::spatial_compaction`; environment overrides let
an operator widen retention or disable it on a running deployment.

:::note Historical reads
Because pruning discards old revisions, a spatial read behind the retention
window would be approximate. The planner handles this rather than letting it be
silently wrong: a `__revision`-scoped spatial predicate is routed to a full scan
instead of the index, and `EXPLAIN` names the pruning as the reason. HEAD queries
still take the index.
:::

A rebuild does **not** prune — it writes *more* tombstones. Do not schedule
periodic rebuilds as a mitigation; the filter is the mechanism.

### The per-cell scan budget

Past 250,000 entries in a single cell (`DEFAULT_SPATIAL_MAX_ENTRIES_PER_CELL`)
the index scan will not answer from a partial read. It raises a typed signal and
the executor **degrades to the fallback row scan** — slow and exact — rather than
failing the query. Fewer precisions still reduce the rate of accumulation.

## Modelling pattern

Keep the **current** position on the tracked entity, in a property whose policy
is the tracking profile:

```sql
UPDATE 'fleet' SET properties = $1::JSONB WHERE id = 'van-17';
```

Write positional **history**, if you need it, as separate append-only nodes under
a **different property name**:

```sql
INSERT INTO 'fleet' (id, path, node_type, properties)
VALUES ($1, $2, 'fleet:Ping', $3::JSONB);   -- geometry property: track_point
```

Splitting them this way is still worth it with the compaction filter on. The
filter bounds `position`, whose old revisions are genuinely superseded; an
append-only history is *not* superseded data and should not be pruned. Keeping it
under a different property name means proximity queries over `position` never
scan it, so the two workloads stop competing.
