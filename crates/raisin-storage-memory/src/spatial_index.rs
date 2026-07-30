use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::GeoJson;
use raisin_models::nodes::properties::SpatialPolicy;
use raisin_storage::spatial::{ProximityResult, SpatialIndexRepository, SpatialPreFilter};

#[derive(Clone, Default)]
pub struct InMemorySpatialIndexRepo;

// NOTE: the in-memory backend is DEPRECATED (see project memory: "raisin-storage-memory
// will be abandoned; don't fix its stubs"). These are signature-only updates to keep the
// workspace compiling after the spatial trait gained the policy / precisions / prefilter
// parameters — no behaviour is added, and none should be.
#[allow(clippy::too_many_arguments)]
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
        _policy: &SpatialPolicy,
        _bucket: Option<&str>,
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
        _old_geometry: &GeoJson,
        _revision: &HLC,
        _policy: &SpatialPolicy,
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
        _precisions: &[usize],
        _prefilter: &SpatialPreFilter,
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
        _precisions: &[usize],
        _prefilter: &SpatialPreFilter,
    ) -> Result<Vec<ProximityResult>> {
        // No spatial querying support in-memory – return empty result.
        Ok(Vec::new())
    }
}
