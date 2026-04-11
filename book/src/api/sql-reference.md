# SQL Reference

RaisinDB provides a content-aware SQL dialect based on PostgreSQL syntax. Queries are workspace-scoped and operate on RaisinDB's hierarchical node model. You can connect via the PostgreSQL wire protocol (psql, any PostgreSQL driver) or the HTTP query endpoint.

## Connecting

### PGWire (PostgreSQL Wire Protocol)

RaisinDB exposes a PostgreSQL-compatible wire protocol on port 5432 (default).

```bash
# Connect with psql
psql -h localhost -p 5432 -U tenant_id -d repo_name

# Connection string format
postgresql://tenant_id:api_key@localhost:5432/repo_name
```

The username maps to the tenant ID and the database name maps to the repository. Branch context can be set after connecting with `USE BRANCH`.

### HTTP Query Endpoint

Queries can also be submitted via the REST API as JSON:

```bash
curl -X POST http://localhost:3000/api/v1/tenants/{tenant}/repos/{repo}/sql \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT * FROM workspace LIMIT 10", "params": []}'
```

## Data Model Mapping

RaisinDB maps its content model to SQL tables as follows:

| RaisinDB Concept | SQL Representation |
|---|---|
| Workspace | Table name (e.g., `FROM 'my-workspace'`) |
| Node | Row |
| Node properties | `properties` column (JSONB) |
| Node path | `path` column (PATH type) |
| Node type | `node_type` column (TEXT) |

### The `nodes` Table

The default table `nodes` (or any workspace name used as a table) exposes these columns:

| Column | Type | Nullable | Description |
|---|---|---|---|
| `id` | TEXT | No | Unique node identifier |
| `path` | PATH | No | Hierarchical path (e.g., `/content/blog/post1`) |
| `name` | TEXT | No | Node name (last path segment) |
| `node_type` | TEXT | No | Type with namespace (e.g., `myapp:Article`) |
| `archetype` | TEXT | Yes | Archetype name |
| `properties` | JSONB | No | All node properties as JSON |
| `parent_name` | TEXT | Yes | Parent node name |
| `version` | INT | No | Node version number |
| `created_at` | TIMESTAMPTZ | No | Creation timestamp |
| `updated_at` | TIMESTAMPTZ | No | Last update timestamp |
| `published_at` | TIMESTAMPTZ | Yes | Publication timestamp |
| `published_by` | TEXT | Yes | User who published |
| `updated_by` | TEXT | Yes | User who last updated |
| `created_by` | TEXT | Yes | User who created |
| `translations` | JSONB | Yes | Translation data |
| `owner_id` | TEXT | Yes | Owner user ID |
| `relations` | JSONB | Yes | Graph relations |
| `parent_path` | PATH | Yes | Parent node path (generated) |
| `depth` | INT | No | Tree depth from root (generated) |
| `locale` | TEXT | No | Locale code (generated) |
| `__revision` | BIGINT | Yes | Internal revision (generated) |
| `__branch` | TEXT | No | Branch name (generated) |
| `__workspace` | TEXT | No | Workspace name (generated) |
| `embedding` | VECTOR(N) | Yes | Vector embedding (when configured) |

### Schema Tables

DML operations are also supported on schema management tables:

- `NodeTypes` -- Node type definitions
- `Archetypes` -- Archetype definitions
- `ElementTypes` -- Element type definitions

## Data Types

### Core Types

| Type | Description | Example |
|---|---|---|
| `INT` | 32-bit integer | `42` |
| `BIGINT` | 64-bit integer | `9223372036854775807` |
| `DOUBLE` | 64-bit floating point | `3.14` |
| `BOOLEAN` | True/false | `true`, `false` |
| `TEXT` | UTF-8 string | `'hello'` |
| `UUID` | UUID string | `'550e8400-e29b-41d4-a716-446655440000'` |

### Temporal Types

| Type | Description |
|---|---|
| `TIMESTAMPTZ` | Timestamp with timezone (UTC normalized) |
| `INTERVAL` | Time interval / duration |

### RaisinDB-Specific Types

| Type | Description |
|---|---|
| `PATH` | Hierarchical path (e.g., `/content/blog/post1`) |
| `JSONB` | JSON data (maps to node properties) |
| `VECTOR(N)` | Fixed-dimension vector for embeddings |
| `GEOMETRY` | GeoJSON geometry (Point, LineString, Polygon) |

### Search Types

| Type | Description |
|---|---|
| `TSVECTOR` | Full-text search document |
| `TSQUERY` | Full-text search query |

### Collection Types

| Type | Description |
|---|---|
| `Array(T)` | Array of elements (e.g., `TEXT[]`, `INT[]`) |
| `Nullable(T)` | Nullable wrapper (e.g., `TEXT?`) |

### Type Coercion (Implicit)

The following implicit coercions are performed automatically:

- `INT` -> `BIGINT` -> `DOUBLE` (numeric widening)
- `TEXT` -> `PATH` (for literal comparisons)
- `T` -> `Nullable(T)` (non-null to nullable)

### Explicit Casting

Use the `::Type` syntax for explicit casts:

```sql
-- Cast JSON property value to a specific type
SELECT * FROM 'workspace' WHERE properties->>'age'::String = '25'

-- Cast between text and numeric types
SELECT version::TEXT FROM 'workspace'
SELECT '42'::INT
```

Allowed explicit casts include:

| From | To |
|---|---|
| TEXT | INT, BIGINT, DOUBLE, BOOLEAN, JSONB, PATH, TIMESTAMPTZ, GEOMETRY |
| INT, BIGINT, DOUBLE | TEXT |
| BOOLEAN | TEXT |
| DOUBLE | INT, BIGINT |
| BIGINT | INT |
| PATH | TEXT |
| JSONB | TEXT |
| TIMESTAMPTZ | TEXT |
| GEOMETRY | TEXT |

JSONB values can be cast to BOOLEAN, INT, BIGINT, DOUBLE, or PATH through an intermediate TEXT conversion (two-step cast handled automatically).

## Statements

### SELECT

```sql
SELECT [DISTINCT] columns
FROM table [AS alias]
[JOIN ...]
[WHERE condition]
[GROUP BY expressions]
[HAVING condition]
[ORDER BY expressions [ASC|DESC]]
[LIMIT count]
[OFFSET count]
```

Table names can be quoted with single quotes for workspace names:

```sql
SELECT * FROM 'my-workspace' WHERE node_type = 'myapp:Article'
```

#### WITH (Common Table Expressions)

