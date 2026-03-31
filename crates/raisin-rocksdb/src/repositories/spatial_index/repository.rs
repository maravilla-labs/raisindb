//! Core spatial index repository with geohash-based indexing and query methods.

use crate::{cf, cf_handle, keys, spatial};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::GeoJson;
use rocksdb::DB;
use std::sync::Arc;

use super::SpatialIndexEntry;
use raisin_storage::spatial::ProximityResult;

/// Repository for managing geospatial indexes
#[derive(Clone)]
pub struct SpatialIndexRepository {
    pub(super) db: Arc<DB>,
}

impl SpatialIndexRepository {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Index a geometry property for a node at multiple precisions
    ///
    /// Creates index entries at precisions 4-8 to support queries at various
    /// zoom levels. Each entry is keyed by geohash + revision for MVCC support.
    pub fn index_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        geometry: &GeoJson,
        revision: &HLC,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::SPATIAL_INDEX)?;

        // Get centroid and generate geohashes at all precisions
        let geohashes = spatial::geohashes_for_geometry(geometry);
        if geohashes.is_empty() {
            return Ok(()); // Invalid geometry, nothing to index
        }

        for geohash in &geohashes {
            let key = keys::spatial_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                geohash,
                revision,
                node_id,
            );

            // Value stores the centroid for efficient distance calculation without node lookup
            if let Some((lon, lat)) = spatial::geometry_centroid(geometry) {
                let value = format!("{},{}", lon, lat);
                self.db
                    .put_cf(cf, key, value.as_bytes())
                    .map_err(|e| Error::storage(e.to_string()))?;
            }
        }

        Ok(())
    }

    /// Remove spatial index entries for a node (creates tombstone at revision)
    ///
    /// Rather than physically deleting, we write tombstones for MVCC consistency.
    /// The tombstones will be cleaned up during garbage collection.
    pub fn unindex_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf = cf_handle(&self.db, cf::SPATIAL_INDEX)?;

        // Scan all entries for this property and write tombstones
        let prefix = keys::spatial_index_property_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
        );

        let iter = self.db.prefix_iterator_cf(cf, &prefix);
        for item in iter {
            let (key, _) = item.map_err(|e| Error::storage(e.to_string()))?;

            if !key.starts_with(&prefix) {
                break;
            }

            // Check if this entry is for our node
            // Key format: ...{geohash}\0{~rev}\0{node_id}
            let key_str = String::from_utf8_lossy(&key);
            if key_str.ends_with(&format!("\0{}", node_id)) {
                // Extract geohash from key for tombstone
                let parts: Vec<&str> = key_str.split('\0').collect();
                if parts.len() >= 7 {
                    let geohash = parts[6]; // geo\0property\0geohash\0...
                    let tombstone_key = keys::spatial_index_key_versioned(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        property_name,
                        geohash,
                        revision,
                        node_id,
                    );
                    // Tombstone marker
                    self.db
                        .put_cf(cf, tombstone_key, b"T")
                        .map_err(|e| Error::storage(e.to_string()))?;
                }
            }
        }

        Ok(())
    }

    /// Scan spatial index entries matching geohash cells
    ///
    /// Low-level scan that returns entries from multiple geohash cells.
    /// Used internally by higher-level query functions.
    pub fn scan_cells(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        cells: &[String],
        max_revision: &HLC,
        limit: usize,
    ) -> Result<Vec<SpatialIndexEntry>> {
        let cf = cf_handle(&self.db, cf::SPATIAL_INDEX)?;
        let mut results = Vec::new();
        let mut seen_nodes = std::collections::HashSet::new();

        for cell in cells {
            if results.len() >= limit {
                break;
            }

            let prefix = keys::spatial_index_geohash_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                cell,
            );

            let iter = self.db.prefix_iterator_cf(cf, &prefix);
            for item in iter {
                if results.len() >= limit {
                    break;
                }

                let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;

                if !key.starts_with(&prefix) {
                    break;
                }

                // Skip tombstones
                if value.as_ref() == b"T" {
                    continue;
                }

                // Extract node_id from key (last component)
                let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
                if parts.len() < 2 {
                    continue;
                }

                let node_id = String::from_utf8_lossy(parts[parts.len() - 1]).to_string();

                // Deduplicate (same node may appear at multiple precisions)
                if seen_nodes.contains(&node_id) {
                    continue;
                }

                // Check revision (HLC is stored before node_id)
                if let Ok(revision) =
                    keys::extract_revision_from_key(&key[..key.len() - node_id.len() - 1])
                {
                    // Only include if at or before max_revision
                    if revision <= *max_revision {
                        seen_nodes.insert(node_id.clone());
                        results.push(SpatialIndexEntry {
                            node_id,
                            geohash: cell.clone(),
                            revision,
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    /// Find nodes within a given distance of a point
    ///
    /// Uses geohash cell expansion to efficiently find candidate nodes,
    /// then filters by exact Haversine distance.
    pub fn find_within_radius(
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
    ) -> Result<Vec<ProximityResult>> {
        let cf = cf_handle(&self.db, cf::SPATIAL_INDEX)?;

        // Get cells to scan based on radius
        let cells = spatial::cells_for_radius(center_lon, center_lat, radius_meters);

        let mut results = Vec::new();
        let mut seen_nodes = std::collections::HashSet::new();

        for cell in &cells {
            let prefix = keys::spatial_index_geohash_prefix(
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                cell,
            );

            let iter = self.db.prefix_iterator_cf(cf, &prefix);
            for item in iter {
                let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;

                if !key.starts_with(&prefix) {
                    break;
                }

                // Skip tombstones
                if value.as_ref() == b"T" {
                    continue;
                }

                // Extract node_id and parse centroid from value
                let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
                if parts.len() < 2 {
                    continue;
                }

                let node_id = String::from_utf8_lossy(parts[parts.len() - 1]).to_string();

                // Deduplicate
                if seen_nodes.contains(&node_id) {
                    continue;
                }

                // Check revision
                if let Ok(revision) =
                    keys::extract_revision_from_key(&key[..key.len() - node_id.len() - 1])
                {
                    if revision > *max_revision {
                        continue;
                    }
                }

                // Parse centroid from value
                let value_str = String::from_utf8_lossy(&value);
                let coords: Vec<&str> = value_str.split(',').collect();
                if coords.len() != 2 {
                    continue;
                }

                let lon: f64 = coords[0].parse().unwrap_or(0.0);
                let lat: f64 = coords[1].parse().unwrap_or(0.0);

                // Calculate exact Haversine distance
                let distance = haversine_distance(center_lon, center_lat, lon, lat);

                // Filter by actual radius
                if distance <= radius_meters {
                    seen_nodes.insert(node_id.clone());
                    results.push(ProximityResult {
                        node_id,
                        distance_meters: distance,
                        centroid: (lon, lat),
                    });
                }
            }
        }

        // Sort by distance
        results.sort_by(|a, b| a.distance_meters.partial_cmp(&b.distance_meters).unwrap());

        // Apply limit
        results.truncate(limit);

        Ok(results)
    }

    /// Find k nearest neighbors to a point
    ///
    /// Uses progressive ring expansion to find the k nearest nodes efficiently.
    /// Starts with a small cell and expands outward until enough results found.
    pub fn find_nearest(
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
    ) -> Result<Vec<ProximityResult>> {
        // Start with precision 7 (about 150m cells) and expand if needed
        let mut precision = 7;
        let mut results = Vec::new();

        while precision >= 4 && results.len() < k {
            let center_hash = spatial::encode_point(center_lon, center_lat, precision);
            let cells = spatial::center_and_neighbors(&center_hash);

            // Use a large radius for initial filtering, actual k-NN will sort
            let candidates = self.find_within_radius(
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                center_lon,
                center_lat,
                spatial::precision_radius_meters(precision) * 2.0, // Cover neighbors too
                max_revision,
                k * 2, // Get extra candidates for better results
            )?;

            if candidates.len() >= k || precision <= 4 {
                results = candidates;
                break;
            }

            precision -= 1;
        }

        // Ensure we have exactly k or fewer results
        results.truncate(k);

        Ok(results)
    }
}

/// Calculate Haversine distance between two points in meters
pub(super) fn haversine_distance(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    const EARTH_RADIUS_METERS: f64 = 6_371_008.8; // Mean radius

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();

    EARTH_RADIUS_METERS * c
}
