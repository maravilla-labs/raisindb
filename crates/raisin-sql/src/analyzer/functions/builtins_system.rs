use crate::analyzer::types::DataType;

use super::types::{FunctionCategory, FunctionRegistry, FunctionSignature};

/// Register system, geospatial, auth, and invocation built-in functions.
pub(super) fn register(registry: &mut FunctionRegistry) {
    register_system(registry);
    register_geospatial(registry);
    register_auth(registry);
    register_invoke(registry);
    register_locks(registry);
}

/// Atomic lock / inventory functions. Thin wrappers over the configured
/// `LockManager` (see `raisin-locks`). All side-effecting, hence non-deterministic.
fn register_locks(registry: &mut FunctionRegistry) {
    // RAISIN_TRY_ACQUIRE(key TEXT, ttl_ms BIGINT) -> JSONB {acquired, token, expires_at_ms}
    registry.register(FunctionSignature {
        name: "RAISIN_TRY_ACQUIRE".into(),
        params: vec![DataType::Text, DataType::BigInt],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::System,
    });
    // RAISIN_RELEASE(key TEXT, token BIGINT) -> BOOLEAN
    registry.register(FunctionSignature {
        name: "RAISIN_RELEASE".into(),
        params: vec![DataType::Text, DataType::BigInt],
        return_type: DataType::Boolean,
        is_deterministic: false,
        category: FunctionCategory::System,
    });
    // RAISIN_RENEW(key TEXT, token BIGINT, ttl_ms BIGINT) -> BOOLEAN
    registry.register(FunctionSignature {
        name: "RAISIN_RENEW".into(),
        params: vec![DataType::Text, DataType::BigInt, DataType::BigInt],
        return_type: DataType::Boolean,
        is_deterministic: false,
        category: FunctionCategory::System,
    });
    // RAISIN_CLAIM(pool TEXT, n BIGINT, capacity BIGINT) -> JSONB {claimed, remaining}
    registry.register(FunctionSignature {
        name: "RAISIN_CLAIM".into(),
        params: vec![DataType::Text, DataType::BigInt, DataType::BigInt],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::System,
    });
    // RAISIN_RELEASE_CLAIM(pool TEXT, n BIGINT) -> BIGINT (new remaining)
    registry.register(FunctionSignature {
        name: "RAISIN_RELEASE_CLAIM".into(),
        params: vec![DataType::Text, DataType::BigInt],
        return_type: DataType::BigInt,
        is_deterministic: false,
        category: FunctionCategory::System,
    });
}

