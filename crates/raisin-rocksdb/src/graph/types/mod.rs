//! Graph algorithm cache types
//!
//! Types for storing precomputed graph algorithm results in the GRAPH_CACHE column family.
//! Uses MessagePack serialization via serde.

mod algorithm;
mod cache;
mod config;
mod projection;

pub use algorithm::GraphAlgorithm;
pub use cache::{CacheStatus, CachedValue, GraphCacheKey, GraphCacheMeta, GraphCacheValue};
pub use config::{GraphScope, GraphTarget, RefreshConfig, TargetMode};
pub use projection::PersistedProjection;