```sql
WITH recent_posts AS (
  SELECT * FROM 'workspace' WHERE node_type = 'cms:Post' ORDER BY created_at DESC LIMIT 10
)
SELECT id, properties->>'title'::String AS title FROM recent_posts
```

#### DISTINCT

```sql
SELECT DISTINCT node_type FROM 'workspace'
```

#### Subqueries in FROM

```sql
SELECT sub.title
FROM (
  SELECT properties->>'title'::String AS title FROM 'workspace'
) AS sub
WHERE sub.title IS NOT NULL
```

### INSERT

```sql
INSERT INTO nodes (path, node_type, properties)
VALUES ('/content/blog/post1', 'myapp:Article', '{"title": "Hello World"}')
```

### UPSERT

Identical syntax to INSERT but uses create-or-update semantics (will update if the node already exists):

```sql
UPSERT INTO nodes (path, node_type, properties)
VALUES ('/content/blog/post1', 'myapp:Article', '{"title": "Updated Title"}')
```

### UPDATE

```sql
UPDATE nodes SET properties = '{"status": "published"}' WHERE id = 'node-123'
```

### DELETE

```sql
DELETE FROM nodes WHERE id = 'node-123'
```

### EXPLAIN

Shows the query execution plan:

```sql
EXPLAIN SELECT * FROM 'workspace' WHERE node_type = 'cms:Article'
EXPLAIN VERBOSE SELECT * FROM 'workspace' WHERE depth = 2
EXPLAIN ANALYZE SELECT * FROM 'workspace' LIMIT 10
```

### SHOW

```sql
SHOW search_path
SHOW server_version
```

## Expressions and Operators

### Comparison Operators

| Operator | Description |
|---|---|
| `=` | Equal |
| `<>` or `!=` | Not equal |
| `<` | Less than |
| `<=` | Less than or equal |
| `>` | Greater than |
| `>=` | Greater than or equal |

### Logical Operators

| Operator | Description |
|---|---|
| `AND` | Logical AND |
| `OR` | Logical OR |
| `NOT` | Logical NOT |

### Arithmetic Operators

| Operator | Description |
|---|---|
| `+` | Addition (also: TIMESTAMPTZ + INTERVAL) |
| `-` | Subtraction (also: TIMESTAMPTZ - INTERVAL, TIMESTAMPTZ - TIMESTAMPTZ -> INTERVAL) |
| `*` | Multiplication |
| `/` | Division |
| `%` | Modulo |

### String Operators

| Operator | Description |
|---|---|
| `\|\|` | String concatenation |
| `LIKE` | Pattern matching (`%` = any chars, `_` = one char) |
| `ILIKE` | Case-insensitive pattern matching |
| `NOT LIKE` | Negated pattern matching |
| `NOT ILIKE` | Negated case-insensitive pattern matching |

### Null Checks

```sql
WHERE published_at IS NULL
WHERE properties->>'title'::String IS NOT NULL
```

### IN Lists and Subqueries

```sql
WHERE node_type IN ('cms:Article', 'cms:Page')
WHERE node_type NOT IN ('cms:Draft')
WHERE id IN (SELECT id FROM 'other-workspace' WHERE node_type = 'cms:Tag')
WHERE id NOT IN (SELECT id FROM 'archive')
```

### BETWEEN

```sql
WHERE version BETWEEN 1 AND 10
```

### CASE Expressions

```sql
SELECT
  CASE
    WHEN depth > 3 THEN 'deep'
    WHEN depth > 1 THEN 'medium'
    ELSE 'shallow'
  END AS depth_category
FROM 'workspace'
```

## JSON Property Access

RaisinDB stores node properties as JSONB. The JSON operators are the primary way to query property values.

### The `->>` Operator (Extract as Text)

```sql
-- Extract a property as text
SELECT properties->>'title' AS title FROM 'workspace'
```

**Important**: When using `->>` in a WHERE clause, cast the **key** to `String`:

```sql
-- Correct: cast the key
SELECT * FROM 'workspace' WHERE properties->>'user_id'::String = $1
SELECT * FROM 'workspace' WHERE properties->>'email'::String = $1

-- Wrong: cast the result (causes type error)
SELECT * FROM 'workspace' WHERE (properties->>'user_id')::String = $1

-- Wrong: no cast (returns empty results)
SELECT * FROM 'workspace' WHERE properties->>'user_id' = $1
```

### The `->` Operator (Extract as JSON)

```sql
-- Extract a nested JSON object
SELECT properties->'metadata' FROM 'workspace'
```

### The `@>` Operator (JSON Containment)

```sql
-- Find nodes where properties contain specific key-value pairs
SELECT * FROM 'workspace' WHERE properties @> '{"status": "published"}'
```

### The `?` Operator (Key Existence)

```sql
-- Check if a key exists in the JSON object
SELECT * FROM 'workspace' WHERE properties ? 'featured'
```

### The `?|` and `?&` Operators (Multiple Key Existence)

```sql
-- Any key exists
SELECT * FROM 'workspace' WHERE properties ?| ARRAY['title', 'subtitle']

-- All keys exist
SELECT * FROM 'workspace' WHERE properties ?& ARRAY['title', 'author']
```

### The `#>` and `#>>` Operators (Path Extraction)

```sql
-- Extract JSON at path
SELECT properties #> ARRAY['metadata', 'author'] FROM 'workspace'

-- Extract text at path
SELECT properties #>> ARRAY['metadata', 'author'] FROM 'workspace'
```

### The `-` Operator (JSON Remove)

```sql
-- Remove a key from JSON
SELECT properties - 'temp_field' FROM 'workspace'
```

### The `#-` Operator (Remove at Path)

```sql
-- Remove value at a nested path
SELECT properties #- ARRAY['metadata', 'draft_notes'] FROM 'workspace'
```

### JSON Path Operators

```sql
-- JSONPath match: @@ tests if predicate matches
SELECT * FROM 'workspace' WHERE properties @@ '$.tags[*] ? (@ == "rust")'

-- JSONPath exists: @? tests if path has matches
SELECT * FROM 'workspace' WHERE properties @? '$.metadata.author'
```

### The `||` Operator (JSON Merge)

```sql
-- Merge two JSONB values
SELECT properties || '{"new_key": "value"}' FROM 'workspace'
```

## Built-in Functions

### String Functions

| Function | Signature | Description |
|---|---|---|
| `LOWER(text)` | TEXT -> TEXT | Convert to lowercase |
| `UPPER(text)` | TEXT -> TEXT | Convert to uppercase |
| `LENGTH(text)` | TEXT -> INT | String length |

