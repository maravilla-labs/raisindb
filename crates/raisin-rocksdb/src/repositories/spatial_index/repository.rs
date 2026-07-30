//! Core spatial index repository: geohash-based writes and proximity queries.
//!
//! Writes delegate to `crate::indexing::spatial`, which is the only code allowed
//! to construct a spatial index key. This file owns the READ side.

use crate::indexing::{IndexCtx, SpatialIndexTargets};
use crate::{cf, cf_handle, keys, spatial};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, SpatialPolicy};
use rocksdb::{WriteBatch, DB};
use std::sync::Arc;

use super::entry::SpatialEntry;
use raisin_storage::spatial::{ProximityResult, SpatialIndexEntry, SpatialPreFilter};

mod knn;
pub(crate) mod scan;

pub use knn::KnnRingPlan;

/// Repository for managing geospatial indexes
#[derive(Clone)]
pub struct SpatialIndexRepository {
    pub(super) db: Arc<DB>,
    /// Per-cell scan budget; see [`crate::DEFAULT_SPATIAL_MAX_ENTRIES_PER_CELL`]
    /// and `repository::scan` for what exceeding it means.
    pub(super) max_entries_per_cell: usize,
}

impl SpatialIndexRepository {
    pub fn new(db: Arc<DB>) -> Self {
        Self {
            db,
            max_entries_per_cell: crate::DEFAULT_SPATIAL_MAX_ENTRIES_PER_CELL,
        }
    }

    /// Override the per-cell scan budget.
    ///
    /// Set from `RocksDBConfig::spatial_max_entries_per_cell`. Exists as a knob
    /// so the DEGRADATION path is reachable in a test without writing a quarter
    /// of a million index entries.
    pub fn with_max_entries_per_cell(mut self, max_entries_per_cell: usize) -> Self {
        self.max_entries_per_cell = max_entries_per_cell;
        self
    }

