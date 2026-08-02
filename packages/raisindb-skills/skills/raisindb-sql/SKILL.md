---
name: raisindb-sql
description: "SQL syntax for querying RaisinDB workspaces: CRUD, JSONB properties, hierarchy queries, graph relations, full-text search. Use when writing queries in frontend or server-side functions."
---

# RaisinDB SQL Reference

## 1. Basics

The workspace name acts as the table name. Quote names containing colons with double quotes.

```sql
SELECT * FROM my_workspace
SELECT * FROM "raisin:access_control" WHERE node_type = 'raisin:User'
```

Every row exposes these built-in columns:

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT | Unique node ID (UUID/nanoid) |
| `path` | TEXT | Full hierarchical path (e.g. `/content/blog/post-1`) |
| `name` | TEXT | Node name (last path segment) |
| `node_type` | TEXT | NodeType identifier (e.g. `news:Article`) |
| `archetype` | TEXT | Archetype name, if set |
| `properties` | JSONB | All user-defined properties |
| `revision` | INT | Version number |
| `created_at` | TIMESTAMP | Creation time |
| `updated_at` | TIMESTAMP | Last modification time |

## 2. SELECT

Basic queries, filtering, ordering, and parameterized bind variables (`$1`, `$2`, ...):

```sql
-- Fetch a single node by path
SELECT id, path, name, node_type, archetype, properties
FROM launchpad
WHERE path = $1
LIMIT 1

-- Filter by node_type
SELECT id, path, name, properties
FROM social
WHERE node_type = 'news:Article'
ORDER BY properties ->> 'publishing_date' DESC
LIMIT 20

```

## 3. JSONB Property Access

### Text extraction with `->>` operator

Cast the **key** to `String`, not the result:

```sql
-- CORRECT: cast the key
SELECT * FROM social WHERE properties->>'status'::String = 'published'
SELECT * FROM "raisin:access_control" WHERE properties->>'email'::String = $1

-- WRONG: cast the result (causes "Cannot coerce type TEXT? to TEXT")
SELECT * FROM social WHERE (properties->>'status')::String = 'published'

-- WRONG: no cast (may return empty results)
SELECT * FROM social WHERE properties->>'status' = 'published'
```

### Boolean property access

Cast the key to `Boolean` when comparing with boolean values:

```sql
-- Filter by boolean property
SELECT * FROM workspace WHERE properties->>'featured'::Boolean = true
SELECT * FROM workspace WHERE properties->>'hide_in_nav'::Boolean != true

-- Also works: direct comparison (TEXT vs BOOLEAN auto-coerced)
SELECT * FROM workspace WHERE properties->>'featured' = true
```

### JSONB containment with `@>`

```sql
SELECT * FROM social WHERE properties @> '{"status": "published", "featured": true}'
```

### Key existence with `?`

```sql
SELECT * FROM social WHERE properties ? 'email'
```

### JSON path functions

```sql
SELECT JSON_VALUE(properties, '$.metadata.author') FROM social
SELECT * FROM social WHERE JSON_EXISTS(properties, '$.tags')
SELECT JSON_GET_INT(properties, '$.rating') FROM social
SELECT JSON_GET_BOOL(properties, '$.featured') FROM social
```

### Timestamp casting and comparison

```sql
WHERE (properties ->> 'publishing_date')::TIMESTAMP <= NOW()
ORDER BY (properties ->> 'publishing_date')::TIMESTAMP DESC
```

## 4. INSERT

`path` is required. The `name` is derived from the last path segment automatically.

```sql
-- Basic insert
INSERT INTO social (path, node_type, properties)
VALUES ($1, $2, $3::jsonb)

-- With literal JSON
INSERT INTO social (path, node_type, name, properties)
VALUES (
  '/articles/tech/my-post',
  'news:Article',
  'my-post',
  '{"title": "My Post", "status": "draft", "author": "jane@example.com"}'::jsonb
)
```

## 5. UPDATE

Use JSONB merge (`||`) to update specific properties without overwriting the rest:

```sql
-- Merge new properties into existing ones
UPDATE social
SET properties = properties || $1::jsonb
WHERE path = $2

-- Update name and properties together
UPDATE social
SET name = $1, properties = properties || $2::jsonb
WHERE path = $3

-- Replace all properties entirely
UPDATE social
SET properties = '{"title": "Replaced"}'::jsonb
WHERE path = '/articles/tech/my-post'
```

