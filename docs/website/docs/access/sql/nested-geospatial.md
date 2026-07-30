---
title: Nested geospatial
description: Address a geometry anywhere in a node's property tree — inside a section, an element or an array — with one dotted-path rule that the index and the query share.
---

# Nested geospatial

A geometry does not have to be a top-level property. RaisinDB indexes **every**
`Geometry` value it finds in a node's property tree — inside an object, inside a
section element, inside an array of elements — and each one gets its own index
namespace, addressed by its **dot path**.

## The rule, in one line

> **The string inside `properties->>'…'` IS the property segment of the index
> key.** One format, used by the writer, the tombstoner, the rebuild job and the
> query.

## Addressing

Given this node:

```json
{
  "title": "Zurich HB",
  "location": { "type": "Point", "coordinates": [8.5402, 47.3779] },
  "venue": {
    "name": "Main Hall",
    "geo": { "type": "Point", "coordinates": [8.5535, 47.3779] }
  },
  "hero": {
    "element_type": "demo:MapSection",
    "map_pin": { "type": "Point", "coordinates": [8.5668, 47.3779] }
  },
  "stops": [
    { "element_type": "demo:Stop", "geo": { "type": "Point", "coordinates": [8.673, 47.3779] } },
    { "element_type": "demo:Stop", "geo": { "type": "Point", "coordinates": [8.5468, 47.3779] } }
  ]
}
```

every geometry is addressable:

| where the geometry sits            | how you write it                     | index key segment |
|------------------------------------|--------------------------------------|-------------------|
| top level                          | `properties->>'location'`            | `location`        |
| inside an object                   | `properties->>'venue.geo'`           | `venue.geo`       |
| inside an element (a section field)| `properties->>'hero.map_pin'`        | `hero.map_pin`    |
| one element of an array            | `properties->>'stops.0.geo'`         | `stops.0.geo`     |
| every element of an array          | `properties->>'stops[].geo'`         | *(not indexed — see below)* |

```sql
-- a geometry nested in a section element
SELECT name FROM 'places'
 WHERE ST_DWITHIN(CAST(properties->>'hero.map_pin' AS GEOMETRY),
                  ST_POINT(8.5668, 47.3779), 500);

-- one element of an array
SELECT name FROM 'places'
 WHERE ST_DWITHIN(CAST(properties->>'stops.0.geo' AS GEOMETRY),
                  ST_POINT(8.673, 47.3779), 500);
```

Array indices are **zero-based**. Separator is `.` and only `.` — there is no
escaping, so a property whose *name* contains a dot is ambiguous against a nested
path and is read as the direct key. (Same limitation as the reference index.)

### `properties->>'venue.geo'` is required — a bare `venue.geo` is not a path

A bare dotted identifier parses as a **qualified column reference**
(`table = venue`, `column = geo`), not a property path. Nested geometry must
always be written as `properties->>'…'`.

## Naming a field searches only that field

Each geometry field has its own index namespace, so a query names exactly what it
searches. In the example above, a 200 m query at `location`'s position matches
the node; the same query against `venue.geo` does not, because that geometry is
1 km east.

This is the whole point of the addressing rule: there is no query shape that
searches "any geometry on the node", and none that silently changes meaning when
somebody adds a geometry field to a node type later.

## Several geometries on one node: **one row per node**

A node that matches via several of its geometries appears **exactly once**.

* `ST_DWITHIN(properties->>'stops[].geo', …)` is **true when ANY** matched
  geometry is within the radius.
* `ST_DISTANCE(properties->>'stops[].geo', …)` is the **MINIMUM** over the
  matched geometries — "how close does this node get".

Minimum, not first-found: it is the only definition that makes

```sql
ORDER BY ST_DISTANCE(properties->>'stops[].geo', ST_POINT(8.54, 47.37)) LIMIT 10
```

mean "the ten nearest nodes". Ties resolve to the lexicographically smallest
concrete path, so the answer is deterministic.

One row per node is also what keeps **keyset pagination** correct. With one row
per geometry, a cursor on distance would straddle rows of the same node and page
boundaries would both duplicate and skip rows.

`LIMIT k` therefore means *k nodes*, not *k geometries*.

### Which geometry matched: `__matched_path` and `__distance`

Two opt-in pseudo-columns say how a spatial predicate was satisfied:

| Column | Type | Meaning |
|---|---|---|
| `__distance` | double | metres from the query centre; for a wildcard, the **minimum** over the node's matched geometries |
| `__matched_path` | text | the **concrete** path that achieved it — `stops.3.geo`, never `stops[].geo` |

```sql
SELECT name, __distance, __matched_path
FROM 'places'
WHERE ST_DWITHIN(CAST(properties->>'stops[].geo' AS GEOMETRY),
                 ST_POINT(8.54, 47.37), 500);
--  name | __distance | __matched_path
--  p1   |       37.4 | stops.3.geo
```

Both are **opt-in**: name them explicitly. `SELECT *` does not expand them,
because they are NULL on any row a spatial access path did not produce.

### The `[]` wildcard is a row scan, not an index scan

`stops[].geo` is accepted, and it is correct — but it is **not index-backed**.
Each array element is indexed under its own concrete path (`stops.0.geo`,
`stops.1.geo`, …), so no single cell-ring scan can answer it. The planner routes
a wildcard to a full scan with the predicate applied per row and says so in
`EXPLAIN`:

```
TableScan: places (spatial index NOT USED for 'places'.'stops[].geo'
  ('stops[].geo' is a wildcard over an array of geometries; each element is
   indexed under its own concrete path … Name one element
   (properties->>'stops.0.geo') to use the index.))
```

There is no `properties->>'*'` and no bare `ST_DWITHIN(properties, …)`.

## Not indexed yet? Correct, but slow — never empty

A path whose index has not been built is answered by a **row scan with the
predicate retained**. Results are correct; the scan is full; a warning names the
path and the rebuild command. It is never a silent empty result, and never an
error.

Existing data becomes visible to nested queries once it has been rebuilt or
rewritten:

```sql
REBUILD SPATIAL INDEX FOR 'places';   -- no PROPERTY filter: re-walks every node
```

Run it with **no property filter** — that is what re-walks whole property trees
and discovers nested paths for the first time.

## Per-field policy, including array fields

Precision sets are configured per property path:

```sql
ALTER SPATIAL INDEX FOR 'places' PROPERTY 'venue.geo' SET PRECISIONS = (8, 6);
```

For an **array** field, configure the `[]` spelling — one declaration covers
every element:

```sql
ALTER SPATIAL INDEX FOR 'places' PROPERTY 'stops[].geo' SET PRECISIONS = (8, 6);
```

Array indices normalise to `[]` for policy lookup only (`stops.3.geo` resolves
against `stops[].geo`); the index entry keeps the concrete path. Resolution is
**exact-match**: a policy on `venue` does *not* apply to `venue.geo`.

## Limits

* At most **64** geometry paths per node are indexed. Beyond that the write logs
  a warning naming the dropped paths. Model the extras as child nodes.
* Selection is **structural**: every stored geometry is indexed, wherever it
  sits. The node type's shape drives *policy* (precisions, cover mode), not
  whether a geometry is indexed at all.