### Math Functions

| Function | Signature | Description |
|---|---|---|
| `ROUND(value)` | DOUBLE -> DOUBLE | Round to nearest integer |
| `ROUND(value, precision)` | DOUBLE, INT -> DOUBLE | Round to N decimal places |

### Null Handling Functions

| Function | Signature | Description |
|---|---|---|
| `COALESCE(val1, val2, ...)` | ANY... -> ANY | First non-NULL value |
| `NULLIF(val1, val2)` | ANY, ANY -> ANY | NULL if values are equal |

### Temporal Functions

| Function | Signature | Description |
|---|---|---|
| `NOW()` | -> TIMESTAMPTZ | Current UTC timestamp |

### JSON Functions

| Function | Signature | Description |
|---|---|---|
| `JSON_VALUE(json, path)` | JSONB, TEXT -> TEXT? | Extract scalar value at path |
| `JSON_QUERY(json, path)` | JSONB, TEXT -> JSONB? | Extract JSON at path |
| `JSON_EXISTS(json, path)` | JSONB, TEXT -> BOOLEAN | Check if path exists |
| `JSON_GET_TEXT(json, key)` | JSONB, TEXT -> TEXT? | Extract as text |
| `JSON_GET_DOUBLE(json, key)` | JSONB, TEXT -> DOUBLE? | Extract as double |
| `JSON_GET_INT(json, key)` | JSONB, TEXT -> INT? | Extract as integer |
| `JSON_GET_BOOL(json, key)` | JSONB, TEXT -> BOOLEAN? | Extract as boolean |
| `TO_JSON(value)` | ANY -> JSONB | Convert to JSON |
| `TO_JSONB(value)` | ANY -> JSONB | Convert to JSONB |
| `JSONB_SET(json, path, value)` | JSONB, TEXT, ANY -> JSONB | Set value at path |
| `JSONB_SET(json, path, value, create)` | JSONB, TEXT, ANY, BOOLEAN -> JSONB | Set value, control creation |

### Hierarchy Functions

| Function | Signature | Description |
|---|---|---|
| `PATH_STARTS_WITH(path, prefix)` | PATH, PATH -> BOOLEAN | Check if path starts with prefix |
| `PARENT(path)` | PATH -> PATH? | Get parent path |
| `PARENT(path, levels)` | PATH, INT -> PATH? | Get ancestor N levels up |
| `DEPTH(path)` | PATH -> INT | Get tree depth |
| `ANCESTOR(path, level)` | PATH, INT -> PATH? | Get ancestor at specific level |
| `CHILD_OF(parent_path)` | PATH -> BOOLEAN | Check if node is direct child |
| `DESCENDANT_OF(parent_path)` | PATH -> BOOLEAN | Check if node is descendant |
| `DESCENDANT_OF(parent_path, max_depth)` | PATH, INT -> BOOLEAN | Descendants up to max depth |
| `REFERENCES(target)` | TEXT -> BOOLEAN | Check if node references target |
| `NEIGHBORS(node_id, direction, type)` | TEXT, TEXT, TEXT -> TEXT[] | Get graph neighbors |
| `RESOLVE(json)` | JSONB -> JSONB | Resolve references (depth=1) |
| `RESOLVE(json, depth)` | JSONB, INT -> JSONB | Resolve references with depth |

The `NEIGHBORS` function takes a direction parameter: `'OUT'` (outgoing), `'IN'` (incoming), or `'BOTH'`.

### Full-Text Search Functions

| Function | Signature | Description |
|---|---|---|
| `to_tsvector(config, text)` | TEXT, TEXT -> TSVECTOR | Create text search vector |
| `to_tsquery(config, text)` | TEXT, TEXT -> TSQUERY | Create text search query |
| `FULLTEXT_MATCH(query, language)` | TEXT, TEXT -> BOOLEAN | Search using Tantivy index |

Full-text search match operator:

```sql
-- Using the @@ operator with tsvector/tsquery
WHERE to_tsvector('english', properties->>'body'::String) @@ to_tsquery('english', 'database & content')

-- Using FULLTEXT_MATCH with the Tantivy index
WHERE FULLTEXT_MATCH('database content', 'english')
```

### Vector Search Functions

| Function | Signature | Description |
|---|---|---|
| `EMBEDDING(text)` | TEXT -> VECTOR | Generate embedding from text |
| `VECTOR_L2_DISTANCE(v1, v2)` | VECTOR, VECTOR -> DOUBLE | Euclidean distance |
| `VECTOR_COSINE_DISTANCE(v1, v2)` | VECTOR, VECTOR -> DOUBLE | Cosine distance |
| `VECTOR_INNER_PRODUCT(v1, v2)` | VECTOR, VECTOR -> DOUBLE | Inner product |

Vector distance operators (pgvector-compatible):

| Operator | Description |
|---|---|
| `<->` | L2 (Euclidean) distance |
| `<=>` | Cosine distance |
| `<#>` | Inner product (negative dot product) |

```sql
-- Semantic similarity search
SELECT *, embedding <-> EMBEDDING('search query') AS distance
FROM 'workspace'
ORDER BY embedding <-> EMBEDDING('search query')
LIMIT 10

-- Filter by max distance in WHERE clause
SELECT id, name, embedding <=> EMBEDDING('query') AS distance
FROM 'workspace'
WHERE embedding <=> EMBEDDING('query') < 0.3
ORDER BY distance
LIMIT 10
```

#### HYBRID_SEARCH Table Function

Combines full-text search and vector search using Reciprocal Rank Fusion (RRF):

