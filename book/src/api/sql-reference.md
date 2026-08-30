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

#### Search Table Functions: HYBRID_SEARCH, FULLTEXT_SEARCH, KNN

```
HYBRID_SEARCH  ( query [, limit] [, workspace] [, named ...] )
FULLTEXT_SEARCH( query ,  language            [, named ...] )
KNN            ( query [, limit]              [, named ...] )
```

All three take the **same** named arguments, return the **same** columns, and
apply the **same** row-level security. `KNN` is `HYBRID_SEARCH` with the
full-text leg switched off.

**The workspace scope is required.** It is part of the query text, so you can
always read the corpus off the statement:

```sql
-- one workspace
SELECT node_id, path, score
FROM   HYBRID_SEARCH('vector index rebuild', 10, workspaces => 'library');

-- the same thing; the third positional is kept forever
SELECT node_id, path, score
FROM   HYBRID_SEARCH('vector index rebuild', 10, 'library');

-- several, or a family
SELECT * FROM HYBRID_SEARCH('retention policy', 10,
                            workspaces => 'library, handbook, policies');
SELECT * FROM FULLTEXT_SEARCH('rollback', 'en',
                              workspaces => 'content-*', limit => 50);

-- every workspace you may read: the recommended RAG form
SELECT node_id, workspace_id, path, properties, score
FROM   HYBRID_SEARCH('Wie baue ich einen Vektorindex neu auf?', 20,
                     workspaces => 'ALL READABLE', language => 'de');
```

`HYBRID_SEARCH(query, k)` and `FULLTEXT_SEARCH(query, language)` used to search
**every workspace in the repository** -- undocumented behaviour that returned
build artifacts and binary assets to callers asking for documents. They now fail
with an error naming the fix.

| named argument | type | default | applies to |
|---|---|---|---|
| `workspaces` | TEXT (grammar below) | **required** | all three |
| `limit` | INT, 1..=1000 | 10 / 100 (fulltext) / 10 | all three |
| `language` | TEXT, ISO 639-1 | the repository default | hybrid, fulltext |
| `vector_weight` | DOUBLE, >= 0.0 | 1.0 | hybrid |
| `fulltext_weight` | DOUBLE, >= 0.0 | 1.0 | hybrid |
| `max_distance` | DOUBLE, (0.0, 2.0] | 0.6 | hybrid, knn |

`workspaces` accepts exactly five forms: one name (`'library'`), a
comma-separated set (`'library, handbook, policies'`), a glob (`'content-*'`),
or `'ALL READABLE'`. `'*'` and a bare `'ALL'` are errors -- there is exactly one
spelling for "broad", so an operator can grep for it. Naming a workspace is an
assertion: a name that does not exist, or that you may not read, is an error
(with the same message for both, deliberately). A glob that matches fewer
workspaces is not.

A **weight of 0 skips that leg entirely**, including the embedding round trip.
`vector_weight => 0` therefore works on a tenant with no embedding provider
configured, and `fulltext_weight => 0` is what a cross-lingual query wants: a
German query against an English corpus gets only noise from the lexical leg, and
RRF fuses that noise at full weight.

```sql
-- cross-lingual: kill the noisy lexical leg and widen the distance cutoff
SELECT node_id, workspace_id, path, vector_distance
FROM   HYBRID_SEARCH('Wie baue ich einen Vektorindex neu auf?', 20,
                     workspaces => 'ALL READABLE',
                     fulltext_weight => 0, max_distance => 0.9);
```

`language` takes ISO 639-1 codes (`'en'`, not `'english'`) -- the index stores
two-letter codes, so anything longer matches no documents.

The RRF constant `k` is fixed at 60 and is not exposed: it is global, so varying
it per query makes two callers' scores incomparable, and tuning it hides real
faults (a dead vector leg looks like it improves when you lower k).

### Which embedding space: `kind =>`

Vectors are stored per *space*: a text tower and an image tower are separate
indexes built by separate models, and `kind` selects which the vector leg reads.

| value | meaning |
|---|---|
| `'text'` | text vectors only. **The default.** |
| `'image'` | image vectors only |
| `'all'` | every configured space, rank-fused |

```sql
-- documents only (the default; the argument is optional)
SELECT node_id, path, vector_distance
FROM   KNN('quarterly revenue', 10, workspaces => 'ALL READABLE');

-- pictures only
SELECT node_id, path, embedding_kind
FROM   KNN('a red bicycle', 10, workspaces => 'assets', kind => 'image');

-- both, fused into one ranking
SELECT node_id, path, embedding_kind, vector_rank
FROM   HYBRID_SEARCH('a red bicycle', 10,
                     workspaces => 'ALL READABLE', kind => 'all');
```

