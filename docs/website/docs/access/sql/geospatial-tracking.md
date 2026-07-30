---
title: Tracking moving objects
description: How to configure a high-frequency position property — what it costs per update, what degrades over time, and what is not yet fixed.
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

## What degrades over time — read this before deploying

**Superseded index entries are never removed.** The revision is part of the index
key, so an update writes a *new* key rather than overwriting an old one, and
RocksDB compaction has nothing to collapse. A radius query prefix-iterates each
scanned cell and visits **every** key in it, including every superseded revision
and every tombstone.

The distribution is counter-intuitive:

* At a **coarse** precision a vehicle circulating one airport stays inside the
  **same cell** across every update, so that one prefix accumulates roughly two
  entries per position update, indefinitely.
* At a **fine** precision the vehicle moves between cells and the entries spread
  thin.

**Coarse cells are where read cost concentrates.** One vehicle at 1 update/second
for 24 h is ~86,400 updates and on the order of 1.7 × 10⁵ entries in its
precision-6 prefix; 200 vehicles share ~3 × 10⁷. A query that is single-digit
milliseconds on day one is seconds by day two.

A per-cell scan budget bounds the damage: past 250,000 entries in one cell the
scan refuses to answer from a partial read and errors, with a log line naming the
cell and the property. That is a guardrail, not a fix.

Fewer precisions reduce the **rate** of accumulation. Nothing today **bounds**
it.

### The real fix, and its status

A RocksDB compaction filter on the spatial column family can drop superseded
entries (the descending revision sits immediately after the geohash in the key,
so the filter can tell). That is the mechanism that makes this workload
sustainable, it is self-contained work, and it is **not implemented** — see
`docs/OPEN-ITEMS.md`.

A rebuild does **not** prune: it writes *more* tombstones. Do not schedule
periodic rebuilds as a mitigation.

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

Stated honestly: on its own this does **not** bound the current-position cell
prefix. What it buys is that the unbounded thing — history — lives in a property
namespace that proximity queries over `position` never scan, so the two workloads
stop competing.
