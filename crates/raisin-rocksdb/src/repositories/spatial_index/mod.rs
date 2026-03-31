//! Spatial index repository implementation for geohash-based geospatial queries
//!
//! This repository provides PostGIS-compatible ST_* query support using geohash
//! indexing in RocksDB. Key features:
//!
//! - Multi-precision indexing (precisions 4-8) for optimal query performance
//! - Ring expansion for k-NN queries
//! - MVCC-aware with revision support
//! - Seamless integration with PATH_STARTS_WITH and CHILD_OF predicates

mod repository;
#[cfg(test)]
mod tests;
mod trait_impl;

// Re-export types from raisin_storage::spatial trait
pub use raisin_storage::spatial::{ProximityResult, SpatialIndexEntry};

pub use repository::SpatialIndexRepository;
