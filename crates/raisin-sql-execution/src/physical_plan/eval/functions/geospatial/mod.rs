//! Geospatial functions for PostGIS-compatible spatial queries
//!
//! This module contains functions for geospatial operations:
//! - ST_POINT: Create a point geometry from lon/lat coordinates
//! - ST_GEOMFROMGEOJSON: Parse GeoJSON text to geometry
//! - ST_ASGEOJSON: Convert geometry to GeoJSON text
//! - ST_DISTANCE: Calculate distance between geometries (meters)
//! - ST_DWITHIN: Check if geometries are within distance
//! - ST_CONTAINS: Check if geometry A contains geometry B
//! - ST_WITHIN: Check if geometry A is within geometry B
//! - ST_INTERSECTS: Check if geometries intersect
//! - ST_X: Get X coordinate (longitude) of a point
//! - ST_Y: Get Y coordinate (latitude) of a point

mod helpers;
mod st_asgeojson;
mod st_contains;
mod st_distance;
mod st_dwithin;
mod st_geomfromgeojson;
mod st_intersects;
mod st_point;
mod st_within;
mod st_x;
mod st_y;

pub use st_asgeojson::StAsGeoJsonFunction;
pub use st_contains::StContainsFunction;
pub use st_distance::StDistanceFunction;
pub use st_dwithin::StDWithinFunction;
pub use st_geomfromgeojson::StGeomFromGeoJsonFunction;
pub use st_intersects::StIntersectsFunction;
pub use st_point::StPointFunction;
pub use st_within::StWithinFunction;
pub use st_x::StXFunction;
pub use st_y::StYFunction;

use super::registry::FunctionRegistry;

/// Register all geospatial functions in the provided registry
///
/// This function is called during registry initialization to register
/// all PostGIS-compatible spatial functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    // Geometry constructors
    registry.register(Box::new(StPointFunction));
    registry.register(Box::new(StGeomFromGeoJsonFunction));

    // Output functions
    registry.register(Box::new(StAsGeoJsonFunction));

    // Distance functions
    registry.register(Box::new(StDistanceFunction));
    registry.register(Box::new(StDWithinFunction));

    // Spatial predicates
    registry.register(Box::new(StContainsFunction));
    registry.register(Box::new(StWithinFunction));
    registry.register(Box::new(StIntersectsFunction));

    // Accessor functions
    registry.register(Box::new(StXFunction));
    registry.register(Box::new(StYFunction));
}