`kind` defaults to `'text'` rather than `'all'` deliberately: were it `'all'`,
the day an image tower first writes a vector every existing query would silently
start fusing a second corpus in -- new rows in every result set, no error, and a
`LIMIT 10` spending slots on pictures. Breadth is opt-in and says so in the query
text, exactly as `workspaces => 'ALL READABLE'` is.

`FULLTEXT_SEARCH` rejects `kind`: it has no vector leg.

Each space becomes its own leg and they are fused **by rank**, never by
distance. Two towers' distances are not comparable — a cosine distance of 0.31
from a text model and 0.31 from an image model are not the same quantity — so a
document found by both scores above one found by either alone, and the reported
`vector_distance` is the one belonging to the best-*ranked* leg. `embedding_kind`
says which leg that was, so you always know which scale the distance is on.

### Search by reference: `VECTOR_OF(...)` and a bound vector

`KNN` argument 1 accepts five forms. The last two answer "find things like this
one" without any text at all.

```sql
KNN('some text')                       -- embedded with the tenant's provider
KNN(EMBEDDING('some text'))            -- identical; the wrapper is unwrapped
KNN(ARRAY[0.1, 0.2, ...])              -- a vector literal
KNN('[0.1, 0.2, ...]')                 -- a BOUND vector (see below)
KNN(VECTOR_OF('assets:/photos/cat.jpg'))   -- a node's own stored vector
```

#### `VECTOR_OF(node_ref [, chunk_index])`

Reads the node's **stored** vector out of `cf::EMBEDDINGS` rather than
recomputing it.

```sql
-- other pictures like this one
SELECT node_id, path, vector_distance
FROM   KNN(VECTOR_OF('assets:/photos/cat.jpg'), 10,
           workspaces => 'assets', kind => 'image');

-- by node id instead of path, and a named embedding spec
SELECT * FROM KNN(VECTOR_OF('assets:0193f2ab-...'), 10, workspaces => 'assets');
SELECT * FROM KNN(VECTOR_OF('library:/manuals/boiler#doc'), 10,
                  workspaces => 'library');
```

The `workspace:` prefix is **required**, exactly as for
`REFERENCES('workspace:/path')`: embeddings are keyed by the workspace the
*node* lives in, which need not be one of the workspaces being searched, so it
cannot be inferred — least of all under `workspaces => 'ALL READABLE'`.

Reading beats re-encoding on four counts. There is no provider call, so no
latency, cost or rate limit on a query. It works when the encoder is offline —
which, for an image tower hosted outside the database, is exactly when you need
it. For an image there is nothing to re-encode *from*: the pixels went through a
model this process does not host. And it **cannot drift**: a re-encode is only
comparable to the index if the query-time pipeline reproduces the index-time one
exactly (same model, same revision, same field selection, same chunker, same
normalisation), and when one of those moves the re-encoded vector lands
elsewhere in the same space — every distance still finite, every ranking still
plausible, nothing logged.

**The reference node is excluded from its own results.** Its vector is at
distance 0 from itself, so without this it is rank 1 in every such query and
`LIMIT 10` spends a slot restating the question. The drop happens in the same
pass as the RLS and `WHERE` drops, and the legs are already drawn wider than
`limit`, so it does not cost you a row.

**The reference node is permission-checked.** It goes through the same RLS pass
every hit does; a node you may not read reports "no such node" rather than
handing you its neighbourhood.

**Chunking is explicit, never guessed.** An asset has one vector and the
question has an answer. A chunked document has several, and they are different
vectors: chunk 0 is merely the paragraph that came first, and a centroid of a
multi-topic document is a point resembling none of its parts. So a source with
one stored chunk resolves; a source with more is an **error** naming the count,
and you say which:

```sql
-- ERROR: ... has 7 stored chunks in partition '...' (indexes 0, 1, ...), so
-- 'similar to it' is ambiguous ... Name the chunk, e.g. VECTOR_OF('...', 0).
SELECT * FROM KNN(VECTOR_OF('library:/manuals/boiler'), 10,
                  workspaces => 'library');

-- explicit, and therefore fine
SELECT * FROM KNN(VECTOR_OF('library:/manuals/boiler', 2), 10,
                  workspaces => 'library');
```

Under `kind => 'all'` the lookup is **per space**: the text index is searched
with the node's text vector and the image index with its image vector. A space
holding no vector for that node contributes no leg (logged at INFO); if none
does, the statement fails naming the node and the spaces rather than returning
an empty result that looks like an empty corpus.

