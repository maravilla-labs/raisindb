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
//! - ST_SETSRID: Relabel a geometry's CRS without moving its coordinates
//! - ST_TRANSFORM: Reproject a geometry into another CRS
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
//!
//! Three-dimensional support (altitude in metres above the WGS84 ellipsoid):
//! - ST_Z: Altitude of a Point
//! - ST_ZMIN / ST_ZMAX: Vertical extent of any geometry
//! - ST_NDIMS: Coordinate dimension, 2 or 3
//! - ST_FORCE2D / ST_FORCE3D: Drop or fill the third ordinate
//! - ST_3DDISTANCE / ST_3DDWITHIN: Distance including the vertical component
//!
//! Every function NOT in that list ignores Z, exactly as PostGIS's 2-D predicates
//! do. See `z_support` for why altitude never enters the `geo` pipeline.

mod axis_guard;
/// The single doorway between a SQL `Literal` and a `geo` geometry. Every ST_\*
/// function reads its arguments through this, which is why "accepts every geometry
/// type" is a property of one function rather than a claim repeated forty times.
mod convert;
mod crs;
mod helpers;
/// The one linear component the vertex accessors operate on.
mod line_access;
/// Measurement, and the one place the geodesic-versus-planar rule is decided.
mod measure;
/// `ST_BUFFER` / `ST_SIMPLIFY`: planar `geo` algorithms run so that their distance
/// argument means metres.
mod metric_ops;
/// Shared DE-9IM machinery: the single code path behind all ten topological
/// predicates and `ST_RELATE`.
mod relate;
/// The four overlay operations, over every pair of geometry types.
mod setops;
/// Real self-intersection detection, on a Bentley-Ottmann sweep.
mod simple;
mod st_3ddistance;
mod st_3ddwithin;
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
mod st_force2d;
mod st_force3d;
mod st_geometrytype;
mod st_geomfromgeojson;
mod st_intersection;
mod st_intersects;
mod st_isclosed;
mod st_isempty;
mod st_issimple;
mod st_isvalid;
mod st_isvalidreason;
mod st_length;
mod st_lineinterpolatepoint;
mod st_makeenvelope;
mod st_makeline;
mod st_makepoint;
mod st_makepolygon;
mod st_makevalid;
mod st_ndims;
mod st_numgeometries;
mod st_numpoints;
mod st_overlaps;
mod st_perimeter;
mod st_point;
mod st_pointn;
mod st_relate;
mod st_reverse;
mod st_setsrid;
mod st_simplify;
mod st_srid;
mod st_startpoint;
mod st_symdifference;
mod st_touches;
mod st_transform;
mod st_union;
mod st_within;
mod st_x;
mod st_y;
mod st_z;
mod st_zmax;
mod st_zmin;
/// OGC validity, its explanation, and its repair.
mod validate;
/// Recursive traversal of a `geo::Geometry` by dimension, so `Multi*` and nested
/// `GeometryCollection`s are handled in one place instead of forty.
mod walk;
mod z_support;

