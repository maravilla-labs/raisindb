//! Geospatial functions for PostGIS-compatible spatial queries
//!
//! This module contains functions for geospatial operations:
//! - ST_POINT: Create a point geometry from lon/lat coordinates
//! - ST_MAKEPOINT: Alias for ST_POINT (PostGIS naming)
//! - ST_GEOMFROMGEOJSON: Parse GeoJSON text to geometry
//! - ST_ASGEOJSON: Convert geometry to GeoJSON text
//! - ST_DISTANCE: Calculate distance between geometries (meters)
//! - ST_DWITHIN: Check if geometries are within distance
//! - ST_CONTAINS: Check if geometry A contains geometry B
//! - ST_WITHIN: Check if geometry A is within geometry B
//! - ST_INTERSECTS: Check if geometries intersect
//! - ST_AREA: Calculate area of a geometry (square meters)
//! - ST_LENGTH: Calculate length of a geometry (meters)
//! - ST_PERIMETER: Calculate perimeter of a geometry (meters)
//! - ST_AZIMUTH: Calculate bearing between two points (radians)
//! - ST_X: Get X coordinate (longitude) of a point
//! - ST_Y: Get Y coordinate (latitude) of a point
//! - ST_TOUCHES: Check if geometries touch (share boundary, not interior)
//! - ST_CROSSES: Check if geometries cross each other
//! - ST_OVERLAPS: Check if same-dimension geometries overlap
//! - ST_DISJOINT: Check if geometries do not intersect
//! - ST_EQUALS: Check if geometries are topologically equal
//! - ST_COVERS: Check if geometry A covers geometry B
//! - ST_COVEREDBY: Check if geometry A is covered by geometry B
//! - ST_GEOMETRYTYPE: Get the geometry type as a string
//! - ST_NUMPOINTS: Get number of coordinate points
//! - ST_NUMGEOMETRIES: Get number of sub-geometries
//! - ST_ISVALID: Check if geometry is valid
//! - ST_ISEMPTY: Check if geometry is empty
//! - ST_ISCLOSED: Check if geometry is closed
//! - ST_ISSIMPLE: Check if geometry has no self-intersections
//! - ST_SRID: Get the spatial reference identifier
//! - ST_BUFFER: Create buffer polygon around geometry
//! - ST_CENTROID: Return centroid of a geometry
//! - ST_ENVELOPE: Return bounding box of a geometry
//! - ST_CONVEXHULL: Return convex hull of a geometry
//! - ST_SIMPLIFY: Simplify geometry using Douglas-Peucker
//! - ST_REVERSE: Reverse coordinate order
//! - ST_BOUNDARY: Return boundary of a geometry
//! - ST_UNION: Union of two geometries
//! - ST_INTERSECTION: Intersection of two geometries
//! - ST_DIFFERENCE: Difference of two geometries
//! - ST_SYMDIFFERENCE: Symmetric difference of two geometries
//! - ST_MAKELINE: Create LineString from two points
//! - ST_MAKEPOLYGON: Create Polygon from closed LineString
//! - ST_MAKEENVELOPE: Create rectangular Polygon from bounds
//! - ST_COLLECT: Collect geometries into GeometryCollection
//! - ST_STARTPOINT: First point of a LineString
//! - ST_ENDPOINT: Last point of a LineString
//! - ST_POINTN: Nth point of a LineString
//! - ST_LINEINTERPOLATEPOINT: Point at fraction along LineString

mod helpers;
mod st_area;
mod st_asgeojson;
mod st_azimuth;
mod st_boundary;
mod st_buffer;
mod st_centroid;
mod st_collect;
mod st_contains;
mod st_convexhull;
mod st_coveredby;
mod st_covers;
mod st_crosses;
mod st_difference;
mod st_disjoint;
mod st_distance;
mod st_dwithin;
mod st_endpoint;
mod st_envelope;
mod st_equals;
mod st_geomfromgeojson;
mod st_geometrytype;
mod st_intersection;
mod st_intersects;
mod st_lineinterpolatepoint;
mod st_makeline;
mod st_makeenvelope;
mod st_makepolygon;
mod st_isclosed;
mod st_isempty;
mod st_issimple;
mod st_isvalid;
mod st_length;
mod st_makepoint;
mod st_numgeometries;
mod st_numpoints;
mod st_overlaps;
mod st_perimeter;
mod st_point;
mod st_pointn;
mod st_reverse;
mod st_simplify;
mod st_srid;
mod st_startpoint;
mod st_symdifference;
mod st_union;
mod st_touches;
mod st_within;
mod st_x;
mod st_y;