    /// Index a geometry property for a node at every precision in `policy`.
    ///
    /// Standalone variant that commits its own batch. Prefer
    /// [`Self::index_geometry_to_batch`] wherever a caller already has a batch —
    /// committing separately opens a window in which the node record exists but its
    /// index entries do not.
    #[allow(clippy::too_many_arguments)]
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
        policy: &SpatialPolicy,
        bucket: Option<&str>,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();
        self.index_geometry_to_batch(
            &mut batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            property_name,
            geometry,
            revision,
            policy,
            bucket,
        )?;
        self.db
            .write(batch)
            .map_err(|e| Error::storage(e.to_string()))
    }

    /// Index a geometry into a WriteBatch (for atomic commits with node data).
    #[allow(clippy::too_many_arguments)]
    pub fn index_geometry_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        geometry: &GeoJson,
        revision: &HLC,
        policy: &SpatialPolicy,
        bucket: Option<&str>,
    ) -> Result<()> {
        let targets = SpatialIndexTargets::from_db(&self.db)?;
        let ctx = IndexCtx::new(tenant_id, repo_id, branch, workspace);
        crate::indexing::write_spatial_property(
            batch,
            &targets,
            &ctx,
            node_id,
            property_name,
            geometry,
            revision,
            policy,
            bucket,
        )
    }

    /// Tombstone the spatial index entries for a node's OLD geometry.
    ///
    /// Takes the old geometry so the cells can be **derived** rather than
    /// discovered by scanning. The previous signature omitted it and prefix-scanned
    /// the whole workspace property range per update — O(all geometries in the
    /// workspace) for a single-node write.
    #[allow(clippy::too_many_arguments)]
    pub fn unindex_geometry_to_batch(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        old_geometry: &GeoJson,
        revision: &HLC,
        policy: &SpatialPolicy,
    ) -> Result<()> {
        let targets = SpatialIndexTargets::from_db(&self.db)?;
        let ctx = IndexCtx::new(tenant_id, repo_id, branch, workspace);
        crate::indexing::tombstone_spatial_property(
            batch,
            &targets,
            &ctx,
            node_id,
            property_name,
            old_geometry,
            revision,
            // An explicitly supplied policy carries no evidence about what is
            // physically present, so this API keeps the unconditional widening.
            crate::indexing::TombstonePrecisions::every(policy),
        )
    }

    /// Tombstone spatial index entries for a node's OLD geometry, committing
    /// immediately.
    #[allow(clippy::too_many_arguments)]
    pub fn unindex_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        old_geometry: &GeoJson,
        revision: &HLC,
        policy: &SpatialPolicy,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();
        self.unindex_geometry_to_batch(
            &mut batch,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            property_name,
            old_geometry,
            revision,
            policy,
        )?;
        self.db
            .write(batch)
            .map_err(|e| Error::storage(e.to_string()))
    }

    /// Scan spatial index entries matching geohash cells.
    ///
    /// Low-level scan returning entries from multiple geohash cells, with MVCC
    /// resolution: for each node only the newest revision at or before
    /// `max_revision` is considered, and if that revision is a tombstone the node is
    /// excluded entirely.
    #[allow(clippy::too_many_arguments)]
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
        let mut results: Vec<SpatialIndexEntry> = self
            .resolve_live_candidates(
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                cells,
                max_revision,
                &SpatialPreFilter::default(),
            )?
            .into_iter()
            .map(|c| SpatialIndexEntry {
                node_id: c.node_id,
                geohash: c.geohash,
                revision: c.revision,
            })
            .collect();

        // Stable output regardless of the resolution map's iteration order, so a
        // `limit` truncation is deterministic across runs and across nodes.
        results.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        results.truncate(limit);
        Ok(results)
    }

    /// Find nodes within a given distance of a point.
    ///
    /// `precisions` must be the precisions the index was **actually built at**. When
    /// no cell plan can cover the requested radius the call returns an error rather
    /// than a partial result set, so the planner falls back to a scan instead of
    /// answering the query wrongly.
    #[allow(clippy::too_many_arguments)]
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
        precisions: &[usize],
        prefilter: &SpatialPreFilter,
    ) -> Result<Vec<ProximityResult>> {
        let precisions = if precisions.is_empty() {
            spatial::INDEX_PRECISIONS
        } else {
            precisions
        };

        let plan = spatial::plan_radius_scan(center_lon, center_lat, radius_meters, precisions);
        let cells = match &plan {
            spatial::SpatialScanPlan::Covering { cells, .. } => cells.clone(),
            spatial::SpatialScanPlan::NotCovering => {
                return Err(Error::storage(format!(
                    "spatial index cannot cover a {radius_meters} m radius at precisions {precisions:?} \
                     within the cell budget; the caller must fall back to a scan"
                )));
            }
        };

        let candidates = self.resolve_live_candidates(
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            &cells,
            max_revision,
            prefilter,
        )?;

        let mut results: Vec<ProximityResult> = candidates
            .into_iter()
            .filter_map(|c| {
                let distance = haversine_distance(center_lon, center_lat, c.entry.lon, c.entry.lat);
                (distance <= radius_meters).then(|| c.into_proximity(distance))
            })
            .collect();

        results.sort_by(|a, b| a.distance_meters.total_cmp(&b.distance_meters));
        results.truncate(limit);
        Ok(results)
    }

    /// Find the k nearest neighbours to a point.
    ///
    /// See [`knn`] for the stopping rule and why the previous implementation was
    /// both dead and wrong.
    #[allow(clippy::too_many_arguments)]
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
        precisions: &[usize],
        prefilter: &SpatialPreFilter,
    ) -> Result<Vec<ProximityResult>> {
        knn::find_nearest(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            center_lon,
            center_lat,
            k,
            max_revision,
            precisions,
            prefilter,
        )
    }

    /// Iterate the workspace's spatial keys for one property, tombstoning every
    /// entry belonging to `node_id`.
    ///
    /// **Repair path only.** This is the O(all entries) shape the hot write path used
    /// to have; it survives so the reindex job can clean up entries whose originating
    /// geometry is no longer recoverable (e.g. written under a cover mode that has
    /// since changed). Never call it from a write path.
    pub fn tombstone_all_entries_for_node(
        &self,
        batch: &mut WriteBatch,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        node_id: &str,
        revision: &HLC,
    ) -> Result<usize> {
        let cf = cf_handle(&self.db, cf::SPATIAL_INDEX)?;
        let prefix = keys::spatial_index_property_prefix(
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
        );

        let mut count = 0usize;
        for item in self.db.prefix_iterator_cf(cf, &prefix) {
            let (key, value) = item.map_err(|e| Error::storage(e.to_string()))?;
            if !key.starts_with(&prefix) {
                break;
            }
            if value.as_ref() == crate::indexing::SPATIAL_TOMBSTONE {
                continue;
            }
            let Some(parsed) = keys::parse_spatial_index_key(&key) else {
                continue;
            };
            if parsed.node_id != node_id {
                continue;
            }
            // A tombstone must SUPERSEDE what it tombstones. Resolution is
            // newest-revision-wins, so a tombstone stamped older than the entry it
            // targets simply loses and the stale entry stays live. That is not
            // hypothetical: entries stranded at a future revision (a rebuild used to
            // stamp wall-clock now, ahead of the branch head) would otherwise
            // survive every subsequent rebuild forever. Stamping at the entry's own
            // revision when that is newer makes the tombstone land on the exact key,
            // which the batch then overwrites — and a live re-emission written after
            // it in the same batch still wins, as it must.
            let stamp = if parsed.revision > *revision {
                parsed.revision
            } else {
                *revision
            };
            let tombstone_key = keys::spatial_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                property_name,
                parsed.geohash,
                &stamp,
                node_id,
            );
            batch.put_cf(cf, tombstone_key, crate::indexing::SPATIAL_TOMBSTONE);
            count += 1;
        }
        Ok(count)
    }
}

/// A live index candidate, resolved across every scanned cell.
pub(super) struct SpatialCandidate {
    pub node_id: String,
    pub geohash: String,
    pub revision: HLC,
    pub entry: SpatialEntry,
}

impl SpatialCandidate {
    pub(super) fn into_proximity(self, distance_meters: f64) -> ProximityResult {
        ProximityResult {
            node_id: self.node_id,
            distance_meters,
            centroid: (self.entry.lon, self.entry.lat),
            bbox: self.entry.bbox,
            z_range: self.entry.z,
            srid: self.entry.srid,
            bucket: self.entry.bucket,
        }
    }
}

/// Calculate Haversine distance between two points in meters.
///
/// Uses the GRS80 mean radius, shared with `raisin_geometry::EARTH_MEAN_RADIUS_METERS`
/// and hence with `ST_DISTANCE`. If the two constants ever diverge, rows flicker in
/// and out at the radius boundary depending on whether the index post-filter or the
/// residual SQL filter saw them last — so the equality is asserted by a test rather
/// than assumed.
pub(super) fn haversine_distance(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();

    raisin_geometry::EARTH_MEAN_RADIUS_METERS * c
}
