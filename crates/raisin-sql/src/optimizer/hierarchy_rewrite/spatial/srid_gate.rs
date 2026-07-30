//! Which constant query geometries may drive a spatial index scan.
//!
//! One rule, applied by both [`super::build_dwithin`] and
//! [`super::extract_distance_order`], so the predicate path and the ordering
//! path cannot disagree about it — the same "two copies of one matcher" hazard
//! the parent module exists to remove.

use super::consts::const_geometry_srid;
use serde_json::Value;

/// EPSG:4326. Spelled out here because `raisin-sql` carries no CRS dependency.
const EPSG_WGS84: u32 = 4326;

/// True when a constant query geometry is in the CRS the spatial index is keyed
/// on, and may therefore drive an index scan.
///
/// # Why a projected centre declines the index instead of being reprojected
///
/// Spatial index cells are geohashes over WGS84 lon/lat — the write path
/// normalises every geometry into that frame
/// (`raisin_rocksdb::spatial::normalize_geometry_for_index`), whatever SRID it
/// is stored in. A query centre in a projected CRS is therefore in the wrong
/// frame twice over: its coordinates are eastings/northings, and its radius is
/// measured in *projected* metres, which are not ground metres. Web Mercator
/// inflates by `1/cos(lat)` (harmless — it only widens the candidate set), but a
/// UTM zone SHRINKS by up to 0.04% on its central meridian. Reprojecting the
/// centre would fix the coordinates and silently leave the radius, and a radius
/// 0.04% too small turns the index scan from a superset into a subset — rows
/// dropped with no signal, which is the exact failure class this subsystem
/// exists to remove.
///
/// So the predicate stays in the residual filter and the query falls back to a
/// scan, where the row-level `ST_DWITHIN` evaluator applies the real,
/// SRID-aware semantics. Slower, never wrong.
///
/// # The regression this closes
///
/// Before this gate the centre's SRID was dropped during constant folding
/// (`ST_SETSRID` folded to its argument), so an EPSG:3857 centre was planned as
/// if `950668` were a longitude: `encode_point` rejected it, the cell plan came
/// back `NotCovering`, and the whole query failed with a confusing "spatial
/// index cannot cover" error.
pub(super) fn query_center_is_wgs84(center: &Value) -> bool {
    match const_geometry_srid(center) {
        None => true,
        Some(srid) => srid == EPSG_WGS84,
    }
}
