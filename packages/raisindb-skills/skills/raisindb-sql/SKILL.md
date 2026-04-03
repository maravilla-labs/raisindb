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

Find all nodes that reference a target path:

```sql
SELECT * FROM social
WHERE REFERENCES('social:/tags/tech-stack/rust')
  AND node_type = 'news:Article'
```

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
| `(a:Article)` | Node with label `Article` |
| `-[:tagged-with]->` | Outgoing relation of type `tagged-with` |
| `<-[:corrects]-` | Incoming relation |
| `-[r:follows]-` | Any direction, bind to variable `r` |
| `-[:continues*]->` | Variable-length (1-10 hops, default) |
| `-[:follows*2..5]->` | 2 to 5 hops |

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

RaisinDB supports 49 PostGIS-compatible geospatial functions. Coordinates use WGS84 (EPSG:4326) in GeoJSON `[longitude, latitude]` order.

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
       ST_NUMPOINTS(boundary),     -- coordinate count
       ST_ISVALID(boundary),       -- true/false
       ST_SRID(location)           -- 4326
FROM regions LIMIT 1
```

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
