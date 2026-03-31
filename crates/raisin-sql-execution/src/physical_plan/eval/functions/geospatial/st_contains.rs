//! ST_CONTAINS function - check if geometry A contains geometry B

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Contains;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_point, geojson_to_polygon, get_geometry_type};

/// Check if geometry A completely contains geometry B
///
/// # SQL Signature
/// `ST_CONTAINS(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Arguments
/// * `geometry_a` - Container geometry (typically a Polygon)
/// * `geometry_b` - Contained geometry (typically a Point)
///
/// # Returns
/// * TRUE if A contains B
/// * FALSE otherwise
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// -- Find points within a delivery zone
/// SELECT * FROM orders
/// WHERE ST_CONTAINS(
///     delivery_zone,
///     delivery_location
/// )
///
/// -- Check if a store is within city limits
/// SELECT ST_CONTAINS(
///     ST_GEOMFROMGEOJSON('{"type":"Polygon","coordinates":[[[-122.5,37.7],[-122.3,37.7],[-122.3,37.8],[-122.5,37.8],[-122.5,37.7]]]}'),
///     store.location
/// )
/// ```
///
/// # Notes
/// - ST_CONTAINS(A, B) is the inverse of ST_WITHIN(B, A)
/// - Currently supports: Polygon contains Point
/// - Other combinations will be added as needed
pub struct StContainsFunction;

impl SqlFunction for StContainsFunction {
    fn name(&self) -> &str {
        "ST_CONTAINS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_CONTAINS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_CONTAINS requires exactly 2 arguments".to_string(),
            ));
        }

        // Evaluate geometries
        let geom_a_val = eval_expr(&args[0], row)?;
        if matches!(geom_a_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom_b_val = eval_expr(&args[1], row)?;
        if matches!(geom_b_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Extract GeoJSON values
        let geom_a = match &geom_a_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_CONTAINS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_CONTAINS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // Handle supported geometry combinations
        let contains = match (type_a, type_b) {
            ("Polygon", "Point") => {
                let polygon = geojson_to_polygon(geom_a)?;
                let point = geojson_to_point(geom_b)?;
                polygon.contains(&point)
            }
            ("Polygon", "Polygon") => {
                // Polygon contains Polygon: all points of B must be inside A
                // For simplicity, we use the geo crate's contains
                let polygon_a = geojson_to_polygon(geom_a)?;
                let polygon_b = geojson_to_polygon(geom_b)?;
                polygon_a.contains(&polygon_b)
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_CONTAINS not supported for {} contains {}. Currently supports: Polygon contains Point/Polygon",
                    type_a, type_b
                )))
            }
        };

        Ok(Literal::Boolean(contains))
    }
}
