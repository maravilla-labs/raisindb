//! Small shared helpers for the geospatial functions.
//!
//! # What used to be here, and why it is gone
//!
//! This file held three hand-rolled GeoJSON converters — `geojson_to_point`,
//! `geojson_to_linestring`, `geojson_to_polygon` — plus a raw-JSON coordinate
//! walker and a centroid extractor. Between them they covered exactly three of
//! the seven GeoJSON types, and **that**, not a missing library, is why `Multi*`
//! and `GeometryCollection` were unsupported across all forty-nine ST_\*
//! functions and why several predicates silently returned `false` for input they
//! did not recognise.
//!
//! They have all been deleted. The replacement is [`geojson_to_geom`], one call
//! into `raisin_geometry::to_geo`, which delegates to `geojson` 1.0's complete
//! `TryFrom` impls and therefore handles every type. Keeping the old converters
//! "for compatibility" is exactly how that drift would return, so there is
//! deliberately nothing left to fall back to.
//!
//! Most functions do not use this module at all: they take their arguments
//! through [`super::convert`] and their maths from [`super::measure`],
//! [`super::setops`], [`super::metric_ops`], [`super::simple`] or
//! [`super::validate`].
//!
//! # Altitude
//!
//! `geo_types::Coord` is strictly two dimensional, so altitude cannot survive
//! into the `geo` pipeline. It is not lost, it is *relocated*: [`geojson_to_geom`]
//! carries it as [`Geom::z_range`](raisin_geometry::Geom::z_range), and
//! `raisin_geometry::zdim` reads it straight off a `serde_json::Value`. Every
//! Z-aware function (`ST_Z`, `ST_ZMIN`/`ST_ZMAX`, `ST_NDIMS`,
//! `ST_FORCE2D`/`ST_FORCE3D`, `ST_3DDISTANCE`, `ST_3DDWITHIN`) uses those; every
//! other function is 2-D, as PostGIS's 2-D predicates are.

use raisin_error::Error;
use serde_json::Value;

/// Parse any GeoJSON value into a [`raisin_geometry::Geom`], preserving its SRID
/// and vertical extent.
///
/// Accepts every geometry type — including `Multi*` and `GeometryCollection` —
/// plus a Feature or FeatureCollection.
///
/// `schema_default` is the NodeType/workspace default SRID for a value that
/// carries no explicit `srid` member; pass `None` when the caller has no schema
/// context (a literal built by `ST_POINT`, for instance).
#[inline]
pub fn geojson_to_geom(
    value: &Value,
    schema_default: Option<u32>,
) -> Result<raisin_geometry::Geom, Error> {
    raisin_geometry::to_geo(value, schema_default).map_err(Into::into)
}

/// Get the geometry type name from a GeoJSON value, without parsing it.
///
/// For the few places that need only the discriminant — reporting it in an error
/// message, or `ST_GEOMETRYTYPE` — and would otherwise pay for a full conversion.
pub fn get_geometry_type(value: &Value) -> Result<&str, Error> {
    value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("GeoJSON missing 'type' field".to_string()))
}

/// Build a GeoJSON Point from a longitude and a latitude.
///
/// Axis order is `(longitude, latitude)`, pinned everywhere in RaisinDB. See
/// `super::axis_guard` for the check that catches the reversed call.
pub fn point_to_geojson(lon: f64, lat: f64) -> Value {
    serde_json::json!({
        "type": "Point",
        "coordinates": [lon, lat]
    })
}

/// Minimum distance between two GeoJSON geometries, in metres.
///
/// True shape-to-shape distance for every type pair, including `Multi*` and
/// `GeometryCollection`. The previous implementation fell back to
/// centroid-to-centroid for Polygon/Polygon and for anything `Multi*`, which
/// reported a positive distance between overlapping shapes; see
/// [`super::measure::distance`] for why fixing that required a projection rather
/// than a different `geo` trait.
///
/// Retained as a function of two `Value`s because `ST_3DDISTANCE` composes the
/// horizontal leg with a vertical one read off the JSON.
pub fn compute_haversine_distance(geom1: &Value, geom2: &Value) -> Result<f64, Error> {
    // One CRS rule for every binary geospatial function; see `super::convert`.
    let (a, b) = super::convert::geom_pair_values("ST_DISTANCE", geom1, geom2)?;
    super::measure::distance(&a, &b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_geometry_type_parses_including_the_ones_the_old_converters_could_not() {
        for value in [
            json!({"type":"Point","coordinates":[1.0,2.0]}),
            json!({"type":"MultiPoint","coordinates":[[1,2],[3,4]]}),
            json!({"type":"MultiLineString","coordinates":[[[0,0],[1,1]]]}),
            json!({"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}),
            json!({"type":"GeometryCollection","geometries":[]}),
        ] {
            geojson_to_geom(&value, None)
                .unwrap_or_else(|e| panic!("{} must parse: {e}", value["type"]));
        }
    }

    #[test]
    fn a_non_numeric_ordinate_is_rejected_rather_than_silently_zeroed() {
        let bad = json!({"type": "Point", "coordinates": ["not_a_number", 37.0]});
        assert!(geojson_to_geom(&bad, None).is_err());
    }

    #[test]
    fn a_truncated_position_is_rejected_rather_than_padded_with_zero() {
        let bad = json!({"type": "Point", "coordinates": [8.54]});
        assert!(geojson_to_geom(&bad, None).is_err());
    }

    #[test]
    fn a_coordinate_shape_that_contradicts_the_declared_type_is_rejected() {
        // The old validation checked only that `type` was a known name and that a
        // `coordinates` key existed, so this passed and failed much later.
        let bad = json!({"type": "Polygon", "coordinates": [[8.54, 47.37]]});
        assert!(geojson_to_geom(&bad, None).is_err());
    }

    #[test]
    fn overlapping_polygons_are_zero_metres_apart() {
        let a = json!({"type":"Polygon","coordinates":[[[0,0],[2,0],[2,2],[0,2],[0,0]]]});
        let b = json!({"type":"Polygon","coordinates":[[[1,1],[3,1],[3,3],[1,3],[1,1]]]});
        assert_eq!(
            compute_haversine_distance(&a, &b).unwrap(),
            0.0,
            "the centroid fallback used to report a positive distance here"
        );
    }

    #[test]
    fn distance_accepts_multi_geometries_as_input() {
        let a = json!({"type":"MultiPoint","coordinates":[[0.0,0.0],[10.0,0.0]]});
        let b = json!({"type":"Point","coordinates":[0.0,1.0]});
        let d = compute_haversine_distance(&a, &b).unwrap();
        // Nearest member is (0,0), one degree of latitude away.
        assert!((1.10e5..1.12e5).contains(&d), "{d}");
    }

    #[test]
    fn a_missing_type_member_names_the_problem() {
        let err = get_geometry_type(&json!({"coordinates": [1, 2]})).unwrap_err();
        assert!(err.to_string().contains("type"), "{err}");
    }
}
