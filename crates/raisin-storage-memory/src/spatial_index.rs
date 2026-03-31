use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::GeoJson;
use raisin_storage::spatial::{ProximityResult, SpatialIndexRepository};

#[derive(Clone, Default)]
pub struct InMemorySpatialIndexRepo;

impl SpatialIndexRepository for InMemorySpatialIndexRepo {
    fn index_geometry(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _property_name: &str,
        _geometry: &GeoJson,
        _revision: &HLC,
    ) -> Result<()> {
        // In-memory backend does not maintain spatial indexes yet.
        Ok(())
    }

    fn unindex_geometry(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _node_id: &str,
        _property_name: &str,
        _revision: &HLC,
    ) -> Result<()> {
        // In-memory backend does not maintain spatial indexes yet.
        Ok(())
    }

    fn find_within_radius(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _property_name: &str,
        _center_lon: f64,
        _center_lat: f64,
        _radius_meters: f64,
        _max_revision: &HLC,
        _limit: usize,
    ) -> Result<Vec<ProximityResult>> {
        // No spatial querying support in-memory – return empty result.
        Ok(Vec::new())
    }

    fn find_nearest(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _workspace: &str,
        _property_name: &str,
        _center_lon: f64,
        _center_lat: f64,
        _k: usize,
        _max_revision: &HLC,
    ) -> Result<Vec<ProximityResult>> {
        // No spatial querying support in-memory – return empty result.
        Ok(Vec::new())
    }
}
