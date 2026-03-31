//! ST_GEOMFROMGEOJSON function - parse GeoJSON text to geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Parse GeoJSON text into a geometry
///
/// # SQL Signature
/// `ST_GEOMFROMGEOJSON(geojson_text) -> GEOMETRY`
///
/// # Arguments
/// * `geojson_text` - GeoJSON string (RFC 7946)
///
/// # Returns
/// * Geometry parsed from GeoJSON
/// * NULL if input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_GEOMFROMGEOJSON('{"type":"Point","coordinates":[-122.4194,37.7749]}')
/// SELECT ST_GEOMFROMGEOJSON('{"type":"Polygon","coordinates":[[[-122.5,37.7],[-122.3,37.7],[-122.3,37.8],[-122.5,37.8],[-122.5,37.7]]]}')
/// ```
///
/// # Notes
/// - Accepts all GeoJSON geometry types: Point, LineString, Polygon, MultiPoint, MultiLineString, MultiPolygon, GeometryCollection
/// - Coordinates must be in WGS84 (EPSG:4326)
pub struct StGeomFromGeoJsonFunction;

impl SqlFunction for StGeomFromGeoJsonFunction {
    fn name(&self) -> &str {
        "ST_GEOMFROMGEOJSON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_GEOMFROMGEOJSON(geojson_text) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_GEOMFROMGEOJSON requires exactly 1 argument".to_string(),
            ));
        }

        let text_val = eval_expr(&args[0], row)?;

        if matches!(text_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geojson_str = match text_val {
            Literal::Text(s) => s,
            Literal::JsonB(v) => {
                // Already JSON, validate it's a geometry
                validate_geometry(&v)?;
                return Ok(Literal::Geometry(v));
            }
            _ => {
                return Err(Error::Validation(
                    "ST_GEOMFROMGEOJSON requires TEXT or JSONB input".to_string(),
                ))
            }
        };

        // Parse JSON
        let geojson: serde_json::Value = serde_json::from_str(&geojson_str)
            .map_err(|e| Error::Validation(format!("Invalid GeoJSON: {}", e)))?;

        // Validate it's a valid geometry
        validate_geometry(&geojson)?;

        Ok(Literal::Geometry(geojson))
    }
}

/// Validate that a JSON value is a valid GeoJSON geometry
fn validate_geometry(value: &serde_json::Value) -> Result<(), Error> {
    let geom_type = value
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("GeoJSON missing 'type' field".to_string()))?;

    // Check for valid geometry types
    let valid_types = [
        "Point",
        "LineString",
        "Polygon",
        "MultiPoint",
        "MultiLineString",
        "MultiPolygon",
        "GeometryCollection",
    ];

    if !valid_types.contains(&geom_type) {
        return Err(Error::Validation(format!(
            "Invalid GeoJSON geometry type: {}. Expected one of: {}",
            geom_type,
            valid_types.join(", ")
        )));
    }

    // Check for coordinates (except GeometryCollection)
    if geom_type == "GeometryCollection" {
        value.get("geometries").ok_or_else(|| {
            Error::Validation("GeometryCollection missing 'geometries' field".to_string())
        })?;
    } else {
        value.get("coordinates").ok_or_else(|| {
            Error::Validation(format!("{} missing 'coordinates' field", geom_type))
        })?;
    }

    Ok(())
}