pub use st_area::StAreaFunction;
pub use st_asgeojson::StAsGeoJsonFunction;
pub use st_azimuth::StAzimuthFunction;
pub use st_boundary::StBoundaryFunction;
pub use st_buffer::StBufferFunction;
pub use st_centroid::StCentroidFunction;
pub use st_collect::StCollectFunction;
pub use st_contains::StContainsFunction;
pub use st_convexhull::StConvexHullFunction;
pub use st_coveredby::StCoveredByFunction;
pub use st_covers::StCoversFunction;
pub use st_crosses::StCrossesFunction;
pub use st_difference::StDifferenceFunction;
pub use st_disjoint::StDisjointFunction;
pub use st_distance::StDistanceFunction;
pub use st_dwithin::StDWithinFunction;
pub use st_endpoint::StEndPointFunction;
pub use st_envelope::StEnvelopeFunction;
pub use st_equals::StEqualsFunction;
pub use st_geomfromgeojson::StGeomFromGeoJsonFunction;
pub use st_geometrytype::StGeometryTypeFunction;
pub use st_intersection::StIntersectionFunction;
pub use st_intersects::StIntersectsFunction;
pub use st_lineinterpolatepoint::StLineInterpolatePointFunction;
pub use st_makeline::StMakeLineFunction;
pub use st_makeenvelope::StMakeEnvelopeFunction;
pub use st_makepolygon::StMakePolygonFunction;
pub use st_isclosed::StIsClosedFunction;
pub use st_isempty::StIsEmptyFunction;
pub use st_issimple::StIsSimpleFunction;
pub use st_isvalid::StIsValidFunction;
pub use st_length::StLengthFunction;
pub use st_makepoint::StMakePointFunction;
pub use st_numgeometries::StNumGeometriesFunction;
pub use st_numpoints::StNumPointsFunction;
pub use st_overlaps::StOverlapsFunction;
pub use st_perimeter::StPerimeterFunction;
pub use st_point::StPointFunction;
pub use st_pointn::StPointNFunction;
pub use st_reverse::StReverseFunction;
pub use st_simplify::StSimplifyFunction;
pub use st_srid::StSridFunction;
pub use st_startpoint::StStartPointFunction;
pub use st_symdifference::StSymDifferenceFunction;
pub use st_union::StUnionFunction;
pub use st_touches::StTouchesFunction;
pub use st_within::StWithinFunction;
pub use st_x::StXFunction;
pub use st_y::StYFunction;

#[cfg(test)]
mod tests;

use super::registry::FunctionRegistry;

/// Register all geospatial functions in the provided registry
///
/// This function is called during registry initialization to register
/// all PostGIS-compatible spatial functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    // Geometry constructors
    registry.register(Box::new(StPointFunction));
    registry.register(Box::new(StMakePointFunction));
    registry.register(Box::new(StGeomFromGeoJsonFunction));
    registry.register(Box::new(StMakeLineFunction));
    registry.register(Box::new(StMakePolygonFunction));
    registry.register(Box::new(StMakeEnvelopeFunction));
    registry.register(Box::new(StCollectFunction));

    // Output functions
    registry.register(Box::new(StAsGeoJsonFunction));

    // Distance functions
    registry.register(Box::new(StDistanceFunction));
    registry.register(Box::new(StDWithinFunction));

    // Measurement functions
    registry.register(Box::new(StAreaFunction));
    registry.register(Box::new(StLengthFunction));
    registry.register(Box::new(StPerimeterFunction));
    registry.register(Box::new(StAzimuthFunction));

    // Spatial predicates
    registry.register(Box::new(StContainsFunction));
    registry.register(Box::new(StWithinFunction));
    registry.register(Box::new(StIntersectsFunction));
    registry.register(Box::new(StTouchesFunction));
    registry.register(Box::new(StCrossesFunction));
    registry.register(Box::new(StOverlapsFunction));
    registry.register(Box::new(StDisjointFunction));
    registry.register(Box::new(StEqualsFunction));
    registry.register(Box::new(StCoversFunction));
    registry.register(Box::new(StCoveredByFunction));

    // Accessor functions
    registry.register(Box::new(StXFunction));
    registry.register(Box::new(StYFunction));
    registry.register(Box::new(StGeometryTypeFunction));
    registry.register(Box::new(StNumPointsFunction));
    registry.register(Box::new(StNumGeometriesFunction));
    registry.register(Box::new(StSridFunction));

    // Info/validation functions
    registry.register(Box::new(StIsValidFunction));
    registry.register(Box::new(StIsEmptyFunction));
    registry.register(Box::new(StIsClosedFunction));
    registry.register(Box::new(StIsSimpleFunction));

    // Geometry processing
    registry.register(Box::new(StBufferFunction));
    registry.register(Box::new(StCentroidFunction));
    registry.register(Box::new(StEnvelopeFunction));
    registry.register(Box::new(StConvexHullFunction));
    registry.register(Box::new(StSimplifyFunction));
    registry.register(Box::new(StReverseFunction));
    registry.register(Box::new(StBoundaryFunction));

    // Set operations
    registry.register(Box::new(StUnionFunction));
    registry.register(Box::new(StIntersectionFunction));
    registry.register(Box::new(StDifferenceFunction));
    registry.register(Box::new(StSymDifferenceFunction));

    // Line operations
    registry.register(Box::new(StStartPointFunction));
    registry.register(Box::new(StEndPointFunction));
    registry.register(Box::new(StPointNFunction));
    registry.register(Box::new(StLineInterpolatePointFunction));
}
