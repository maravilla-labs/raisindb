# Indexing and Full-Text Search

The `raisin-indexer` crate provides a pluggable indexing framework for RaisinDB, including property indexes for efficient lookups and full-text search powered by Tantivy.

## Architecture

```
┌─────────────────────────────────────────────┐
│              Indexing Framework              │
│                                             │
│  ┌─────────────────┐  ┌─────────────────┐  │
│  │  Property Index  │  │   Full-Text     │  │
│  │     Plugin       │  │  Search (FTS)   │  │
│  └────────┬────────┘  └────────┬────────┘  │
│           └──────────┬─────────┘            │
│                      ▼                      │
│           ┌──────────────────┐              │
│           │  Index Manager   │              │
│           └────────┬─────────┘              │
│                    ▼                        │
│           ┌──────────────────┐              │
│           │  Event Handler   │              │
│           └────────┬─────────┘              │
│                    ▼                        │
│           ┌──────────────────┐              │
│           │  Background      │              │
│           │  Worker          │              │
│           └──────────────────┘              │
└─────────────────────────────────────────────┘
```

## Property Indexes

The `PropertyIndexPlugin` maintains secondary indexes on node properties, enabling efficient lookups without full workspace scans:

```rust
use raisin_indexer::PropertyIndexPlugin;

let plugin = PropertyIndexPlugin::new(storage.clone());

// Property indexes are updated automatically via the event handler
// when nodes are created or modified
```

Property indexes are used by the SQL engine's physical planner to select efficient scan strategies (index lookup instead of table scan).

## Compound Indexes

Compound indexes (also called composite or multi-column indexes) allow efficient queries on combinations of properties. Instead of scanning every node in a workspace, the query engine performs a prefix scan on the index, making queries like `WHERE category = 'tech' AND status = 'published' ORDER BY __created_at DESC LIMIT 20` run in O(LIMIT) time.

### Defining Compound Indexes

Compound indexes are defined on node types using the `COMPOUND_INDEX` clause in `CREATE NODETYPE`:

```sql
CREATE NODETYPE 'myapp:Article'
  PROPERTIES (
    title String NOT NULL,
    category String,
    status String DEFAULT 'draft',
    priority Integer
  )
  COMPOUND_INDEX 'idx_category_status_created' ON (
    category,
    status,
    __created_at DESC
  )
  COMPOUND_INDEX 'idx_status_priority' ON (
    status,
    priority DESC
  )
```

Each index has:
- A **unique name** (e.g., `idx_category_status_created`)
- An ordered list of **columns** with optional sort direction (`ASC` or `DESC`)

### Column Types

Column types are automatically inferred from the property definition:

| Column Type | Encoding | Properties |
|-------------|----------|------------|
| `String` | Null-byte delimited (lexicographic order) | String properties |
| `Integer` | Big-endian i64 (numeric order) | Integer properties |
| `Timestamp` | Special encoding for sort direction | `__created_at`, `__updated_at` |
| `Boolean` | Single byte (0 or 1) | Boolean properties |

**Timestamp encoding:** `DESC` timestamps use bitwise NOT of the microsecond value, so the most recent entries appear first in a natural forward scan. `ASC` timestamps use direct big-endian encoding.

### System Properties

In addition to user-defined properties, compound indexes can include system properties as columns:

| System Property | Type | Description |
|----------------|------|-------------|
| `__node_type` | String | The node's type (e.g., `myapp:Article`) |
| `__created_at` | Timestamp | Node creation time |
| `__updated_at` | Timestamp | Last modification time |

This enables cross-type indexes. For example, to query all content types sorted by creation date:

```sql
CREATE NODETYPE 'myapp:Content'
  PROPERTIES (
    workspace_id String
  )
  COMPOUND_INDEX 'idx_ws_type_created' ON (
    workspace_id,
    __node_type,
    __created_at DESC
  )
```

### How Queries Use Compound Indexes

The SQL query planner automatically matches compound indexes to query predicates. The matching algorithm works on **equality prefix + optional trailing ORDER BY**:

```sql
-- Given index: idx_category_status_created ON (category, status, __created_at DESC)

-- Uses index: equality on leading columns + ORDER BY on trailing column
SELECT * FROM 'default'
WHERE properties->>'category'::String = 'tech'
  AND properties->>'status'::String = 'published'
ORDER BY __created_at DESC
LIMIT 20

-- Uses index: partial prefix (category only)
SELECT * FROM 'default'
WHERE properties->>'category'::String = 'tech'

-- Cannot use index: skips leading column
SELECT * FROM 'default'
WHERE properties->>'status'::String = 'published'
```

The key rule: **equality conditions must match a prefix of the index columns, in order.** The last matched column can optionally be used for ORDER BY instead of equality.

### Query Optimization Example

Without a compound index, this query scans every node:

