# raisin-graph-algorithms

Graph projection engine for RaisinDB with global graph algorithms.

## Overview

Enables running global graph algorithms (PageRank, Community Detection, Centrality) on RaisinDB data. Loads subgraphs from storage into an optimized in-memory structure (CSR - Compressed Sparse Row) for efficient parallel computation.

## Features

- **Graph Projection** - Load subgraphs from RocksDB with optional relation type filtering
- **CSR Representation** - Memory-efficient Compressed Sparse Row format via petgraph
- **Parallel Execution** - Rayon-powered parallelism for algorithm execution
- **ID Mapping** - Bidirectional mapping between string IDs (UUIDs/Paths) and dense integers
- **Result Writeback** - Write algorithm results back to node properties

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Data Sources                           │
├─────────────────────────────────────────────────────────────┤
│             RocksDB (Relations / Path Index)                │
└───────────────────────────┬─────────────────────────────────┘
                            │
                            ▼
              ┌───────────────────────────┐
              │     GraphProjection       │
              │  - Scan relations         │
              │  - Build ID mapping       │
              │  - Construct CSR graph    │
              └─────────────┬─────────────┘
                            │
                            ▼
              ┌───────────────────────────┐
              │       Algorithms          │
              │  (rayon parallelism)      │
              └─────────────┬─────────────┘
                            │
         ┌──────────┬───────┴───────┬──────────┐
         ▼          ▼               ▼          ▼
   ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
   │ PageRank │ │  Louvain │ │   WCC    │ │ Triangle │
   │          │ │ Community│ │          │ │  Count   │
   └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘
        └────────────┴────────────┴────────────┘
                            │
                            ▼
              ┌───────────────────────────┐
              │        Writeback          │
              │  Map results to node IDs  │
              │  Update node properties   │
              └───────────────────────────┘
```

## Algorithms

| Algorithm | Description | Returns |
|-----------|-------------|---------|
| `page_rank` | Iterative link analysis ranking | NodeID -> Score (f64) |
| `louvain` | Modularity-optimizing community detection | NodeID -> CommunityID (u32) |
| `weakly_connected_components` | Union-Find based component discovery | NodeID -> ComponentID (u32) |
| `triangle_count` | Local clustering via triangle enumeration | NodeID -> Count (usize) |
| `astar` | A* shortest path with custom heuristics | (cost, path) |
| `k_shortest_paths` | Yen's algorithm for K shortest paths | Vec<(cost, path)> |

## Usage

### Building a Projection

```rust
use raisin_graph_algorithms::GraphProjection;

// From explicit nodes and edges
let nodes = vec!["A".to_string(), "B".to_string(), "C".to_string()];
let edges = vec![
    ("A".to_string(), "B".to_string()),
    ("B".to_string(), "C".to_string()),
];
let projection = GraphProjection::from_parts(nodes, edges);

// From storage (async)
let projection = GraphProjection::from_storage(
    &storage,
    tenant_id,
    repo_id,
    branch,
    Some("FOLLOWS"),  // Optional relation type filter
    None,             // Max revision
).await?;
```

### Running Algorithms

```rust
use raisin_graph_algorithms::algorithms::{page_rank, louvain, weakly_connected_components};

// PageRank with damping factor 0.85, 20 iterations, tolerance 1e-6
let scores = page_rank(&projection, 0.85, 20, 1e-6);

// Louvain community detection with 10 iterations, resolution 1.0
let communities = louvain(&projection, 10, 1.0);

// Connected components
let components = weakly_connected_components(&projection);
```

### Pathfinding

```rust
use raisin_graph_algorithms::algorithms::{astar, k_shortest_paths};

// A* with custom cost and heuristic
let result = astar(
    &projection,
    "start_node",
    "end_node",
    |u, v| 1.0,           // Edge cost function
    |n| 0.0,              // Heuristic (Dijkstra when 0)
);

// K shortest paths
let paths = k_shortest_paths(
    &projection,
    "A",
    "E",
    3,                    // Find 3 shortest paths
    |u, v| edge_weights.get(&(u, v)).unwrap_or(&1.0),
);
```

### Writing Results Back

```rust
use raisin_graph_algorithms::writeback::{write_float_results, write_integer_results};

// Write PageRank scores to node properties
write_float_results(
    &storage,
    tenant_id, repo_id, branch, workspace,
    "pagerank_score",
    scores,
).await?;

// Write community IDs to node properties
write_integer_results(
    &storage,
    tenant_id, repo_id, branch, workspace,
    "community_id",
    communities,
).await?;
```

## Components

| Module | Description |
|--------|-------------|
| `projection.rs` | `GraphProjection` struct with CSR graph and ID mapping |
| `algorithms.rs` | Algorithm implementations (PageRank, Louvain, WCC, etc.) |
| `writeback.rs` | Helpers to persist results to node properties |
| `error.rs` | `GraphError` type for projection/execution errors |

## Performance Notes

- **ID Mapping**: String IDs are mapped to dense `u32` integers for cache-friendly access
- **CSR Format**: Compressed Sparse Row provides O(1) neighbor iteration
- **Parallel Iteration**: Degree calculation, convergence checks, and triangle counting use rayon
- **Memory**: Only topology is stored (no edge weights by default)

## Crate Usage

Used by:
- `raisin-rocksdb` - Graph algorithm caching and precomputation

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