## 6. DELETE

```sql
DELETE FROM social WHERE path = $1
```

## 7. Hierarchy Functions

RaisinDB paths form a tree. Query the hierarchy without JOINs:

```sql
-- Direct children only
SELECT * FROM social WHERE CHILD_OF('/articles')

-- All descendants at any depth
SELECT * FROM social WHERE DESCENDANT_OF('/articles')

-- Descendants with max depth
SELECT * FROM social WHERE DESCENDANT_OF('/content', 2)

-- Path prefix matching
SELECT * FROM social WHERE PATH_STARTS_WITH('/blog/posts')

-- Navigate up
SELECT PARENT(path) AS parent_path FROM social WHERE path = '/a/b/c'
SELECT DEPTH(path) AS level FROM social
```

## 8. MOVE / COPY

### MOVE

Relocate a node and all descendants. Node IDs are preserved.

```sql
MOVE social SET path = $1 TO path = $2
MOVE workspace SET id='abc123' TO path='/target/parent'
MOVE workspace IN BRANCH 'feature-x' SET path='/source' TO path='/target'
```

### COPY / COPY TREE

Duplicate a node (new IDs are generated):

```sql
-- Copy single node
COPY workspace SET path='/templates/page' TO path='/content' AS 'new-page'

-- Copy entire subtree recursively
COPY TREE workspace SET path='/templates/section' TO path='/content'
```

## 9. ORDER (Sibling Reordering)

Reorder siblings within a shared parent:

```sql
ORDER social SET path = $1 ABOVE path = $2
ORDER social SET path = $1 BELOW path = $2
```

## 10. RELATE / UNRELATE (Graph Relations)

Create typed, weighted, directed edges between nodes -- even across workspaces.

### RELATE

```sql
-- Basic relation
RELATE FROM path='/articles/post-1' TO path='/tags/rust' TYPE 'tagged-with'

-- With weight (0.0 to 1.0)
RELATE FROM path='/articles/post-1' TO path='/articles/post-2'
  TYPE 'similar-to' WEIGHT 0.85

-- Cross-workspace
RELATE
  FROM path='/articles/post-1' IN WORKSPACE 'social'
  TO path='/tags/rust' IN WORKSPACE 'social'
  TYPE 'tagged-with' WEIGHT 0.9

-- By node ID
RELATE FROM id='abc-123' TO id='def-456' TYPE 'follows'
```

### UNRELATE

```sql
-- Remove a specific relation type
UNRELATE FROM path='/articles/post-1' IN WORKSPACE 'social'
  TO path='/tags/rust' IN WORKSPACE 'social'
  TYPE 'tagged-with'

-- Remove all relations between two nodes
UNRELATE FROM path='/articles/post-1' TO path='/articles/post-2'
```

### NEIGHBORS (simple graph traversal)

Query connected nodes in one hop:

```sql
SELECT n.id, n.path, n.name, n.relation_type, n.weight
FROM NEIGHBORS('social:/articles/tech/rust-web-dev', 'OUT', 'tagged-with') AS n

SELECT n.path, n.relation_type
FROM NEIGHBORS('social:/articles/tech/my-post', 'OUT', NULL) AS n
```

Directions: `'OUT'` (outgoing), `'IN'` (incoming), `'BOTH'`.

### REFERENCES (reverse lookup)

Find all nodes that reference a target path. The argument MUST be
`'workspace:/path'` — a bare path errors (paths are only unique per
workspace). Backed by the reverse reference index (keyed by the target's
stable id, so it survives target moves), and it works cross-workspace.

```sql
SELECT * FROM social
WHERE REFERENCES('social:/tags/tech-stack/rust')
  AND node_type = 'news:Article'

-- Composes with hierarchy scoping, ordering, limits, and bound parameters:
-- $1 = 'tags:/university/data'
SELECT id, path, name FROM stories
WHERE REFERENCES($1)
  AND DESCENDANT_OF('/university')
  AND node_type = 'studio:Page'
ORDER BY properties->>'published_at'::String DESC
LIMIT 20
```

REFERENCES drives the scan (only the referrers are read); the other
predicates filter that small set. `COUNT(*)` over a REFERENCES filter works
for per-tag facet counts.

