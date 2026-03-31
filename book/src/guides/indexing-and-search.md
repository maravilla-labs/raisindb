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