```sql
-- Hybrid search: fulltext + vector with RRF ranking
SELECT * FROM HYBRID_SEARCH('search query', 10)

-- With additional filters
SELECT id, name, score
FROM HYBRID_SEARCH('database optimization', 20)
WHERE node_type = 'myapp:Article'
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `query` | TEXT | Search query text |
| `k` | INT | Number of results to return |

#### EXPLAIN for Vector Queries

`EXPLAIN` shows `VectorScan` details for vector queries, including distance metric, HNSW parameters, and candidate count:

```sql
EXPLAIN SELECT id, name, embedding <=> EMBEDDING('query') AS distance
FROM 'workspace'
ORDER BY distance
LIMIT 10
```

### Geospatial Functions (PostGIS-Compatible)

**Constructors:**

| Function | Signature | Description |
|---|---|---|
| `ST_POINT(lon, lat)` | DOUBLE, DOUBLE → GEOMETRY | Create a point |
| `ST_MAKEPOINT(x, y)` | DOUBLE, DOUBLE → GEOMETRY | Create a point (alias) |
| `ST_GEOMFROMGEOJSON(json)` | TEXT → GEOMETRY | Parse GeoJSON |
| `ST_MAKELINE(p1, p2)` | GEOMETRY, GEOMETRY → GEOMETRY | Create LineString from two points |
| `ST_MAKEPOLYGON(ring)` | GEOMETRY → GEOMETRY | Create Polygon from closed LineString |
| `ST_MAKEENVELOPE(xmin, ymin, xmax, ymax)` | DOUBLE×4 → GEOMETRY | Create bounding box Polygon |
| `ST_COLLECT(g1, g2)` | GEOMETRY, GEOMETRY → GEOMETRY | Collect into GeometryCollection |

**Output:**

| Function | Signature | Description |
|---|---|---|
| `ST_ASGEOJSON(geom)` | GEOMETRY → TEXT | Convert to GeoJSON string |

**Measurement:**

| Function | Signature | Description |
|---|---|---|
| `ST_DISTANCE(g1, g2)` | GEOMETRY, GEOMETRY → DOUBLE | Distance in meters |
| `ST_AREA(geom)` | GEOMETRY → DOUBLE | Area in square meters |
| `ST_LENGTH(geom)` | GEOMETRY → DOUBLE | Length in meters |
| `ST_PERIMETER(geom)` | GEOMETRY → DOUBLE | Perimeter in meters |
| `ST_AZIMUTH(p1, p2)` | GEOMETRY, GEOMETRY → DOUBLE | Bearing in radians |

**Spatial Predicates:**

| Function | Signature | Description |
|---|---|---|
| `ST_DWITHIN(g1, g2, dist)` | GEOMETRY, GEOMETRY, DOUBLE → BOOLEAN | Within distance (indexed) |
| `ST_CONTAINS(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | A contains B |
| `ST_WITHIN(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | A within B |
| `ST_INTERSECTS(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | Geometries intersect |
| `ST_DISJOINT(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | Geometries don't intersect |
| `ST_EQUALS(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | Topologically equal |
| `ST_TOUCHES(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | Boundaries touch |
| `ST_CROSSES(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | Geometry crosses another |
| `ST_OVERLAPS(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | Same-dimension overlap |
| `ST_COVERS(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | A covers B |
| `ST_COVEREDBY(g1, g2)` | GEOMETRY, GEOMETRY → BOOLEAN | A covered by B |

**Processing:**

| Function | Signature | Description |
|---|---|---|
| `ST_BUFFER(geom, dist)` | GEOMETRY, DOUBLE → GEOMETRY | Buffer zone |
| `ST_CENTROID(geom)` | GEOMETRY → GEOMETRY | Center point |
| `ST_ENVELOPE(geom)` | GEOMETRY → GEOMETRY | Bounding box |
| `ST_CONVEXHULL(geom)` | GEOMETRY → GEOMETRY | Convex hull |
| `ST_SIMPLIFY(geom, tol)` | GEOMETRY, DOUBLE → GEOMETRY | Simplify (Douglas-Peucker) |
| `ST_REVERSE(geom)` | GEOMETRY → GEOMETRY | Reverse coordinates |
| `ST_BOUNDARY(geom)` | GEOMETRY → GEOMETRY | Geometry boundary |

**Set Operations:**

| Function | Signature | Description |
|---|---|---|
| `ST_UNION(g1, g2)` | GEOMETRY, GEOMETRY → GEOMETRY | Union |
| `ST_INTERSECTION(g1, g2)` | GEOMETRY, GEOMETRY → GEOMETRY | Intersection |
| `ST_DIFFERENCE(g1, g2)` | GEOMETRY, GEOMETRY → GEOMETRY | Difference (A - B) |
| `ST_SYMDIFFERENCE(g1, g2)` | GEOMETRY, GEOMETRY → GEOMETRY | Symmetric difference |

**Accessors:**

| Function | Signature | Description |
|---|---|---|
| `ST_X(geom)` | GEOMETRY → DOUBLE? | Longitude of point |
| `ST_Y(geom)` | GEOMETRY → DOUBLE? | Latitude of point |
| `ST_GEOMETRYTYPE(geom)` | GEOMETRY → TEXT | Type name ("ST_Point", etc.) |
| `ST_NUMPOINTS(geom)` | GEOMETRY → INT | Number of coordinates |
| `ST_NUMGEOMETRIES(geom)` | GEOMETRY → INT | Number of sub-geometries |
| `ST_SRID(geom)` | GEOMETRY → INT | SRID (always 4326) |
| `ST_ISVALID(geom)` | GEOMETRY → BOOLEAN | Is geometry valid |
| `ST_ISEMPTY(geom)` | GEOMETRY → BOOLEAN | Has no coordinates |
| `ST_ISCLOSED(geom)` | GEOMETRY → BOOLEAN | Is ring closed |
| `ST_ISSIMPLE(geom)` | GEOMETRY → BOOLEAN | No self-intersections |

**Line Functions:**

| Function | Signature | Description |
|---|---|---|
| `ST_STARTPOINT(geom)` | GEOMETRY → GEOMETRY | First point of LineString |
| `ST_ENDPOINT(geom)` | GEOMETRY → GEOMETRY | Last point of LineString |
| `ST_POINTN(geom, n)` | GEOMETRY, INT → GEOMETRY | Nth point (1-based) |
| `ST_LINEINTERPOLATEPOINT(geom, frac)` | GEOMETRY, DOUBLE → GEOMETRY | Point at fraction along line |

### System Functions

| Function | Signature | Description |
|---|---|---|
| `VERSION()` | -> TEXT | RaisinDB version |
| `CURRENT_SCHEMA()` | -> TEXT | Current schema |
| `CURRENT_DATABASE()` | -> TEXT | Current database (repo) |
| `CURRENT_USER` | -> TEXT | Current user |
| `SESSION_USER` | -> TEXT | Session user |
| `CURRENT_CATALOG` | -> TEXT | Current catalog |

### Authentication Functions

| Function | Signature | Description |
|---|---|---|
| `RAISIN_AUTH_CURRENT_USER()` | -> TEXT? | Current authenticated user ID |
| `RAISIN_CURRENT_USER()` | -> JSONB? | Current user as JSON object |
| `RAISIN_AUTH_CURRENT_WORKSPACE()` | -> TEXT? | Current workspace |
| `RAISIN_AUTH_HAS_PERMISSION(resource, action)` | TEXT, TEXT -> BOOLEAN | Check permission |
| `RAISIN_AUTH_GET_SETTINGS()` | -> JSONB | Get auth settings |
| `RAISIN_AUTH_UPDATE_SETTINGS(json)` | TEXT -> JSONB | Update auth settings |
| `RAISIN_AUTH_ADD_PROVIDER(name, config)` | TEXT, TEXT -> TEXT | Add auth provider |
| `RAISIN_AUTH_UPDATE_PROVIDER(name, config)` | TEXT, TEXT -> JSONB | Update auth provider |
| `RAISIN_AUTH_REMOVE_PROVIDER(name)` | TEXT -> BOOLEAN | Remove auth provider |

### Invocation Functions

| Function | Signature | Description |
|---|---|---|
| `INVOKE(path)` | TEXT -> JSONB | Invoke a function asynchronously |
| `INVOKE(path, input)` | TEXT, JSONB -> JSONB | Invoke with input |
| `INVOKE(path, input, workspace)` | TEXT, JSONB, TEXT -> JSONB | Invoke in specific workspace |
| `INVOKE_SYNC(path)` | TEXT -> JSONB | Invoke synchronously |
| `INVOKE_SYNC(path, input)` | TEXT, JSONB -> JSONB | Invoke synchronously with input |
| `INVOKE_SYNC(path, input, workspace)` | TEXT, JSONB, TEXT -> JSONB | Invoke synchronously in workspace |

## Aggregate Functions

| Function | Signature | Description |
|---|---|---|
| `COUNT(*)` | -> BIGINT | Count all rows |
| `COUNT(expr)` | ANY -> BIGINT | Count non-NULL values |
| `SUM(expr)` | DOUBLE -> DOUBLE? | Sum of values |
| `AVG(expr)` | DOUBLE -> DOUBLE? | Average of values |
| `MIN(expr)` | ANY -> ANY | Minimum value |
| `MAX(expr)` | ANY -> ANY | Maximum value |
| `ARRAY_AGG(expr)` | ANY -> ANY | Collect values into array |

Aggregates support the `FILTER` clause:

```sql
SELECT
  COUNT(*) AS total,
  COUNT(*) FILTER (WHERE node_type = 'cms:Article') AS articles
FROM 'workspace'
```

## Window Functions

Window functions compute values across a set of rows related to the current row.

### Ranking Functions

| Function | Description |
|---|---|
| `ROW_NUMBER()` | Sequential row number within partition |
| `RANK()` | Rank with gaps for ties |
| `DENSE_RANK()` | Rank without gaps |

### Aggregate Window Functions

All aggregate functions (COUNT, SUM, AVG, MIN, MAX) can be used as window functions.

### Syntax

```sql
function() OVER (
  [PARTITION BY expr, ...]
  [ORDER BY expr [ASC|DESC], ...]
  [frame_clause]
)
```

### Frame Clause

```sql
ROWS BETWEEN frame_start AND frame_end
RANGE BETWEEN frame_start AND frame_end
```

Frame bounds: `UNBOUNDED PRECEDING`, `N PRECEDING`, `CURRENT ROW`, `N FOLLOWING`, `UNBOUNDED FOLLOWING`.

### Examples

```sql
-- Number rows within each node type
SELECT
  name,
  node_type,
  ROW_NUMBER() OVER (PARTITION BY node_type ORDER BY created_at) AS rn
FROM 'workspace'

-- Running total of versions
SELECT
  name,
  version,
  SUM(version) OVER (ORDER BY created_at ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running_total
FROM 'workspace'
```

## JOINs

RaisinDB supports the following join types:

| Join Type | Description |
|---|---|
| `INNER JOIN` | Rows matching in both tables |
| `LEFT JOIN` | All rows from left, matching from right |
| `RIGHT JOIN` | All rows from right, matching from left |
| `FULL JOIN` | All rows from both tables |
| `CROSS JOIN` | Cartesian product |

### Examples

```sql
-- Join two workspaces
SELECT a.name, b.name AS related_name
FROM 'content' AS a
INNER JOIN 'metadata' AS b ON a.id = b.properties->>'content_id'::String

-- Left join for optional data
SELECT a.*, b.properties->>'locale'::String AS locale
FROM 'workspace' AS a
LEFT JOIN 'translations' AS b ON a.id = b.properties->>'node_id'::String
```

### Semi-Joins (IN Subqueries)

`IN` and `NOT IN` subqueries are compiled to efficient semi-joins and anti-joins:

```sql
SELECT * FROM 'workspace'
WHERE id IN (SELECT properties->>'target_id'::String FROM 'references')
```

## Subqueries

Subqueries are supported in the following positions:

- **FROM clause** (derived tables): `SELECT * FROM (SELECT ...) AS sub`
- **IN expressions**: `WHERE col IN (SELECT ...)`
- **NOT IN expressions**: `WHERE col NOT IN (SELECT ...)`

## LATERAL Joins

LATERAL joins allow applying a scalar function to each row:

```sql
SELECT a.*, resolved.value
FROM 'workspace' AS a,
LATERAL RESOLVE(a.properties) AS resolved
```

## Graph Queries (SQL/PGQ)

RaisinDB supports graph pattern matching via the SQL/PGQ standard (ISO SQL:2023) using `GRAPH_TABLE`.

### GRAPH_TABLE Syntax

```sql
SELECT * FROM GRAPH_TABLE(
  NODES_GRAPH
  MATCH (a:User)-[:follows]->(b:User)
  WHERE a.name = 'Alice'
  COLUMNS (a.name AS user_name, b.name AS friend_name)
)
```

### Node Patterns

```sql
(n)                                  -- any node
(n:Article)                          -- with label (maps to node_type)
(n:Article|Page)                     -- multiple labels (OR)
(n:User WHERE n.name = 'Alice')      -- with inline filter
```

### Relationship Patterns

```sql
-[r]->                   -- any type, outgoing
-[:follows]->            -- specific type
-[:follows|likes]->      -- multiple types (OR)
<-[r]-                   -- incoming
-[r]-                    -- any direction
-[r:follows*2]->         -- exactly 2 hops
-[r:follows*1..3]->      -- 1 to 3 hops
-[r:follows*]->          -- variable length (1..10 default)
-[r:follows*2..]->       -- 2 to default max (10)
-[r:follows*..5]->       -- 1 to 5 hops
```

### Graph Expressions

Within GRAPH_TABLE, you can use:

- Property access: `n.name`, `n.properties.title`
- JSONPath-style access: `$.friend.properties.email`
- JSON operators: `n.properties->>'title'`
- All comparison, logical, and arithmetic operators
- Functions: `degree(n)`, `shortestPath(a, b)`
- CASE, IN, BETWEEN, LIKE, IS NULL

### Example: Multi-hop Traversal

```sql
SELECT * FROM GRAPH_TABLE(
  NODES_GRAPH
  MATCH (user:User)-[:follows*1..3]->(friend:User)
  WHERE user.id = 'user-123'
  COLUMNS (
    friend.name AS friend_name,
    friend.properties->>'email' AS email
  )
)
```

## DDL Statements (Schema Management)

### CREATE NODETYPE

```sql
CREATE NODETYPE 'myapp:Article'
  EXTENDS 'raisin:Page'
  MIXINS ('myapp:Publishable', 'myapp:SEO')
  DESCRIPTION 'Blog article content type'
  ICON 'article'
  PROPERTIES (
    title String REQUIRED FULLTEXT,
    slug String REQUIRED UNIQUE,
    body String FULLTEXT TRANSLATABLE,
    category String PROPERTY_INDEX,
    tags Array OF String,
    metadata Object {
      author String,
      source URL
    } ALLOW_ADDITIONAL_PROPERTIES,
    featured_image Resource,
    related_article Reference
  )
  ALLOWED_CHILDREN ('myapp:Paragraph', 'myapp:Image')
  COMPOUND_INDEX 'idx_category_created' ON (
    category,
    __created_at DESC
  )
  PUBLISHABLE
  VERSIONABLE;
```

#### Property Types

| Type | Description |
|---|---|
| `String` | Text data |
| `Number` | Numeric values (f64) |
| `Boolean` | True/false |
| `Date` | DateTime (ISO-8601) |
| `URL` | URL strings |
| `Reference` | Cross-node reference |
| `Resource` | File/media with metadata |
| `Object { ... }` | Nested object with inline fields |
| `Array OF Type` | Ordered collection |
| `Composite` | Rich content structure (blocks) |
| `Element` | Single element in composite |
| `NodeType` | Reference to a type definition |

#### Property Modifiers

| Modifier | Description |
|---|---|
| `REQUIRED` | Value must be provided |
| `UNIQUE` | Value must be unique across nodes |
| `FULLTEXT` | Enable Tantivy full-text search index |
| `VECTOR` | Enable HNSW vector embedding index |
| `PROPERTY_INDEX` | Enable RocksDB exact-match index |
| `TRANSLATABLE` | Enable i18n translations |
| `DEFAULT value` | Default value when not provided |
| `LABEL 'text'` | Human-readable label for UI |
| `DESCRIPTION 'text'` | Human-readable description |
| `ORDER N` | Display order hint |
| `ALLOW_ADDITIONAL_PROPERTIES` | For Object types: allow extra fields |

#### NodeType Flags

| Flag | Description |
|---|---|
| `VERSIONABLE` | Enable version history |
| `PUBLISHABLE` | Enable draft/published workflow |
| `AUDITABLE` | Track all changes with user and timestamp |
| `INDEXABLE` | Include in search indexes (default: true) |
| `STRICT` | Reject unknown properties |

### ALTER NODETYPE

```sql
ALTER NODETYPE 'myapp:Article'
  ADD PROPERTY subtitle String FULLTEXT
  DROP PROPERTY legacy_field
  SET DESCRIPTION = 'Updated description';

ALTER NODETYPE 'myapp:Article'
  ADD MIXIN 'myapp:Taggable'
  SET VERSIONABLE TRUE;

ALTER NODETYPE 'myapp:Article'
  MODIFY PROPERTY 'specs.dimensions.width' Number LABEL 'Width (cm)';
```

### DROP NODETYPE

```sql
DROP NODETYPE 'myapp:OldType';
DROP NODETYPE 'myapp:OldType' CASCADE;
```

### CREATE/ALTER/DROP ARCHETYPE

```sql
CREATE ARCHETYPE 'myapp:BlogPost'
  BASE_NODE_TYPE 'myapp:Article'
  DESCRIPTION 'Blog post archetype'
  FIELDS (
    title String REQUIRED,
    body Composite
  );

ALTER ARCHETYPE 'myapp:BlogPost'
  ADD FIELD heading String
  SET DESCRIPTION = 'Updated description';

DROP ARCHETYPE 'myapp:BlogPost';
DROP ARCHETYPE 'myapp:BlogPost' CASCADE;
```

### CREATE/ALTER/DROP ELEMENTTYPE

```sql
CREATE ELEMENTTYPE 'myapp:Paragraph'
  DESCRIPTION 'Rich text paragraph'
  FIELDS (
    text String REQUIRED TRANSLATABLE,
    style String
  );

ALTER ELEMENTTYPE 'myapp:Paragraph'
  ADD FIELD alignment String DEFAULT 'left';

DROP ELEMENTTYPE 'myapp:Paragraph';
DROP ELEMENTTYPE 'myapp:Paragraph' CASCADE;
```

### CREATE/ALTER/DROP MIXIN

Mixins are reusable property sets that can be composed into node types. Under the hood, a mixin is stored as a `NodeType` with `is_mixin: true`, but it has its own dedicated DDL syntax.

#### CREATE MIXIN

```sql
CREATE MIXIN 'myapp:SEO'
  DESCRIPTION 'SEO metadata fields'
  ICON 'search'
  PROPERTIES (
    meta_title String,
    meta_description String,
    og_image URL,
    canonical_url URL
  );

CREATE MIXIN 'myapp:Timestamps'
  DESCRIPTION 'Standard timestamp fields'
  PROPERTIES (
    created_at Date REQUIRED,
    updated_at Date REQUIRED
  );
```

Clauses (all optional, any order):

| Clause | Description |
|---|---|
| `DESCRIPTION 'text'` | Human-readable description |
| `ICON 'name'` | Icon identifier for UI display |
| `PROPERTIES (...)` | Property definitions (same syntax as `CREATE NODETYPE`) |

Once created, a mixin can be referenced in a `CREATE NODETYPE` statement via the `MIXINS` clause:

```sql
CREATE NODETYPE 'myapp:Article'
  MIXINS ('myapp:SEO', 'myapp:Timestamps')
  PROPERTIES (
    title String REQUIRED,
    body String
  );
```

#### ALTER MIXIN

```sql
ALTER MIXIN 'myapp:SEO'
  ADD PROPERTY robots String DEFAULT 'index,follow'
  DROP PROPERTY og_image
  SET DESCRIPTION = 'Updated SEO fields';

ALTER MIXIN 'myapp:Timestamps'
  MODIFY PROPERTY updated_at Date REQUIRED
  SET ICON = 'clock';
```

Supported alterations:

| Alteration | Description |
|---|---|
| `ADD PROPERTY name Type [modifiers]` | Add a new property to the mixin |
| `DROP PROPERTY name` | Remove a property from the mixin |
| `MODIFY PROPERTY name Type [modifiers]` | Replace an existing property definition (or add if not found) |
| `SET DESCRIPTION = 'text'` | Update the mixin description |
| `SET ICON = 'name'` | Update the mixin icon |

`ALTER MIXIN` validates that the target is actually a mixin (has `is_mixin: true`). If you attempt to alter a regular node type with `ALTER MIXIN`, you will receive an error directing you to use `ALTER NODETYPE` instead.

#### DROP MIXIN

```sql
DROP MIXIN 'myapp:SEO';
DROP MIXIN 'myapp:SEO' CASCADE;
```

The optional `CASCADE` keyword indicates that dependent node types should also be updated. Without `CASCADE`, the mixin is removed directly.

## Branch Management

Branches provide Git-like versioning for content.

### CREATE BRANCH

```sql
CREATE BRANCH 'feature/new-layout' FROM 'main'
CREATE BRANCH 'feature/x' FROM 'main' AT REVISION HEAD~2 DESCRIPTION 'Experimental' PROTECTED
CREATE BRANCH 'feature/x' FROM 'main' UPSTREAM 'main' WITH HISTORY
```

### DROP BRANCH

```sql
DROP BRANCH 'feature/old'
DROP BRANCH IF EXISTS 'feature/old'
```

### ALTER BRANCH

```sql
ALTER BRANCH 'feature/x' SET UPSTREAM 'main'
ALTER BRANCH 'feature/x' UNSET UPSTREAM
ALTER BRANCH 'feature/x' SET PROTECTED TRUE
ALTER BRANCH 'feature/x' SET DESCRIPTION 'Updated description'
ALTER BRANCH 'old-name' RENAME TO 'new-name'
```

### MERGE BRANCH

```sql
MERGE BRANCH 'feature/x' INTO 'main'
MERGE BRANCH 'feature/x' INTO 'main' USING FAST_FORWARD
MERGE BRANCH 'feature/x' INTO 'main' USING THREE_WAY MESSAGE 'Merge feature'
MERGE BRANCH 'feature/x' INTO 'main' MESSAGE 'Merge' RESOLVE CONFLICTS (
  ('node-uuid-1', KEEP_OURS),
  ('node-uuid-2', KEEP_THEIRS),
  ('node-uuid-3', 'en', KEEP_THEIRS),
  ('node-uuid-4', USE_VALUE '{"title": "Merged Title"}'),
  ('node-uuid-5', DELETE)
)
```

Merge strategies:
- `FAST_FORWARD` -- Only if target is a direct ancestor of source
- `THREE_WAY` -- Three-way merge with conflict detection (default)

Conflict resolution types:
- `KEEP_OURS` -- Keep the target branch version
- `KEEP_THEIRS` -- Keep the source branch version
- `DELETE` -- Accept deletion
- `USE_VALUE 'json'` -- Use a custom merged value

### USE BRANCH

```sql
USE BRANCH 'feature/x'           -- Set for session
USE LOCAL BRANCH 'feature/x'     -- Set for single statement
SET app.branch = 'feature/x'     -- Alternative syntax
SET LOCAL app.branch = 'feature/x'
```

### Branch Inspection

```sql
SHOW CURRENT BRANCH
SHOW BRANCHES
DESCRIBE BRANCH 'feature/x'
SHOW DIVERGENCE 'feature/x' FROM 'main'
SHOW CONFLICTS FOR MERGE 'feature/x' INTO 'main'
```

## Transaction Control

```sql
BEGIN;
UPDATE nodes SET properties = '{"status": "published"}' WHERE id = 'node-123';
COMMIT WITH MESSAGE 'Published article' ACTOR 'user-456';
```

### Statements

| Statement | Description |
|---|---|
| `BEGIN` or `BEGIN TRANSACTION` | Start a transaction |
| `COMMIT` | Commit with no message |
| `COMMIT WITH MESSAGE 'msg'` | Commit with a descriptive message |
| `COMMIT WITH MESSAGE 'msg' ACTOR 'user'` | Commit with message and user attribution |
| `SET variable = value` | Set session variable within transaction |

## Content Operations

### ORDER (Sibling Reordering)

Reorder nodes among their siblings:

```sql
ORDER 'workspace' SET path='/content/page2' ABOVE path='/content/page1'
ORDER 'workspace' SET path='/content/page2' BELOW path='/content/page1'
ORDER 'workspace' SET id='node-abc' ABOVE id='node-def'
```

### MOVE (Reparenting)

Move a node to a new parent:

```sql
MOVE 'workspace' SET path='/content/old-section/page' TO path='/content/new-section'
MOVE 'workspace' SET id='node-abc' TO id='parent-node-def'
```

### COPY

Copy a node (or subtree) to a new parent:

```sql
COPY 'workspace' SET path='/templates/article' TO path='/content/blog'
COPY 'workspace' SET path='/templates/article' TO path='/content/blog' AS 'my-new-article'
COPY TREE 'workspace' SET path='/templates/section' TO path='/content'
```

### TRANSLATE

Set locale-specific translations for node properties:

```sql
UPDATE 'workspace' FOR LOCALE 'de'
SET properties.title = 'Deutscher Titel',
    properties.description = 'Deutsche Beschreibung'
WHERE path = '/content/article-1';

-- Translate block content
UPDATE 'workspace' FOR LOCALE 'fr'
SET blocks['block-uuid-1'].text = 'Texte en francais'
WHERE path = '/content/article-1';
```

### RELATE / UNRELATE

Create or remove relationships between nodes:

```sql
-- Create a relationship
RELATE FROM default:path='/content/article-1' TO default:path='/content/tag-rust'
  TYPE 'tagged_with' WEIGHT 1.0

-- With explicit workspaces
RELATE FROM workspace1:id='node-abc' TO workspace2:id='node-def'
  TYPE 'references'

-- Remove a relationship
UNRELATE FROM default:path='/content/article-1' TO default:path='/content/tag-rust'
  TYPE 'tagged_with'

-- Remove all relationships between two nodes
UNRELATE FROM default:id='node-abc' TO default:id='node-def'
```

### RESTORE

Restore a node (or subtree) to a previous revision:

```sql
RESTORE NODE path='/content/article-1' TO REVISION 42
RESTORE TREE NODE id='node-abc' TO REVISION HEAD~5
RESTORE NODE path='/content/article-1' TO REVISION 42 TRANSLATIONS ('en', 'de')
```

## Parameter Binding

Use `$1`, `$2`, etc. for parameterized queries. Parameters are 1-indexed.

```sql
SELECT * FROM 'workspace' WHERE properties->>'email'::String = $1
SELECT * FROM 'workspace' WHERE node_type = $1 AND version > $2 LIMIT $3
```

Parameter types are inferred from context:
- Strings are single-quoted
- Numbers are unquoted
- Booleans are `true`/`false`
- NULL is the `NULL` keyword
- JSON arrays/objects are serialized and single-quoted

Parameters can be reused:

```sql
SELECT * FROM 'workspace' WHERE id = $1 OR properties->>'ref_id'::String = $1
```

## Query Optimization

The query planner applies several optimizations automatically:

- **Constant folding** -- Evaluates deterministic expressions at plan time
- **Predicate pushdown** -- Pushes filters closer to the data source
- **Projection pruning** -- Reads only the columns needed
- **Hierarchy rewriting** -- Optimizes PATH/DEPTH function calls into efficient prefix scans
- **Common subexpression elimination** -- Avoids redundant computation

Use `EXPLAIN` to inspect the query plan:

```sql
EXPLAIN SELECT * FROM 'workspace'
  WHERE PATH_STARTS_WITH(path, '/content/')
  AND properties->>'status'::String = 'published'
  ORDER BY created_at DESC
  LIMIT 10
```

## AI & Embedding Configuration

Manage AI providers, embedding configuration, and vector indexes directly via SQL.

### Embedding Configuration

```sql
-- View current embedding configuration
SHOW EMBEDDING CONFIG;

-- Configure embedding provider
ALTER EMBEDDING CONFIG
  SET PROVIDER = 'OpenAI'
  SET MODEL = 'text-embedding-3-small'
  SET API_KEY = 'sk-...'
  SET ENABLED = true;

-- Configure Ollama (local)
ALTER EMBEDDING CONFIG
  SET PROVIDER = 'Ollama'
  SET MODEL = 'nomic-embed-text'
  SET ENABLED = true;

-- Configure Ollama (remote with optional auth)
ALTER EMBEDDING CONFIG
  SET PROVIDER = 'Ollama'
  SET MODEL = 'nomic-embed-text'
  SET BASE_URL = 'https://ollama.mycompany.com'
  SET API_KEY = 'optional-auth-token'
  SET ENABLED = true;

-- Configure Voyage AI
ALTER EMBEDDING CONFIG
  SET PROVIDER = 'Claude'
  SET MODEL = 'voyage-3'
  SET API_KEY = 'pa-...'
  SET ENABLED = true;

-- Disable embeddings
ALTER EMBEDDING CONFIG SET ENABLED = false;

-- Configure max distance threshold for search results
ALTER EMBEDDING CONFIG SET DEFAULT_MAX_DISTANCE = '0.5';

-- Test connection to configured provider
TEST EMBEDDING CONNECTION;
```

**Supported settings for ALTER EMBEDDING CONFIG:**

| Setting | Type | Description |
|---------|------|-------------|
| `PROVIDER` | String | `OpenAI`, `Claude` (Voyage AI), `Ollama`, `HuggingFace` |
| `MODEL` | String | Model identifier (e.g., `text-embedding-3-small`) |
| `API_KEY` | String | Provider API key (encrypted at rest) |
| `BASE_URL` | String | Custom endpoint URL (for remote Ollama) |
| `DIMENSIONS` | Integer | Vector dimensions (auto-set by model) |
| `ENABLED` | Boolean | `true` or `false` |
| `INCLUDE_NAME` | Boolean | Include node name in embedding text |
| `INCLUDE_PATH` | Boolean | Include node path in embedding text |
| `DISTANCE_METRIC` | String | `Cosine`, `L2`, `InnerProduct`, `Hamming` |
| `DEFAULT_MAX_DISTANCE` | String | Maximum distance threshold for search results (default: `0.6`) |
| `MAX_EMBEDDINGS_PER_REPO` | Integer | Limit per repository (empty = unlimited) |

### AI Provider Management

```sql
-- View configured AI providers
SHOW AI PROVIDERS;

-- View full AI configuration
SHOW AI CONFIG;

-- Add/update a provider
ALTER AI CONFIG ADD PROVIDER 'OpenAI'
  SET API_KEY = 'sk-...'
  SET ENABLED = true;

-- Add Ollama with custom endpoint
ALTER AI CONFIG ADD PROVIDER 'Ollama'
  SET ENDPOINT = 'http://gpu-server:11434'
  SET ENABLED = true;

-- Remove a provider
ALTER AI CONFIG DROP PROVIDER 'Ollama';

-- Test a specific provider
TEST AI PROVIDER 'OpenAI';
```

### Vector Index Management

```sql
-- Check vector index health and statistics
SHOW VECTOR INDEX HEALTH;

-- Rebuild HNSW index from stored embeddings
REBUILD VECTOR INDEX;

-- Regenerate all embeddings (re-calls provider API)
REGENERATE EMBEDDINGS;

-- Verify vector index integrity
VERIFY VECTOR INDEX;
```

## Limitations

The following standard SQL features are **not** supported:

- **CREATE TABLE / DROP TABLE** -- Tables are workspaces, managed via the API
- **ALTER TABLE** -- Use DDL statements (CREATE/ALTER NODETYPE) for schema changes
- **Views** -- Not supported
- **Stored procedures / triggers** -- Use serverless functions instead
- **UNION / INTERSECT / EXCEPT** -- Set operations are not supported
- **HAVING** without GROUP BY -- GROUP BY is required for HAVING
- **Recursive CTEs** -- WITH RECURSIVE is not supported
- **Multiple statements per query** -- Use `analyze_batch` or transaction blocks for multi-statement execution
- **TRUNCATE** -- Use DELETE without a WHERE clause
- **GRANT / REVOKE** -- See [SQL Access Control Extensions](../architecture/sql-access-control.md) for RaisinDB's access control SQL syntax