## 11. GRAPH_TABLE (SQL/PGQ -- ISO SQL:2023)

For multi-hop patterns and complex graph queries, use `GRAPH_TABLE`:

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (pattern)
  [WHERE condition]
  COLUMNS (output_columns)
) AS alias
```

### Pattern syntax

| Pattern | Meaning |
|---------|---------|
| `(a:Article)` | Node whose type ends in `:Article` (see below) |
| ``(a:`news:Article`)`` | Node of exactly that type |
| `-[:tagged-with]->` | Outgoing relation of type `tagged-with` |
| `<-[:corrects]-` | Incoming relation |
| `-[r:follows]-` | Any direction, bind to variable `r` |
| `-[:continues]->{1,3}` | 1 to 3 hops (canonical) |
| `-[:continues*1..3]->` | same, deprecated spelling |

### Labels are matched loosely -- quote to be exact

A label matches when the node type **equals** it or **ends with** `:label`,
case-insensitively. So `(a:Article)` matches `news:Article` *and*
`studio:Article`.

That is deliberate -- it lets you name a type without hardcoding its package.
When two packages share a local name and you need one, backtick-quote the full
type (backticks make an identifier accept any character):

```sql
MATCH (a:Article)              -- every namespace
MATCH (a:`news:Article`)       -- exactly one
MATCH (a:news:Article)         -- parse error: quote it
```

`WHERE a.node_type = 'news:Article'` works too.

### Quantifiers, selectors, restrictors

The canonical quantifier is the brace form **after** the arrow: `->{2}`,
`->{1,3}`, `->{2,}`, `->*` (`{0,}`), `->+` (`{1,}`), `->?` (`{0,1}`). The old
in-bracket form (`*1..3`) still works but warns; note `*` means `{1,}` there and
`{0,}` in the canonical form.

An **unbounded** quantifier (`*`, `+`, `{m,}`) must sit under a selector or a
restrictor, or it is a parse error. Bounded ones need neither. Unbounded is
capped at 10 hops.

| | |
|---|---|
| Selectors | `ANY`, `ANY SHORTEST`, `ALL SHORTEST`, `ANY CHEAPEST` (needs `COST`) |
| Restrictors | `WALK`, `TRAIL`, `ACYCLIC` (**default**) |
| Path accessors | `path_length(p)`, `nodes(p)`, `edges(p)`, `path_first(p)`, `path_last(p)`, `element_id(p)`, `is_trail(p)`, `is_acyclic(p)` |

```sql
-- Cheapest route, not the shortest. COST needs a BOUND edge variable, and a
-- relation has no arbitrary properties -- the weight field is `r.weight`.
SELECT * FROM GRAPH_TABLE(
  MATCH ANY CHEAPEST p = (a:Stop)-[r:route COST r.weight]->{1,8}(b:Stop)
  COLUMNS (path_length(p) AS hops)
)
```

A path variable is not selectable on its own -- `COLUMNS (p)` is an error naming
the accessors. Note also that a single-node pattern (`MATCH (n)`) resolves from
the relation index, so it returns only nodes that have at least one relation.

### Find tags for an article

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (article:Article)-[:tagged-with]->(tag:Tag)
  WHERE article.path = '/articles/tech/rust-web-dev'
  COLUMNS (tag.path, tag.name AS label)
) AS tags
```

### Find related articles (multiple relation types)

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (this)-[r:`similar-to`|`see-also`|updates]->(related)
  WHERE this.path = '/articles/tech/rust-web-dev'
  COLUMNS (
    related.id AS id,
    related.path AS path,
    related.name AS title,
    related.properties AS properties,
    r.type AS relation_type,
    r.weight AS weight
  )
) AS g
ORDER BY g.weight DESC
LIMIT 5
```

### Multi-hop chain (article timeline)

```sql
SELECT * FROM GRAPH_TABLE(
  MATCH (this)-[:continues*]->(prev)
  WHERE this.path = '/articles/tech/part-3'
  COLUMNS (
    prev.path AS path,
    prev.name AS name,
    prev.properties AS properties,
    prev.created_at AS created_at
  )
) AS g
ORDER BY (g.properties ->> 'publishing_date')::TIMESTAMP ASC
```

### GRAPH_TABLE composes with standard SQL

```sql
-- With JOINs
SELECT g.title, n.properties->>'excerpt' AS excerpt
FROM GRAPH_TABLE(
  MATCH (source:Article)-[r:`similar-to`]->(target:Article)
  WHERE source.path = $1
  COLUMNS (target.id AS id, target.name AS title, r.weight)
) AS g
JOIN social n ON n.id = g.id
WHERE n.properties ->> 'status' = 'published'
ORDER BY g.weight DESC
LIMIT 5