`VECTOR_OF` is `KNN`-only. `HYBRID_SEARCH` needs text for its full-text half,
and accepting a vector there would be a vector-only search reported as hybrid.

#### Binding a vector as a parameter

The realistic way to search by a vector computed elsewhere — an image encoder
outside the database — is to **bind** it:

```js
await raisin.sql.query(
  `SELECT node_id, path, vector_distance
     FROM KNN($1, 10, workspaces => 'assets', kind => 'image')`,
  [embeddingFrom1024DimImageEncoder]   // a plain array of numbers
);
```

`$1` is substituted into the SQL text before it is parsed, and the two
substituters in this tree render a JSON array two ways: the HTTP and pgwire path
produces the quoted string `'[0.1,0.2,...]'` (pgvector's own literal form), the
functions runtime produces `ARRAY[0.1, 0.2, ...]`. `KNN` accepts both, so the
same parameter means the same thing from either surface.

The text form costs one ambiguity: a `KNN` whose search *text* is literally
`'[1, 2, 3]'` is read as a 3-dimensional vector. That trade is deliberate,
because the two failure modes are not symmetric — treating a bound vector as
text is silent and unfixable from outside (it was the previous behaviour: the
string `"[0.1,0.2,...]"` went to the embedding provider and the result ranked
plausibly), while treating a bracketed phrase as a vector fails loudly on the
first dimension check. Recognition is strict: brackets, at least one element,
and every element a finite number. `'[a, b]'`, `'[]'` and `'[draft] notes'` stay
text.

**Returned columns** (identical for all three, in this order):
`node_id`, `workspace_id`, `name`, `path`, `node_type`, `score`,
`fulltext_rank`, `vector_rank`, `vector_distance`, `chunk_index`,
`embedding_kind`, `revision`, `created_at`, `updated_at`, `properties`. Rank,
distance and kind columns are NULL when that leg did not contribute -- including
when its weight was 0.

**The universe is an argument; everything else is `WHERE`.**

```sql
SELECT node_id, workspace_id, path, score
FROM   HYBRID_SEARCH('retention policy', 10, workspaces => 'ALL READABLE')
WHERE  node_type = 'myapp:Article';
```

`workspaces` is an argument because it changes what top-k *means*:
`workspaces => 'a, b' LIMIT 10` returns the ten best rows in a and b, while
`'ALL READABLE' ... WHERE workspace_id IN ('a','b') LIMIT 10` returns whichever
of a's and b's rows survived the *global* best-N -- which can be empty while
matching documents exist. There is no `node_types =>` and no `path_prefix =>`
argument; those are columns, and they are filtered with `WHERE`.

The function's own `limit` is **retrieval depth, not a display cap**:
`HYBRID_SEARCH('q', 10, ...) LIMIT 100` returns 10. Ask the function for 100 to
get 100. Permission filtering happens *before* the count, so a restricted caller
asking for 10 receives up to 10 -- not "however many of the global top 10 they
happened to be allowed to see".

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

Set locale-specific translations for node properties.

The `SET` path is relative to the node's **properties** — write `title`, not
`properties.title`, which addresses a `properties` key inside them and resolves to
nothing. Array items are addressed by their `uuid`, at any depth:

```sql
UPDATE 'workspace' FOR LOCALE 'de'
SET title = 'Deutscher Titel',
    description = 'Deutsche Beschreibung'
WHERE path = '/content/article-1';

-- Translate block content: field[uuid='…'], not field['…']
UPDATE 'workspace' FOR LOCALE 'fr'
SET blocks[uuid='block-uuid-1'].text = 'Texte en francais',
    sections[uuid='s1'].features[uuid='f1'].title = 'Développement rapide'
WHERE path = '/content/article-1';
```

A write **merges** into whatever the locale already holds, so translating a
document one statement at a time accumulates rather than replacing. Assign `NULL`
to remove a single translated field — it then falls back to the base language
again; use the `raisin:cmd/delete-translation` command to drop a whole locale:

```sql
UPDATE 'workspace' FOR LOCALE 'de' SET title = NULL WHERE path = '/content/article-1';
```

Read it back with a `locale` predicate — `SELECT … WHERE locale = 'de'` — which
resolves the overlay recursively, exactly as the REST `?lang=de` read does.

A `SET` path is not validated against the node's content: a `uuid` that does not
exist is stored and silently skipped at read time, while the statement still
reports the node as affected. Verify a translation by reading it back.

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
