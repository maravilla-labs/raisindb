//! Spatial index repository implementation for geohash-based geospatial queries
//!
//! This repository provides PostGIS-compatible ST_* query support using geohash
//! indexing in RocksDB. Key features:
//!
//! - Multi-precision indexing at the precisions the resolved `SpatialPolicy`
//!   configures (default `[2, 4, 6, 7, 8, 9, 10, 11]` — sub-metre through global)
//! - Cover-guaranteed cell planning, so the precision set is a performance knob and
//!   never changes which rows a query returns
//! - Correct expanding-ring k-NN with an admissible stopping bound
//! - MVCC-aware with cross-cell tombstone resolution
//! - Seamless integration with PATH_STARTS_WITH and CHILD_OF predicates

mod entry;
mod repository;
#[cfg(test)]
mod tests;
mod trait_impl;

// Re-export types from raisin_storage::spatial trait
pub use raisin_storage::spatial::{ProximityResult, SpatialIndexEntry, SpatialPreFilter};

pub use entry::{
    SpatialEntry, SpatialGeometryKind, SPATIAL_ENTRY_LEGACY_VERSION, SPATIAL_ENTRY_VERSION,
};
pub use repository::{KnnRingPlan, SpatialIndexRepository};