```

## 12. RESOLVE (Dereference References)

Resolve `raisin:ref` objects in JSONB, replacing them with the referenced node's data:

```sql
-- Resolve at depth 1 (default)
SELECT RESOLVE(properties) FROM social WHERE path = $1

-- Resolve nested references up to depth 3 (max 10)
SELECT RESOLVE(properties, 3) FROM social WHERE path = '/posts/my-post'
```

References are JSON objects with `raisin:ref` (path or ID) and `raisin:workspace` keys.

## 13. FULLTEXT_MATCH

Full-text search on indexed properties:

```sql
SELECT * FROM social WHERE FULLTEXT_MATCH('database management', 'english')
```

Requires `index: [Fulltext]` on the property in the NodeType definition.

For keyword search without a full-text index, use ILIKE:

```sql
SELECT * FROM social
WHERE DESCENDANT_OF('/articles')
  AND (
    COALESCE(properties ->> 'title', '') ILIKE '%' || $1 || '%'
    OR COALESCE(properties ->> 'body', '') ILIKE '%' || $1 || '%'
  )
ORDER BY properties ->> 'publishing_date' DESC
LIMIT 20
```

## 14. Geospatial Functions

RaisinDB registers **62** PostGIS-compatible geospatial functions. Coordinates
use WGS84 (EPSG:4326) in GeoJSON `[longitude, latitude]` order -- longitude
FIRST. An unambiguously reversed pair is rejected naming the corrected call.

**Units differ from PostGIS on 4326, deliberately.** `ST_DISTANCE`, `ST_LENGTH`,
`ST_AREA`, and the `d`/`t` arguments of `ST_DWITHIN` / `ST_BUFFER` /
`ST_SIMPLIFY` are all **metres**, where PostGIS `geometry` gives degrees. So
`ST_SIMPLIFY(boundary, 25)` is a 25 m tolerance, not 25 degrees.

A geometry argument may be written three ways -- `location`,
`properties->>'location'`, or `CAST(properties->>'location' AS GEOMETRY)`. All
three work and return the same rows.

### Creating Geometries

```sql
-- Point from coordinates
SELECT ST_POINT(-122.4194, 37.7749)

-- Parse GeoJSON
SELECT ST_GEOMFROMGEOJSON('{"type":"Polygon","coordinates":[...]}')

-- Bounding box
SELECT ST_MAKEENVELOPE(-122.5, 37.7, -122.4, 37.8)

-- Line from two points
SELECT ST_MAKELINE(ST_POINT(-122.4, 37.7), ST_POINT(-122.3, 37.8))
```

### Proximity Queries (Indexed)

```sql
-- Find stores within 5km (uses spatial index)
SELECT name, ST_DISTANCE(location, ST_POINT($1, $2)) AS distance
FROM stores
WHERE ST_DWITHIN(location, ST_POINT($1, $2), 5000)
ORDER BY distance

-- Nearest 10 locations
SELECT name, location FROM stores
ORDER BY ST_DISTANCE(location, ST_POINT($1, $2))
LIMIT 10
```

### Containment & Predicates

```sql
-- Points in a region
SELECT * FROM stores
WHERE ST_CONTAINS(
    (SELECT boundary FROM regions WHERE name = 'Downtown'),
    location
)

-- Overlapping zones
SELECT a.name, b.name FROM zones a JOIN zones b
  ON ST_INTERSECTS(a.boundary, b.boundary)
WHERE a.id < b.id
```

### Measurements

```sql
-- Area of a region (sq meters)
SELECT name, ST_AREA(boundary) FROM regions

-- Route length (meters)
SELECT name, ST_LENGTH(path) FROM routes

-- Bearing between two cities
SELECT ST_AZIMUTH(ST_POINT(-122.4, 37.7), ST_POINT(-73.9, 40.7))
```

### Geometry Processing

```sql
-- 2km buffer zone around a store
SELECT ST_BUFFER(location, 2000) AS zone FROM stores WHERE id = $1

