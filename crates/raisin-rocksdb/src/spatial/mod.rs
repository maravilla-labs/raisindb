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
//!
//! # The correctness invariant
//!
//! **The configured precision set is a PERFORMANCE knob only. It must never
//! change which rows a query returns.** [`plan_radius_scan`] is written to
//! guarantee that: whatever precisions an index was built at, the returned cell
//! set is a proven-complete cover of the query circle, so the Haversine
//! post-filter can only ever *remove* false positives — never restore a missing
//! row. A too-coarse index over-fetches; a too-fine index over-fetches; neither
//! silently drops a match.
//!
//! That property is what makes a heterogeneous cluster mid-reindex safe: every
//! node returns the same result set at differing latency.
//!
//! # The SRID invariant
//!
//! Geohash cells are lon/lat degrees, so **every** indexed geometry is
//! normalised to WGS84 before its cells are derived — see [`normalize`] and
//! [`cells_for_geometry`], which is the one funnel all write paths reach. The
//! stored geometry keeps its own SRID; only the index is normalised. An SRID the
//! built-in projection tier cannot normalise fails the write loudly on *every*
//! build configuration, so two nodes compiled with different projection features
//! can never disagree about what is in the index.

mod cover;
mod geometry;
mod normalize;
mod ops;
mod plan;
#[cfg(test)]
mod tests;

pub use cover::*;
pub use geometry::*;
pub use normalize::{is_indexable_srid, normalize_geometry_for_index};
pub use ops::*;
pub use plan::*;

/// Geohash precisions used for multi-precision indexing when no policy applies.
///
/// Re-exported from `raisin-models` so the write path, the query planner and the
/// clients all agree on one list. `&[2, 4, 6, 7, 8, 9, 10, 11]` — cell radii
/// 1250 km, 39 km, 1.2 km, 153 m, 38 m, 4.8 m, 1.2 m, 0.15 m.
pub use raisin_models::nodes::properties::INDEX_PRECISIONS_DEFAULT as INDEX_PRECISIONS;