/// System information functions (PostgreSQL compatibility).
fn register_system(registry: &mut FunctionRegistry) {
    registry.register(FunctionSignature {
        name: "VERSION".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_SCHEMA".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_DATABASE".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_USER".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "SESSION_USER".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });

    registry.register(FunctionSignature {
        name: "CURRENT_CATALOG".into(),
        params: vec![],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::System,
    });
}

/// Geospatial functions (PostGIS-compatible).
fn register_geospatial(registry: &mut FunctionRegistry) {
    // ST_POINT - Create a Point geometry from longitude and latitude
    registry.register(FunctionSignature {
        name: "ST_POINT".into(),
        params: vec![DataType::Double, DataType::Double],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_GEOMFROMGEOJSON".into(),
        params: vec![DataType::Text],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_ASGEOJSON".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ASGEOJSON(geometry, max_decimals) - round every ordinate on the way out,
    // which is the practical way to shrink a tile payload (5 places is about a
    // metre of longitude, 7 about a centimetre).
    registry.register(FunctionSignature {
        name: "ST_ASGEOJSON".into(),
        params: vec![DataType::Geometry, DataType::Int],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_DISTANCE".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_DWITHIN".into(),
        params: vec![DataType::Geometry, DataType::Geometry, DataType::Double],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_CONTAINS".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_WITHIN".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_INTERSECTS".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_X".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_Y".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_AREA - Calculate area of a geometry in square meters
    registry.register(FunctionSignature {
        name: "ST_AREA".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_LENGTH - Calculate length of a geometry in meters
    registry.register(FunctionSignature {
        name: "ST_LENGTH".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_PERIMETER - Calculate perimeter of a geometry in meters
    registry.register(FunctionSignature {
        name: "ST_PERIMETER".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Double,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_AZIMUTH - Calculate bearing between two points in radians
    registry.register(FunctionSignature {
        name: "ST_AZIMUTH".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        // Nullable: the azimuth from a point to itself is undefined, and the
        // arguments may not be single locations at all.
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_MAKEPOINT - Create a point geometry (PostGIS alias for ST_POINT)
    registry.register(FunctionSignature {
        name: "ST_MAKEPOINT".into(),
        params: vec![DataType::Double, DataType::Double],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // --- New spatial predicates ---

    // ST_TOUCHES - Check if geometries touch (share boundary, not interior)
    registry.register(FunctionSignature {
        name: "ST_TOUCHES".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_CROSSES - Check if geometries cross each other
    registry.register(FunctionSignature {
        name: "ST_CROSSES".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_OVERLAPS - Check if same-dimension geometries overlap
    registry.register(FunctionSignature {
        name: "ST_OVERLAPS".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_DISJOINT - Check if geometries do not intersect
    registry.register(FunctionSignature {
        name: "ST_DISJOINT".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_EQUALS - Check if geometries are topologically equal
    registry.register(FunctionSignature {
        name: "ST_EQUALS".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_COVERS - Check if geometry A covers geometry B
    registry.register(FunctionSignature {
        name: "ST_COVERS".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_COVEREDBY - Check if geometry A is covered by geometry B
    registry.register(FunctionSignature {
        name: "ST_COVEREDBY".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_RELATE - the raw DE-9IM matrix that backs every predicate above.
    // Two overloads, resolved by arity (see FunctionRegistry::resolve): the
    // 2-argument form returns the nine-character matrix, the 3-argument form
    // tests it against a DE-9IM pattern.
    registry.register(FunctionSignature {
        name: "ST_RELATE".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });
    registry.register(FunctionSignature {
        name: "ST_RELATE".into(),
        params: vec![DataType::Geometry, DataType::Geometry, DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // --- New accessor/info functions ---

    // ST_GEOMETRYTYPE - Get geometry type as string
    registry.register(FunctionSignature {
        name: "ST_GEOMETRYTYPE".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_NUMPOINTS - Get number of coordinate points
    registry.register(FunctionSignature {
        name: "ST_NUMPOINTS".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Int,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_NUMGEOMETRIES - Get number of sub-geometries
    registry.register(FunctionSignature {
        name: "ST_NUMGEOMETRIES".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Int,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_SRID - Get spatial reference identifier
    registry.register(FunctionSignature {
        name: "ST_SRID".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Int,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // --- Coordinate reference systems ---
    //
    // ST_SETSRID relabels; ST_TRANSFORM reprojects. Both take the target CRS as
    // an INTEGER EPSG code or as a TEXT form ('EPSG:2056', 'SRID=2056',
    // 'urn:ogc:def:crs:EPSG::2056'), which is two overloads each. The registry
    // resolves overloads by `params.len()` and then by coercion, so a second
    // signature of the same arity but a different parameter type is exactly how a
    // union-typed argument is expressed here.

    // ST_SETSRID - Reassign the CRS label without touching coordinates
    registry.register(FunctionSignature {
        name: "ST_SETSRID".into(),
        params: vec![DataType::Geometry, DataType::Int],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });
    registry.register(FunctionSignature {
        name: "ST_SETSRID".into(),
        params: vec![DataType::Geometry, DataType::Text],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_TRANSFORM - Reproject into another CRS
    registry.register(FunctionSignature {
        name: "ST_TRANSFORM".into(),
        params: vec![DataType::Geometry, DataType::Int],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });
    registry.register(FunctionSignature {
        name: "ST_TRANSFORM".into(),
        params: vec![DataType::Geometry, DataType::Text],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ISVALID - Check if geometry is valid
    registry.register(FunctionSignature {
        name: "ST_ISVALID".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ISEMPTY - Check if geometry is empty
    registry.register(FunctionSignature {
        name: "ST_ISEMPTY".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ISCLOSED - Check if geometry is closed
    registry.register(FunctionSignature {
        name: "ST_ISCLOSED".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ISSIMPLE - Check if geometry has no self-intersections
    registry.register(FunctionSignature {
        name: "ST_ISSIMPLE".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Boolean,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ISVALIDREASON - Explain why a geometry is invalid.
    //
    // The companion to ST_ISVALID: that one says whether, this one says why, and
    // ST_MAKEVALID repairs. Returns the literal "Valid Geometry" for valid input,
    // as PostGIS does, so diagnostic queries port unchanged.
    registry.register(FunctionSignature {
        name: "ST_ISVALIDREASON".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Text,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_MAKEVALID - Repair an invalid geometry, leaving valid input untouched.
    registry.register(FunctionSignature {
        name: "ST_MAKEVALID".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // --- Geometry processing functions ---

    // ST_BUFFER - Create buffer polygon around geometry at distance.
    //
    // The distance is METRES on a geographic CRS and native units on a projected
    // one. Two arities: the optional third argument is the number of straight
    // segments per quarter circle, so a smaller value gives a coarser outline.
    // Overloads work because `FunctionRegistry` keys a Vec of signatures per name
    // and resolves on argument count.
    registry.register(FunctionSignature {
        name: "ST_BUFFER".into(),
        params: vec![DataType::Geometry, DataType::Double],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_BUFFER".into(),
        params: vec![DataType::Geometry, DataType::Double, DataType::Int],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_CENTROID - Return the centroid of a geometry
    registry.register(FunctionSignature {
        name: "ST_CENTROID".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ENVELOPE - Return the bounding box as a Polygon
    registry.register(FunctionSignature {
        name: "ST_ENVELOPE".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_CONVEXHULL - Return the convex hull as a Polygon
    registry.register(FunctionSignature {
        name: "ST_CONVEXHULL".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_SIMPLIFY - Simplify geometry using Douglas-Peucker
    registry.register(FunctionSignature {
        name: "ST_SIMPLIFY".into(),
        params: vec![DataType::Geometry, DataType::Double],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_REVERSE - Reverse coordinate order of a geometry
    registry.register(FunctionSignature {
        name: "ST_REVERSE".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_BOUNDARY - Return the boundary of a geometry
    registry.register(FunctionSignature {
        name: "ST_BOUNDARY".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // --- Set operations ---

    // ST_UNION - Compute the union of two geometries
    registry.register(FunctionSignature {
        name: "ST_UNION".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_INTERSECTION - Compute the intersection of two geometries
    registry.register(FunctionSignature {
        name: "ST_INTERSECTION".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_DIFFERENCE - Compute the difference of two geometries
    registry.register(FunctionSignature {
        name: "ST_DIFFERENCE".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_SYMDIFFERENCE - Compute the symmetric difference of two geometries
    registry.register(FunctionSignature {
        name: "ST_SYMDIFFERENCE".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // --- Constructor functions ---

    // ST_MAKELINE - Create a LineString from two points
    registry.register(FunctionSignature {
        name: "ST_MAKELINE".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_MAKEPOLYGON - Create a Polygon from a closed LineString
    registry.register(FunctionSignature {
        name: "ST_MAKEPOLYGON".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_MAKEENVELOPE - Create a rectangular Polygon from bounds.
    //
    // Argument order is (xmin, ymin, xmax, ymax) = (west, south, east, north),
    // following the pinned (longitude, latitude) axis order. The optional fifth
    // argument LABELS the result's SRID; it does not reproject.
    registry.register(FunctionSignature {
        name: "ST_MAKEENVELOPE".into(),
        params: vec![
            DataType::Double,
            DataType::Double,
            DataType::Double,
            DataType::Double,
        ],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_MAKEENVELOPE".into(),
        params: vec![
            DataType::Double,
            DataType::Double,
            DataType::Double,
            DataType::Double,
            DataType::Int,
        ],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_COLLECT - Collect two geometries into a GeometryCollection
    registry.register(FunctionSignature {
        name: "ST_COLLECT".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // --- Line operations ---
    //
    // All four are NULLABLE on purpose: they are defined for a single linear
    // component, and a geometry column holding a mix of types must not abort the
    // query on its first polygon. That is PostGIS's behaviour too.

    // ST_STARTPOINT - Return the first point of a LineString
    registry.register(FunctionSignature {
        name: "ST_STARTPOINT".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Geometry)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ENDPOINT - Return the last point of a LineString
    registry.register(FunctionSignature {
        name: "ST_ENDPOINT".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Geometry)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_POINTN - Return the Nth point of a LineString.
    //
    // 1-based, and a negative index counts back from the end (-1 is the last
    // vertex). Out of range is NULL rather than an error, because paths in a column
    // have different lengths.
    registry.register(FunctionSignature {
        name: "ST_POINTN".into(),
        params: vec![DataType::Geometry, DataType::Int],
        return_type: DataType::Nullable(Box::new(DataType::Geometry)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_LINEINTERPOLATEPOINT - Point at fraction along a LineString, measured by
    // GEODESIC length on a geographic CRS rather than in raw coordinate units.
    registry.register(FunctionSignature {
        name: "ST_LINEINTERPOLATEPOINT".into(),
        params: vec![DataType::Geometry, DataType::Double],
        return_type: DataType::Nullable(Box::new(DataType::Geometry)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });
    // === Three-dimensional support ===
    //
    // Altitude is metres above the WGS84 ellipsoid. These are the ONLY functions
    // that look at the third ordinate; every other ST_* function is 2-D, exactly
    // as PostGIS's 2-D predicates are.
    //
    // The nullable return types are load-bearing: ST_Z of a 2-D point and
    // ST_3DDISTANCE involving a 2-D operand are SQL NULL, not errors, so a query
    // over a mixed-dimension column does not fail on its first flat row.

    // ST_Z - Altitude of a Point; NULL when 2-D or not a Point
    registry.register(FunctionSignature {
        name: "ST_Z".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_ZMIN / ST_ZMAX - Vertical extent of any geometry
    registry.register(FunctionSignature {
        name: "ST_ZMIN".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    registry.register(FunctionSignature {
        name: "ST_ZMAX".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_NDIMS - Coordinate dimension: 2 or 3
    registry.register(FunctionSignature {
        name: "ST_NDIMS".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Int,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_FORCE2D - Drop every altitude
    registry.register(FunctionSignature {
        name: "ST_FORCE2D".into(),
        params: vec![DataType::Geometry],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_FORCE3D - Fill in missing altitudes, keeping any already present
    registry.register(FunctionSignature {
        name: "ST_FORCE3D".into(),
        params: vec![DataType::Geometry, DataType::Double],
        return_type: DataType::Geometry,
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_3DDISTANCE - hypot(geodesic horizontal, vertical gap), in metres
    registry.register(FunctionSignature {
        name: "ST_3DDISTANCE".into(),
        params: vec![DataType::Geometry, DataType::Geometry],
        return_type: DataType::Nullable(Box::new(DataType::Double)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });

    // ST_3DDWITHIN - ST_3DDISTANCE <= d. No index support; pair it with a 2-D
    // ST_DWITHIN on the same radius for an index-driven candidate set, which is
    // safe because horizontal distance is a lower bound on 3-D distance.
    registry.register(FunctionSignature {
        name: "ST_3DDWITHIN".into(),
        params: vec![DataType::Geometry, DataType::Geometry, DataType::Double],
        return_type: DataType::Nullable(Box::new(DataType::Boolean)),
        is_deterministic: true,
        category: FunctionCategory::Geospatial,
    });
}

/// Authentication configuration functions (RAISIN_AUTH_*).
fn register_auth(registry: &mut FunctionRegistry) {
    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_CURRENT_USER".into(),
        params: vec![],
        return_type: DataType::Nullable(Box::new(DataType::Text)),
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_CURRENT_USER".into(),
        params: vec![],
        return_type: DataType::Nullable(Box::new(DataType::JsonB)),
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_CURRENT_WORKSPACE".into(),
        params: vec![],
        return_type: DataType::Nullable(Box::new(DataType::Text)),
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_HAS_PERMISSION".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_GET_SETTINGS".into(),
        params: vec![],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_UPDATE_SETTINGS".into(),
        params: vec![DataType::Text],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_ADD_PROVIDER".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::Text,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_UPDATE_PROVIDER".into(),
        params: vec![DataType::Text, DataType::Text],
        return_type: DataType::JsonB,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });

    registry.register(FunctionSignature {
        name: "RAISIN_AUTH_REMOVE_PROVIDER".into(),
        params: vec![DataType::Text],
        return_type: DataType::Boolean,
        is_deterministic: false,
        category: FunctionCategory::Auth,
    });
}

/// Function invocation from SQL: INVOKE() and INVOKE_SYNC().
fn register_invoke(registry: &mut FunctionRegistry) {
    for name in ["INVOKE", "INVOKE_SYNC"] {
        // 1-arg: INVOKE(path)
        registry.register(FunctionSignature {
            name: name.into(),
            params: vec![DataType::Text],
            return_type: DataType::JsonB,
            is_deterministic: false,
            category: FunctionCategory::System,
        });
        // 2-arg: INVOKE(path, input)
        registry.register(FunctionSignature {
            name: name.into(),
            params: vec![DataType::Text, DataType::JsonB],
            return_type: DataType::JsonB,
            is_deterministic: false,
            category: FunctionCategory::System,
        });
        // 3-arg: INVOKE(path, input, workspace)
        registry.register(FunctionSignature {
            name: name.into(),
            params: vec![DataType::Text, DataType::JsonB, DataType::Text],
            return_type: DataType::JsonB,
            is_deterministic: false,
            category: FunctionCategory::System,
        });
    }
}