-- Simplify a complex polygon
SELECT ST_SIMPLIFY(boundary, 0.001) FROM regions

-- Bounding box of a geometry
SELECT ST_ENVELOPE(boundary) FROM regions

-- Overlap area between two zones
SELECT ST_AREA(ST_INTERSECTION(a.boundary, b.boundary))
FROM zones a, zones b WHERE a.id = $1 AND b.id = $2
```

### Geometry Info

```sql
SELECT ST_GEOMETRYTYPE(location),  -- 'ST_Point'
       ST_NUMPOINTS(boundary),     -- vertex count, for ANY type (PostGIS: LineString only)
       ST_ISVALID(boundary),       -- true/false
       ST_SRID(location)           -- 4326
FROM regions LIMIT 1
```

### Nested geometry

A geometry may sit anywhere in the property tree, and the string you write IS
the index key -- so name the dot path:

| where it sits | how to query it |
|---|---|
| top level | `properties->>'location'` |
| inside an object | `properties->>'venue.geo'` |
| inside a section element | `properties->>'hero.map_pin'` |
| one array element | `properties->>'stops.0.geo'` |
| every array element | `properties->>'stops[].geo'` -- correct, but a row scan, never indexed |

Two opt-in columns say HOW a spatial predicate was satisfied. Name them
explicitly; `SELECT *` does not expand them.

```sql
SELECT name, __distance, __matched_path FROM 'places'
WHERE ST_DWITHIN(CAST(properties->>'stops[].geo' AS GEOMETRY),
                 ST_POINT($1, $2), 500)
```

`__distance` is metres (the MINIMUM over a node's matched geometries) and
`__matched_path` is the concrete path that achieved it (`stops.3.geo`). A node
matching via several geometries is returned **once**, so `LIMIT k` means k nodes.

### Altitude (3D)

A position may carry a third ordinate. `ST_NDIMS` reports 3 when any position
has one; `ST_Z` is Point-only, so use `ST_ZMIN` / `ST_ZMAX` for other types.
`ST_FORCE3D(geom, z)` fills a missing Z (it does not overwrite -- drop with
`ST_FORCE2D` first).

```sql
-- Within 500 m in space, not just on the ground. Narrows through the 2-D
-- index and re-checks altitude per candidate row.
SELECT callsign FROM 'flights'
WHERE ST_3DDWITHIN(CAST(properties->>'position' AS GEOMETRY),
                   ST_FORCE3D(ST_POINT($1, $2), $3), 500)
```

`ST_FORCE3D` is the only way to write a 3-D constant point -- `ST_POINT` and
`ST_MAKEPOINT` both reject a third argument.

### High-frequency positions

For a tracked object, cut the indexed precisions -- each one costs a write plus
a tombstone per update:

```sql
ALTER SPATIAL INDEX FOR 'fleet' PROPERTY 'position' SET PRECISIONS = (8, 6);
```

That takes the default 8 precisions (16 writes/update) down to 2 (4/update).
Superseded entries are pruned by a compaction filter, on by default, so a hot
cell stays constant rather than growing with update count.

## 15. Pagination & Prev/Next Navigation

Offset paging works, but prefer keyset (cursor) queries — they stay fast at
any depth and give you prev/next navigation for free:

```sql
-- Offset paging (simple)
SELECT * FROM blog WHERE node_type = 'blog:Article'
ORDER BY created_at DESC LIMIT 20 OFFSET 40

-- Keyset paging by path (hierarchical listings; sibling paths sort naturally)
SELECT * FROM blog
WHERE CHILD_OF('/posts') AND path > $1   -- $1 = last path of previous page
ORDER BY path LIMIT 20

-- Next / previous sibling of a node (path order)
SELECT * FROM blog WHERE CHILD_OF('/posts') AND path > $1 ORDER BY path ASC  LIMIT 1
SELECT * FROM blog WHERE CHILD_OF('/posts') AND path < $1 ORDER BY path DESC LIMIT 1

