//! Spatial index repository trait for geohash-based geospatial queries.
//!
//! This module defines the interface for spatial indexing operations using
//! geohash-based indexing for PostGIS-compatible ST_* queries.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::GeoJson;

/// Entry from spatial index scan
#[derive(Debug, Clone)]
pub struct SpatialIndexEntry {
    /// Node ID
    pub node_id: String,
    /// Geohash at which this entry was indexed
    pub geohash: String,
    /// Revision at which this entry was created
    pub revision: HLC,
}

/// Result from a proximity query
#[derive(Debug, Clone)]
pub struct ProximityResult {
    /// Node ID
    pub node_id: String,
    /// Distance from query point in meters
    pub distance_meters: f64,
    /// The node's geometry centroid (lon, lat)
    pub centroid: (f64, f64),
}

/// Spatial index repository trait for geospatial queries.
///
/// This trait defines the interface for geohash-based spatial indexing,
/// supporting PostGIS-compatible ST_DWithin and k-NN queries.
///
/// # Implementation Notes
///
/// Implementations should use geohash multi-precision indexing (precisions 4-8)
/// to support efficient queries at various zoom levels. All methods are
/// MVCC-aware with revision support.
pub trait SpatialIndexRepository: Send + Sync {
    /// Index a geometry property for a node at multiple precisions.
    ///
    /// Creates index entries at precisions 4-8 to support queries at various
    /// zoom levels. Each entry is keyed by geohash + revision for MVCC support.
    fn index_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        geometry: &GeoJson,
        revision: &HLC,
    ) -> Result<()>;

    /// Remove spatial index entries for a node (creates tombstone at revision).
    fn unindex_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        revision: &HLC,
    ) -> Result<()>;

    /// Find nodes within a given distance of a point (ST_DWithin).
    ///
    /// Uses geohash cell expansion to efficiently find candidate nodes,
    /// then filters by exact Haversine distance.
    ///
    /// # Returns
    /// Nodes within the radius, sorted by distance (nearest first)
    fn find_within_radius(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        center_lon: f64,
        center_lat: f64,
        radius_meters: f64,
        max_revision: &HLC,
        limit: usize,
    ) -> Result<Vec<ProximityResult>>;

    /// Find k nearest neighbors to a point (k-NN query).
    ///
    /// Uses progressive ring expansion to find the k nearest nodes efficiently.
    ///
    /// # Returns
    /// Up to k nearest nodes, sorted by distance (nearest first)
    fn find_nearest(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        center_lon: f64,
        center_lat: f64,
        k: usize,
        max_revision: &HLC,
    ) -> Result<Vec<ProximityResult>>;
}