pub use st_3ddistance::St3DDistanceFunction;
pub use st_3ddwithin::St3DDWithinFunction;
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
pub use st_force2d::StForce2DFunction;
pub use st_force3d::StForce3DFunction;
pub use st_geometrytype::StGeometryTypeFunction;
pub use st_geomfromgeojson::StGeomFromGeoJsonFunction;
pub use st_intersection::StIntersectionFunction;
pub use st_intersects::StIntersectsFunction;
pub use st_isclosed::StIsClosedFunction;
pub use st_isempty::StIsEmptyFunction;
pub use st_issimple::StIsSimpleFunction;
pub use st_isvalid::StIsValidFunction;
pub use st_isvalidreason::StIsValidReasonFunction;
pub use st_length::StLengthFunction;
pub use st_lineinterpolatepoint::StLineInterpolatePointFunction;
pub use st_makeenvelope::StMakeEnvelopeFunction;
pub use st_makeline::StMakeLineFunction;
pub use st_makepoint::StMakePointFunction;
pub use st_makepolygon::StMakePolygonFunction;
pub use st_makevalid::StMakeValidFunction;
pub use st_ndims::StNDimsFunction;
pub use st_numgeometries::StNumGeometriesFunction;
pub use st_numpoints::StNumPointsFunction;
pub use st_overlaps::StOverlapsFunction;
pub use st_perimeter::StPerimeterFunction;
pub use st_point::StPointFunction;
pub use st_pointn::StPointNFunction;
pub use st_relate::StRelateFunction;
pub use st_reverse::StReverseFunction;
pub use st_setsrid::StSetSridFunction;
pub use st_simplify::StSimplifyFunction;
pub use st_srid::StSridFunction;
pub use st_startpoint::StStartPointFunction;
pub use st_symdifference::StSymDifferenceFunction;
pub use st_touches::StTouchesFunction;
pub use st_transform::StTransformFunction;
pub use st_union::StUnionFunction;
pub use st_within::StWithinFunction;
pub use st_x::StXFunction;
pub use st_y::StYFunction;
pub use st_z::StZFunction;
pub use st_zmax::StZMaxFunction;
pub use st_zmin::StZMinFunction;

#[cfg(test)]
mod tests;

// Area-specific unit test modules, declared here (the only place `mod` lines for
// this directory live) so that the areas owning them never have to edit each
// other's files. `tests_relate` (D1) and `tests_crs` (D3) are added by their
// owners; the declarations are pre-placed by whoever gets there first.
/// CRS resolution across a binary geometry pair — the rule that decides which
/// frame two operands are compared in.
#[cfg(test)]
mod tests_convert;
#[cfg(test)]
mod tests_crs;
#[cfg(test)]
mod tests_dim3;
/// The type-coverage matrix: every measurement, set-operation, processing and
/// accessor function against every geometry type its signature admits.
#[cfg(test)]
mod tests_ops;
#[cfg(test)]
mod tests_relate;

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
    // The raw DE-9IM matrix behind all ten predicates above, plus arbitrary
    // pattern matching for relationships none of them names.
    registry.register(Box::new(StRelateFunction));

    // Accessor functions
    registry.register(Box::new(StXFunction));
    registry.register(Box::new(StYFunction));
    registry.register(Box::new(StGeometryTypeFunction));
    registry.register(Box::new(StNumPointsFunction));
    registry.register(Box::new(StNumGeometriesFunction));
    registry.register(Box::new(StSridFunction));

    // Coordinate reference systems. ST_SETSRID relabels, ST_TRANSFORM moves —
    // see st_setsrid.rs for why conflating them is the classic multi-CRS bug.
    registry.register(Box::new(StSetSridFunction));
    registry.register(Box::new(StTransformFunction));

    // Info/validation functions
    registry.register(Box::new(StIsValidFunction));
    registry.register(Box::new(StIsEmptyFunction));
    registry.register(Box::new(StIsClosedFunction));
    registry.register(Box::new(StIsSimpleFunction));
    // ST_ISVALID answers whether, ST_ISVALIDREASON answers why, ST_MAKEVALID
    // repairs. All three run on geo's OGC validation rather than on the array-shape
    // check that let a self-intersecting bow-tie pass.
    registry.register(Box::new(StIsValidReasonFunction));
    registry.register(Box::new(StMakeValidFunction));

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

    // Three-dimensional support. Altitude is read off the GeoJSON
    // representation, never off a geo::Geometry (geo's Coord has no Z).
    registry.register(Box::new(StZFunction));
    registry.register(Box::new(StZMinFunction));
    registry.register(Box::new(StZMaxFunction));
    registry.register(Box::new(StNDimsFunction));
    registry.register(Box::new(StForce2DFunction));
    registry.register(Box::new(StForce3DFunction));
    registry.register(Box::new(St3DDistanceFunction));
    registry.register(Box::new(St3DDWithinFunction));

    // Line operations
    registry.register(Box::new(StStartPointFunction));
    registry.register(Box::new(StEndPointFunction));
    registry.register(Box::new(StPointNFunction));
    registry.register(Box::new(StLineInterpolatePointFunction));
}