-- Older / newer article links (property cursor; ->> yields text, so keep
-- cursor values lexicographically sortable, e.g. ISO dates)
SELECT * FROM blog
WHERE DESCENDANT_OF('/posts') AND properties->>'published_at'::String < $1
ORDER BY properties->>'published_at'::String DESC LIMIT 1
```

### Editorial-order cursors (`__order`) — the fastest folder pagination

`CHILD_OF('/parent')` with no `ORDER BY` returns children in editorial
(drag-and-drop) order, and that order **is** exposed as the `__order` column, so
it can drive a keyset cursor. This is the only ordering inside a folder that is
a pure index seek — cost depends on the page size, not on how big the folder is.

```sql
-- page 1 (select __order to get the cursor for the next page)
SELECT name, __order FROM mail
WHERE CHILD_OF('/inbox/2026/05')
ORDER BY __order LIMIT 50

-- page 2+ : feed back the LAST row's __order
SELECT name, __order FROM mail
WHERE CHILD_OF('/inbox/2026/05') AND __order > $1
ORDER BY __order LIMIT 50

-- newest-first (insertion order tracks arrival for imported content)
SELECT name, __order FROM mail
WHERE CHILD_OF('/inbox/2026/05') AND __order < $1
ORDER BY __order DESC LIMIT 50
```

**Match the comparison to the direction: `>` with `ASC`, `<` with `DESC`.** The
cursor is a start position, not a range bound. Get it backwards and the bound is
silently dropped — you still get correct rows, but the query reads the whole
folder on every page instead of seeking. Same for `>=`: only `>` and `<` are
recognised as cursors.

`__order` is opaque sortable text. Pass it back verbatim as a string; never cast
it to a number or compare it to a `path`.

`SELECT *` already includes `__order`; an explicit projection must name it.

Notes:
- **Never mix an `__order` cursor with `ORDER BY path`** (or vice versa). Both
  order parents before children, but `path` sorts siblings alphabetically while
  `__order` sorts them editorially. They agree only while the manual order
  happens to be alphabetical — so this looks fine until someone drags something,
  then silently drops and duplicates rows.
- Every node carries `created_at`/`updated_at` (stamped at the storage layer
  on every write path), so `ORDER BY created_at` is always meaningful. Nodes
  written before this guarantee may have NULL `created_at`; prefer an explicit
  property (e.g. `published_at`) when your data predates it.
- `ORDER BY` on anything OTHER than `__order` inside a folder (`created_at`, a
  property) cannot seek by default — it reads the folder and sorts. To make it a
  seek, declare a compound index; see §15.1.

### 15.1 Making `ORDER BY` inside a folder a seek

`WHERE CHILD_OF(...) ORDER BY <property>` reads every child and sorts, because
no index carries both a hierarchy handle and a value ordering. Declare a
compound index on the NodeType to fix that:

```yaml
compound_indexes:
  - name: folder_time
    columns: ["__parent_path", "__created_at"]
    has_order_column: true
