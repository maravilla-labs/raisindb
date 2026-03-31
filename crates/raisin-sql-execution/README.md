# raisin-sql-execution

Physical execution engine for RaisinDB SQL, Cypher, and PGQ queries.

## Overview

This crate provides the physical execution layer that bridges logical query plans with storage operations. It converts optimized logical plans into physical operators and executes them in an async streaming fashion.

- **Physical Plan Execution**: Volcano-style iterator model with async/await support
- **Multi-Query Language**: SQL, Cypher, and PGQ (ISO SQL:2023 Part 16) execution
- **Intelligent Scan Selection**: Automatic index selection based on query predicates
- **Expression Evaluation**: 50+ built-in functions (string, numeric, JSON, geospatial, etc.)
- **Batch Processing**: Columnar data format for vectorized operations

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Query Engine                           │
│  (high-level API: parse → analyze → optimize → execute)     │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                    Physical Planner                         │
│  - Converts LogicalPlan → PhysicalPlan                      │
│  - Selects optimal scans (Table, Index, FullText, Vector)   │
│  - Chooses join strategies (Hash, NestedLoop, IndexLookup)  │
└──────────────────────────┬──────────────────────────────────┘
                           │
        ┌──────────────────┼──────────────────┐
        ▼                  ▼                  ▼
┌──────────────┐  ┌──────────────┐  ┌──────────────┐
│  SQL Executor │  │Cypher Engine │  │  PGQ Engine  │
│  - DML/DDL    │  │ - Patterns   │  │ - GRAPH_TABLE│
│  - Joins      │  │ - Paths      │  │ - Flat rows  │
│  - Aggregates │  │ - Algorithms │  │              │
└──────┬───────┘  └──────┬───────┘  └──────┬───────┘
       └─────────────────┼─────────────────┘
                         ▼
┌─────────────────────────────────────────────────────────────┐
│                  Storage Layer (raisin-storage)             │
│  RocksDB, Tantivy (full-text), HNSW (vectors)               │
└─────────────────────────────────────────────────────────────┘
```

## Usage

```rust
use raisin_sql_execution::QueryEngine;
use raisin_rocksdb::RocksDBStorage;
use std::sync::Arc;
use futures::StreamExt;

let storage = Arc::new(RocksDBStorage::open("./data")?);
let engine = QueryEngine::new(storage, "tenant1", "repo1", "main");

// Execute SQL with workspace-as-table pattern
let mut stream = engine.execute("SELECT * FROM default LIMIT 10").await?;

while let Some(row) = stream.next().await {
    println!("{:?}", row?);
}
```

### Time-Travel Queries

```rust
// Query at specific revision
let sql = "SELECT * FROM default WHERE __revision = 100";
let stream = engine.execute(sql).await?;
```

### Cypher Execution

```rust
// Graph pattern matching
let cypher = "MATCH (u:User)-[:FOLLOWS]->(f:User) RETURN u.name, f.name";
let stream = engine.execute_cypher(cypher).await?;
```

## Physical Operators

| Category | Operators |
|----------|-----------|
| **Scans** | TableScan, PrefixScan, PropertyIndexScan, FullTextScan, NodeIdScan |
| **Joins** | HashJoin, NestedLoopJoin, IndexLookupJoin, SemiJoin |
| **Aggregation** | HashAggregate (COUNT, SUM, AVG, MIN, MAX, COLLECT, STRING_AGG) |
| **Transforms** | Filter, Project, Sort, Limit, Distinct, Union |
| **Window** | Row numbering, ranking, lag/lead functions |
| **Vector** | VectorDistanceScan (L2, Cosine, InnerProduct) |

## Expression Functions

| Category | Functions |
|----------|-----------|
| **String** | LOWER, UPPER, COALESCE, NULLIF, CONCAT, SUBSTRING |
| **Numeric** | ROUND, ABS, CEIL, FLOOR, SQRT, MOD, POWER |
| **JSON** | JSON_EXTRACT, JSON_SET, JSON_CONTAINS, JSON_EXISTS |
| **Geospatial** | ST_POINT, ST_DISTANCE, ST_WITHIN, ST_INTERSECTS, ST_DWithin |
| **Hierarchy** | ANCESTOR_OF, CHILD_OF, DESCENDANT_OF, DEPTH, PARENT |
| **Full-Text** | TS_RANK, TSVECTOR, TSQUERY |
| **Temporal** | NOW, date/timestamp operations |

## Modules

| Module | Description |
|--------|-------------|
| `engine.rs` | QueryEngine high-level API |
| `physical_plan/operators.rs` | PhysicalPlan enum and operator definitions |
| `physical_plan/planner.rs` | Logical → Physical plan conversion |
| `physical_plan/executor.rs` | Streaming execution engine |
| `physical_plan/eval/` | Expression evaluation (12 submodules) |
| `physical_plan/cypher/` | Cypher execution engine (14 submodules) |
| `physical_plan/pgq/` | PGQ GRAPH_TABLE execution |
| `physical_plan/batch.rs` | Columnar batch processing |

## Key Types

```rust
// Streaming row results
pub type RowStream = Pin<Box<dyn Stream<Item = Result<Row>> + Send>>;

// Row with named columns
pub struct Row {
    pub columns: IndexMap<String, PropertyValue>,
}

// Batch for vectorized processing
pub struct Batch { /* columnar data */ }
pub type BatchStream = Pin<Box<dyn Stream<Item = Result<Batch>> + Send>>;
```

## Dependencies

**Internal crates:**
- `raisin-sql` - Parsing, analysis, optimization (WASM-compatible)
- `raisin-storage` - Storage trait abstraction
- `raisin-rocksdb` - RocksDB implementation
- `raisin-indexer` - Tantivy full-text search
- `raisin-hnsw` - Vector similarity search
- `raisin-cypher-parser` - Cypher query parsing
- `raisin-core` - Node services, RLS filtering

**Note:** This crate depends on tokio, RocksDB, and other heavy dependencies that prevent WASM compilation. Use `raisin-sql` for WASM-compatible parsing and planning.

## Features

```toml
[features]
default = []
profiling = ["tracing/max_level_debug"]  # Detailed execution tracing
```

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
