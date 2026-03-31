//! Storage trait implementation for the spatial index repository.

use super::repository::SpatialIndexRepository;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::GeoJson;
use raisin_storage::spatial::ProximityResult;

// Implement the storage trait for RocksDB
impl raisin_storage::SpatialIndexRepository for SpatialIndexRepository {
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
    ) -> Result<()> {
        SpatialIndexRepository::index_geometry(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            property_name,
            geometry,
            revision,
        )
    }

    fn unindex_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        revision: &HLC,
    ) -> Result<()> {
        SpatialIndexRepository::unindex_geometry(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            property_name,
            revision,
        )
    }

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
    ) -> Result<Vec<ProximityResult>> {
        SpatialIndexRepository::find_within_radius(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            property_name,
            center_lon,
            center_lat,
            radius_meters,
            max_revision,
            limit,
        )
    }

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
    ) -> Result<Vec<ProximityResult>> {
        SpatialIndexRepository::find_nearest(
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
        )
    }
}
