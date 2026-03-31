# raisin-hnsw

HNSW vector search engine for RaisinDB with Moka LRU cache and cosine distance.

## Overview

This crate provides fast approximate nearest neighbor (ANN) search using the Hierarchical Navigable Small World (HNSW) graph algorithm. Optimized for normalized OpenAI embeddings with cosine distance.

## Features

- **O(log n) Search** - Fast approximate nearest neighbor queries
- **Cosine Distance** - Optimized for normalized vectors (OpenAI embeddings)
- **Memory-Bounded** - Moka LRU cache with configurable size limits
- **Multi-Tenant** - Separate indexes per tenant/repo/branch
- **Persistent** - Periodic snapshots to disk with dirty tracking
- **Crash-Safe** - Graceful shutdown ensures all dirty indexes are saved
- **Chunk-Aware** - Document chunking support with deduplication modes

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    HnswIndexingEngine                        │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │                   Moka LRU Cache                        │ │
│  │   ┌──────────────┐  ┌──────────────┐  ┌──────────────┐ │ │
│  │   │ tenant1/     │  │ tenant2/     │  │ tenant1/     │ │ │
│  │   │ repo1/main   │  │ repo1/main   │  │ repo1/feat   │ │ │
│  │   │ (HnswIndex)  │  │ (HnswIndex)  │  │ (HnswIndex)  │ │ │
│  │   └──────────────┘  └──────────────┘  └──────────────┘ │ │
│  └────────────────────────────────────────────────────────┘ │
│                              │                               │
│                              ▼                               │
│                    ┌──────────────────┐                      │
│                    │  Dirty Tracking  │                      │
│                    │  (HashSet<Key>)  │                      │
│                    └────────┬─────────┘                      │
│                              │                               │
│                              ▼                               │
│                    ┌──────────────────┐                      │
│                    │ Snapshot Task    │ (60s interval)       │
│                    │ → bincode files  │                      │
│                    └──────────────────┘                      │
└─────────────────────────────────────────────────────────────┘
```

## Usage

### Basic Search

```rust
use raisin_hnsw::HnswIndexingEngine;
use raisin_hlc::HLC;
use std::path::PathBuf;
use std::sync::Arc;

// Create engine with 2GB cache, 1536 dimensions (OpenAI)
let engine = Arc::new(HnswIndexingEngine::new(
    PathBuf::from("./.data/hnsw"),
    2 * 1024 * 1024 * 1024,  // 2GB cache
    1536                      // OpenAI ada-002 dimensions
)?);

// Start periodic snapshot task
let snapshot_handle = engine.start_snapshot_task();

// Add embedding
engine.add_embedding(
    "tenant1",
    "repo1",
    "main",
    "workspace1",
    "node-123",
    HLC::new(1, 0),
    embedding_vector
)?;

// Search for similar vectors
let results = engine.search(
    "tenant1",
    "repo1",
    "main",
    Some("workspace1"),  // Optional workspace filter
    &query_vector,
    10  // k results
)?;

// Graceful shutdown
engine.shutdown().await?;
snapshot_handle.abort();
```

### Chunk-Aware Search

For documents split into multiple chunks:

```rust
use raisin_hnsw::{SearchRequest, SearchMode, ScoringConfig};

// Add chunked document
engine.add_embedding("t1", "r1", "main", "ws1", "doc1#0", hlc1, chunk0_vec)?;
engine.add_embedding("t1", "r1", "main", "ws1", "doc1#1", hlc2, chunk1_vec)?;
engine.add_embedding("t1", "r1", "main", "ws1", "doc1#2", hlc3, chunk2_vec)?;

// Search all chunks
let request = SearchRequest::new(query, 10)
    .with_mode(SearchMode::Chunks)
    .with_workspace("ws1".to_string());
let chunks = engine.search_chunks("t1", "r1", "main", &request)?;

// Search documents (deduplicated, one result per source doc)
let request = SearchRequest::new(query, 10)
    .with_mode(SearchMode::Documents)
    .with_max_distance(0.4);  // Stricter threshold
let docs = engine.search_documents("t1", "r1", "main", &request)?;
```

### Position-Based Scoring

Prioritize earlier chunks (often contain summaries/introductions):

```rust
let scoring = ScoringConfig {
    position_decay: 0.1,      // 10% decay per chunk position
    first_chunk_boost: 1.2,   // 20% boost for first chunk
    exact_match_boost: 1.0,   // Reserved for future use
};

let request = SearchRequest::new(query, 10)
    .with_scoring(scoring);
```

### Vector Normalization

```rust
use raisin_hnsw::normalize_vector;

// Normalize vector to unit length for cosine distance
let normalized = normalize_vector(&raw_embedding);
```

## Distance Metrics

Uses **cosine distance** for normalized vectors:

| Distance | Cosine Similarity | Interpretation |
|----------|------------------|----------------|
| 0.0-0.2  | 0.80-1.00        | Very similar (highly relevant) |
| 0.2-0.4  | 0.60-0.80        | Similar (relevant) |
| 0.4-0.6  | 0.40-0.60        | Weakly related (possibly relevant) |
| 0.6+     | < 0.40           | Not related (filtered out by default) |

Why cosine distance instead of L2:
- OpenAI embeddings are pre-normalized to unit length
- Better semantic differentiation in high dimensions (3072D)
- Faster computation (no sqrt, no squaring)

## Search Modes

| Mode | Description | Use Case |
|------|-------------|----------|
| `SearchMode::Chunks` | Returns all matching chunks | RAG context retrieval |
| `SearchMode::Documents` | Best chunk per document | Document recommendations |

## Modules

| Module | Description |
|--------|-------------|
| `engine.rs` | `HnswIndexingEngine` - LRU cache, dirty tracking, snapshots |
| `index.rs` | `HnswIndex` - instant-distance wrapper with persistence |
| `types.rs` | Result types, search modes, scoring config |
| `excerpt.rs` | `ExcerptFetcher` trait for text retrieval |

## Configuration

| Setting | Default | Description |
|---------|---------|-------------|
| `cache_size` | - | Maximum cache size in bytes |
| `dimensions` | - | Vector dimensionality (e.g., 1536 for ada-002) |
| `MAX_DISTANCE` | 0.6 | Default distance threshold for filtering |
| Snapshot interval | 60s | Background save frequency |

## Branch Operations

Supports Git-like branch semantics:

```rust
// Copy index when creating a feature branch
engine.copy_for_branch("tenant1", "repo1", "main", "feature")?;
```

## Persistence

- **Format**: bincode serialization
- **Path**: `{base_path}/{tenant}/{repo}/{branch}.hnsw`
- **Dirty tracking**: Only modified indexes are saved
- **Graceful shutdown**: Ensures all changes are persisted

## Crate Usage

Used by:
- `raisin-core` - Embedding index management
- `raisin-transport-http` - Vector search API endpoints

## Dependencies

- `instant-distance` - HNSW graph implementation
- `moka` - High-performance LRU cache
- `bincode` - Binary serialization
- `raisin-hlc` - Hybrid Logical Clock timestamps

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
