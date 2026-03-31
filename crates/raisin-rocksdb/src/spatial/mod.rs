//! Geohash-based spatial indexing utilities for PostGIS-compatible ST_* queries
//!
//! This module provides geohash encoding/decoding and neighbor calculation
//! for efficient geospatial indexing in RocksDB.
//!
//! # Design Philosophy
//!
//! We use geohash-based indexing instead of R-tree for several reasons:
//! - **Infinite scale**: O(1) insert, O(log n) range queries via LSM-tree
//! - **Zero RAM**: No in-memory tree structure needed
//! - **Instant startup**: No tree loading from disk
//! - **MVCC-friendly**: Works seamlessly with revision-aware storage

mod geometry;
mod ops;
#[cfg(test)]
mod tests;

pub use geometry::*;
pub use ops::*;

/// Geohash precisions used for multi-precision indexing
pub const INDEX_PRECISIONS: &[usize] = &[4, 5, 6, 7, 8];

/// Default precision for proximity queries (balances precision vs query cost)
pub const DEFAULT_QUERY_PRECISION: usize = 6;
