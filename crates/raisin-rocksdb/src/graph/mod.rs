//! Graph algorithm precomputation framework
//!
//! This module provides infrastructure for precomputing and caching graph algorithm
//! results (PageRank, Louvain, Connected Components, etc.) for efficient querying.
//!
//! ## Architecture
//!
//! - **Storage**: Dedicated `GRAPH_CACHE` column family in RocksDB
//! - **Cache**: In-memory LRU cache for hot lookups
//! - **Background Task**: Periodic computation (NOT job queue) - like compaction
//! - **Scoping**: Computations can be scoped by paths, node types, workspaces, or relation types
//! - **Branch-Level**: Operates at branch level (like vector search)
//!
//! ## Design Decisions
//!
//! - NOT using job queue to avoid queue congestion from many small changes
//! - Natural debouncing: 100 changes → 1 recomputation when background tick runs
//! - TTL-based + staleness marking for cache invalidation
//!
//! ## Target Modes
//!
//! - `Branch`: Specific branches, tracks HEAD (recalculates on HEAD change)
//! - `AllBranches`: Compute for HEAD of all branches
//! - `Revision`: Specific revisions (immutable, computed once)
//! - `BranchPattern`: Glob pattern matching for branch names
//!
//! ## Supported Algorithms
//!
//! - PageRank
//! - Louvain (community detection)
//! - Connected Components
//! - Betweenness Centrality
//! - Triangle Count
//! - RELATES Cache (permission precomputation)

pub mod algorithms;
pub mod background_compute;
pub mod cache_layer;
pub mod config;
pub mod event_handler;
pub mod projection_cache;
pub mod scope;
pub mod types;

pub use algorithms::{AlgorithmExecutor, AlgorithmRegistry, AlgorithmResult};
pub use background_compute::{GraphComputeConfig, GraphComputeStats, GraphComputeTask, TickStats};
pub use cache_layer::{CacheStats, GraphCacheLayer};
pub use config::GraphAlgorithmConfig;
pub use event_handler::GraphProjectionEventHandler;
pub use projection_cache::{GraphProjectionStore, ProjectionKey};
pub use scope::ScopeFilter;
pub use types::PersistedProjection;
pub use types::*;
