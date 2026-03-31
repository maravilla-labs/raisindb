# raisin-indexer

Pluggable indexing framework for RaisinDB with full-text search and property indexing.

## Overview

This crate provides indexing capabilities for efficient property lookups, full-text search, and query patterns that would otherwise require full workspace scans. It features a multi-tenant, branch-aware architecture with Tantivy-powered full-text search.

## Features

- **Pluggable Index Architecture** - Register custom index plugins via `IndexPlugin` trait
- **Full-Text Search (Tantivy)** - Multi-language, branch-aware full-text indexing
- **Property Indexing** - O(1) property value lookups with in-memory indexes
- **Schema-Driven Indexing** - Index only properties marked with `Fulltext` in node type schema
- **Background Worker** - Async job processing with configurable batch sizes
- **Event-Driven Updates** - Automatic index updates on node create/update/delete
- **LRU Caching** - Memory-bounded index cache with size-based eviction

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Event Sources                         │
├─────────────────────────────────────────────────────────────┤
│  Node Created   │   Node Updated   │   Node Deleted         │
└────────┬────────┴────────┬─────────┴────────┬───────────────┘
         └─────────────────┼──────────────────┘
                           │
                           ▼
               ┌───────────────────────┐
               │  FullTextEventHandler │
               │  (enqueues jobs)      │
               └───────────┬───────────┘
                           │
                           ▼
               ┌───────────────────────┐
               │     Job Queue         │
               │  (persistent store)   │
               └───────────┬───────────┘
                           │
                           ▼
               ┌───────────────────────┐
               │    IndexerWorker      │
               │  (background task)    │
               └───────────┬───────────┘
                           │
            ┌──────────────┼──────────────┐
            ▼              ▼              ▼
      ┌──────────┐  ┌──────────────┐  ┌───────────┐
      │ Tantivy  │  │  Property    │  │  Custom   │
      │ Engine   │  │  Index       │  │  Plugin   │
      └──────────┘  └──────────────┘  └───────────┘
```

## Usage

### Full-Text Search Setup

```rust
use raisin_indexer::{TantivyIndexingEngine, IndexerWorker, WorkerConfig, IndexCacheConfig};
use std::sync::Arc;
use std::path::PathBuf;

// Create indexing engine with cache configuration
let cache_config = IndexCacheConfig::production();
let engine = Arc::new(TantivyIndexingEngine::new(
    PathBuf::from("/data/indexes"),
    cache_config.fulltext_cache_size
)?);

// Create and start background worker
let worker = IndexerWorker::new(storage, engine, WorkerConfig::default());
let handle = worker.start();

// Stop gracefully when done
worker.stop().await;
handle.await??;
```

### Event Handler Registration

```rust
use raisin_indexer::FullTextEventHandler;
use raisin_events::EventBus;

let handler = FullTextEventHandler::new(storage.clone());
event_bus.register(Arc::new(handler));

// Node events will now automatically enqueue indexing jobs
```

### Custom Index Plugin

```rust
use raisin_indexer::{IndexPlugin, IndexQuery, IndexManager};
use raisin_events::EventHandler;

struct MyCustomIndex { /* ... */ }

impl EventHandler for MyCustomIndex {
    fn handle(&self, event: &Event) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        // Handle node events to update index
    }
    fn name(&self) -> &str { "my_custom_index" }
}

impl IndexPlugin for MyCustomIndex {
    fn index_name(&self) -> &str { "my_index" }

    fn query(&self, query: IndexQuery) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send>> {
        // Return matching node IDs
    }

    fn supports_query(&self, query: &IndexQuery) -> bool {
        matches!(query, IndexQuery::FindByProperty { .. })
    }
}

// Register with manager
let manager = IndexManager::new();
manager.register_plugin(Arc::new(MyCustomIndex::new()));
```

### Index Management Operations

```rust
use raisin_indexer::TantivyManagement;

let management = TantivyManagement::new(base_path, engine);

// Verify index integrity
let report = management.verify_index("tenant", "repo", "branch").await?;
println!("Health: {:.2}%", report.health_score * 100.0);

// Optimize by merging segments
let stats = management.optimize_index("tenant", "repo", "branch").await?;
println!("Merged {} segments, saved {} bytes",
    stats.segments_merged,
    stats.bytes_before - stats.bytes_after);

// Get health metrics
let health = management.get_health("tenant", "repo", "branch").await?;
println!("Disk usage: {} bytes, {} entries",
    health.disk_usage_bytes,
    health.entry_count);
```

## Query Types

| Query | Description |
|-------|-------------|
| `FindByProperty` | Exact property value match |
| `FindByPropertyJson` | Property match using JSON comparison |
| `FindNodesWithProperty` | Find nodes that have a specific property |
| `FindByPropertyRange` | Numeric property range queries |
| `FindByDateRange` | Date property range queries |
| `FindReferences` | Find nodes referencing a target node |
| `FullTextSearch` | Full-text search queries |

## Modules

| Module | Description |
|--------|-------------|
| `tantivy_engine` | Tantivy-based full-text indexing engine |
| `worker` | Background worker for job processing |
| `event_handler` | Event handler for automatic index updates |
| `manager` | Index manager for coordinating plugins |
| `plugin` | `IndexPlugin` trait definition |
| `property_index` | In-memory property index plugin |
| `query` | Index query type definitions |
| `config` | Cache configuration |
| `management` | Index management operations (verify, rebuild, optimize) |

## Index Organization

Full-text indexes are stored at: `{base_path}/{tenant_id}/{repo_id}/{branch}/`

Each branch maintains a separate Tantivy index for:
- Efficient branch operations (copy-on-write)
- Isolated search contexts
- Point-in-time queries via HLC revision filtering

## Cache Configuration

| Preset | Fulltext Cache | Hot Indexes |
|--------|----------------|-------------|
| `development()` | 256MB | ~8 indexes |
| `production()` | 1GB | ~34 indexes |

Memory estimation: ~30MB per Tantivy index

## Worker Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `batch_size` | 10 | Jobs per iteration |
| `poll_interval` | 1s | Time between polls |
| `max_retries` | 3 | Retry attempts for failed jobs |

## Multi-Language Support

Supported languages with stemming:
- English, German, French, Spanish, Italian, Portuguese
- Russian, Arabic, Danish, Dutch, Finnish
- Hungarian, Norwegian, Romanian, Swedish, Turkish

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