```sql
SELECT * FROM 'default'
WHERE properties->>'category'::String = 'tech'
  AND properties->>'status'::String = 'published'
ORDER BY __created_at DESC
LIMIT 10
```

With `COMPOUND_INDEX 'idx' ON (category, status, __created_at DESC)`, the query becomes:
1. Seek to the index prefix `category=tech, status=published`
2. Read 10 entries (already sorted by `__created_at DESC`)
3. Look up the 10 nodes by ID

This is O(LIMIT) instead of O(total nodes).

### Background Index Building

When you add a compound index to a node type that already has data, RaisinDB automatically schedules a background job to build the index:

- Scans all existing nodes of the target node type
- Processes nodes in batches of 1,000
- Skips nodes where required index columns are missing
- New nodes created during the build are indexed inline (no data loss)

The job runs through the unified job queue and can be monitored like any other background job.

### Draft vs. Published Spaces

Compound indexes maintain separate entries for draft and published node states. The index key includes a space marker (`cidx` for draft, `cidx_pub` for published), ensuring that queries against published content only return published nodes.

### MVCC and Versioning

Compound index entries are revision-aware:
- Each entry includes the HLC revision in the key (using descending encoding so the latest revision comes first)
- Deleted nodes are tracked with tombstone markers
- Index scans deduplicate by node ID, returning only the latest non-tombstoned entry

This ensures correct results even during concurrent writes and branch operations.

### Practical Examples

**Social feed index** -- show the latest posts by a user:

```sql
CREATE NODETYPE 'social:Post'
  PROPERTIES (
    author_id String NOT NULL,
    visibility String DEFAULT 'public'
  )
  COMPOUND_INDEX 'idx_author_feed' ON (
    author_id,
    visibility,
    __created_at DESC
  )
```

```sql
-- Get latest 20 public posts by user
SELECT * FROM 'default'
WHERE properties->>'author_id'::String = 'user-123'
  AND properties->>'visibility'::String = 'public'
ORDER BY __created_at DESC
LIMIT 20
```

**E-commerce catalog** -- browse products by category with price sorting:

```sql
CREATE NODETYPE 'shop:Product'
  PROPERTIES (
    category String NOT NULL,
    in_stock Boolean DEFAULT true,
    price Integer
  )
  COMPOUND_INDEX 'idx_category_stock_price' ON (
    category,
    in_stock,
    price ASC
  )
```

```sql
-- Cheapest in-stock electronics
SELECT * FROM 'default'
WHERE properties->>'category'::String = 'electronics'
  AND properties->>'in_stock'::Boolean = true
ORDER BY properties->>'price'::Integer ASC
LIMIT 50
```

## Full-Text Search

### Tantivy Engine

RaisinDB uses [Tantivy](https://github.com/quickwit-oss/tantivy) as its full-text search engine. The `TantivyIndexingEngine` manages per-workspace indexes with language-aware tokenization:

```rust
use raisin_indexer::{TantivyIndexingEngine, BatchIndexContext};

let engine = TantivyIndexingEngine::new(index_path)?;

// Index a batch of documents
let ctx = BatchIndexContext {
    tenant: "tenant1",
    repo: "repo1",
    branch: "main",
    workspace: "default",
};
engine.index_batch(&ctx, &documents).await?;
```

### Search

```rust
use raisin_indexer::IndexQuery;

let query = IndexQuery {
    text: "content management".to_string(),
    workspace: "default".to_string(),
    limit: 20,
    // ...
};

let results = engine.search(&query).await?;
```

### Full-Text Search via SQL

Full-text search is integrated into the SQL engine:

```sql
SELECT id, name, __score
FROM 'default'
WHERE FULLTEXT_SEARCH(properties, 'content management')
ORDER BY __score DESC
LIMIT 20
```

## Event-Driven Indexing

The `FullTextEventHandler` listens for node change events and automatically updates indexes:

```rust
use raisin_indexer::FullTextEventHandler;

let handler = FullTextEventHandler::new(engine.clone());
// Register with the event system
event_bus.subscribe(handler);
```

When a node is created, updated, or deleted, the event handler enqueues an indexing job.

## Background Worker

The `IndexerWorker` processes indexing jobs in the background with configurable batch sizes and intervals:

```rust
use raisin_indexer::{IndexerWorker, WorkerConfig};

let config = WorkerConfig {
    batch_size: 100,
    poll_interval_ms: 1000,
    // ...
};

let worker = IndexerWorker::new(config, engine, storage);
worker.start().await;
```

## Index Cache

The `IndexCacheConfig` controls memory usage for index caches:

```rust
use raisin_indexer::IndexCacheConfig;

let cache_config = IndexCacheConfig {
    max_entries: 10_000,
    // ...
};
```

Indexes use Moka's synchronous LRU cache for efficient memory management.

## Tantivy Management

The `TantivyManagement` API provides administrative operations:

- List all indexes
- Get index statistics (document count, size)
- Rebuild indexes from scratch
- Delete indexes for specific workspaces