```

`__parent_path` is the containing directory, so `CHILD_OF` becomes an equality
on the leading column and the trailing column serves the `ORDER BY`. The sort is
then eliminated and the `LIMIT` bounds the scan.

```sql
-- a seek once the index above exists
SELECT name FROM mail
WHERE CHILD_OF('/inbox/2026/05')
ORDER BY created_at DESC LIMIT 50
```

Constraints worth knowing before you rely on it:

- **`DESCENDANT_OF` cannot use this.** A subtree is a path *range*, and in any
  sorted index a range cannot precede the order column — only equality can. Use
  `CHILD_OF`, or sort a bounded subtree result.
- **Existing rows are not in a new index.** Entries are written on write, so
  data created before the index needs a rebuild.
- **Prefer `__created_at` / `__updated_at` as the order column**, declared with
  `column_type: Timestamp` — they are stored inverted, so newest-first is a
  forward scan that bounds properly. A `String` column ordered `DESC` is correct
  but reads the whole group before truncating.
- Index **names are branch-global**: two NodeTypes declaring the same name share
  one keyspace. Give each index a distinct name.

## 16. Schema Tables (Reserved)

Four reserved table names read the **type registry** rather than content nodes:

| Table | Contents | Writable via SQL |
|-------|----------|------------------|
| `NodeTypes` | Registered node types | DDL only — `CREATE/ALTER/DROP NODETYPE` |
| `Archetypes` | Page templates | DDL only — `CREATE/ALTER/DROP ARCHETYPE` |
| `ElementTypes` | Content blocks | DDL only — `CREATE/ALTER/DROP ELEMENTTYPE` |
| `Workspaces` | Workspace definitions | **No — read-only** |

They are queryable from the HTTP SQL API and from `raisin.sql` in server-side
functions, which is the only route to schema information inside a function: the
function runtime has no workspaces/types binding, and `raisin.asAdmin()` only
re-exposes nodes and SQL.

```sql
SELECT name, base_node_type, extends, title, fields, meta FROM Archetypes
SELECT name, allowed_children FROM NodeTypes
SELECT name, extends, description, fields, meta FROM ElementTypes
SELECT name, allowed_node_types, allowed_root_node_types FROM Workspaces
```

Nothing is cached — a freshly deployed type is visible on the next query, with
no catalog to rebuild.

### Three rules

**Full-table SELECTs only.** An equality `WHERE name = 'x'` plans a point-lookup
that bypasses the schema-table read path and silently returns **nothing**.
Filter in your own code instead:

```sql
-- WRONG: returns no rows
SELECT fields FROM Archetypes WHERE name = 'news:ArticlePage'
-- RIGHT: read all, filter client-side
SELECT name, fields FROM Archetypes
```

**`ElementTypes` has no `title` column.** Selecting it errors the whole query.

**`fields` and `meta` are raw, not `extends`-merged.** Walk the `extends` chain
yourself if you need the resolved schema.

### `Workspaces` specifics

Read-only, and repo-scoped rather than branch-scoped — workspaces are shared
across branches and carry no revision history. Writes are refused, because a row
write would skip everything that actually creating a workspace involves
(building its nodes table, seeding `initial_structure`, registering it in the
SQL catalog). Define workspaces in `workspaces/*.yaml` and install the package,
or use the management API.

Columns: `name`, `description`, `allowed_node_types`, `allowed_root_node_types`,
`depends_on`, `initial_structure`, `config`, `created_at`, `updated_at`.

`allowed_node_types` / `allowed_root_node_types` are the **coarse, server-enforced
containment rule** — which node types may exist in a workspace at all, and which
may sit at its root. Combined with `NodeTypes.allowed_children` (structural
composition, for typed parents) they let a server-side function answer "what may
be created here?" the same way an admin UI does:

```sql
-- What may exist in this workspace, and what does this parent type accept?
SELECT name, allowed_node_types, allowed_root_node_types FROM Workspaces
SELECT name, allowed_children FROM NodeTypes
```

Because `workspaces` is now a reserved table name, a *content* workspace may not
be named `workspaces` — the schema table would shadow it.

## Quick Reference: Statement Summary

| Operation | Syntax |
|-----------|--------|
| Select | `SELECT ... FROM workspace WHERE ...` |
| Insert | `INSERT INTO workspace (path, node_type, properties) VALUES (...)` |
| Update | `UPDATE workspace SET properties = properties \|\| $1::jsonb WHERE ...` |
| Delete | `DELETE FROM workspace WHERE path = $1` |
| Move | `MOVE workspace SET path=$1 TO path=$2` |
| Copy | `COPY workspace SET path=$1 TO path=$2 AS 'name'` |
| Copy tree | `COPY TREE workspace SET path=$1 TO path=$2` |
| Order | `ORDER workspace SET path=$1 ABOVE/BELOW path=$2` |
| Relate | `RELATE FROM path=$1 TO path=$2 TYPE 'name' [WEIGHT n]` |
| Unrelate | `UNRELATE FROM path=$1 TO path=$2 [TYPE 'name']` |
| Graph query | `SELECT * FROM GRAPH_TABLE(MATCH pattern COLUMNS (...)) AS alias` |
| Fulltext | `WHERE FULLTEXT_MATCH('terms', 'language')` |
| Hierarchy | `WHERE CHILD_OF('/path')` / `WHERE DESCENDANT_OF('/path')` |
| Resolve | `SELECT RESOLVE(properties) FROM workspace WHERE ...` |
| References | `WHERE REFERENCES('workspace:/path')` |
| Geospatial | `WHERE ST_DWITHIN(location, ST_POINT($1, $2), 5000)` |
| Schema tables | `SELECT ... FROM NodeTypes \| Archetypes \| ElementTypes \| Workspaces` (no `WHERE name=`) |
