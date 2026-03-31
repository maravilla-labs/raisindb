//! ST_WITHIN function - check if geometry A is within geometry B

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Contains;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_point, geojson_to_polygon, get_geometry_type};

/// Check if geometry A is completely within geometry B
///
/// # SQL Signature
/// `ST_WITHIN(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Arguments
/// * `geometry_a` - The geometry to test (typically a Point)
/// * `geometry_b` - The container geometry (typically a Polygon)
///
/// # Returns
/// * TRUE if A is within B
/// * FALSE otherwise
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// -- Find stores within city limits
/// SELECT * FROM stores
/// WHERE ST_WITHIN(location, city_boundary)
///
/// -- Check if order is in delivery zone
/// SELECT ST_WITHIN(
///     order.location,
///     zone.boundary
/// ) AS in_zone
/// FROM orders
/// ```
///
/// # Notes
/// - ST_WITHIN(A, B) is equivalent to ST_CONTAINS(B, A)
/// - Currently supports: Point within Polygon
/// - Other combinations will be added as needed
pub struct StWithinFunction;

impl SqlFunction for StWithinFunction {
    fn name(&self) -> &str {
        "ST_WITHIN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_WITHIN(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_WITHIN requires exactly 2 arguments".to_string(),
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
                    "ST_WITHIN requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_WITHIN requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // Handle supported geometry combinations
        // ST_WITHIN(A, B) is the same as ST_CONTAINS(B, A)
        let within = match (type_a, type_b) {
            ("Point", "Polygon") => {
                let point = geojson_to_point(geom_a)?;
                let polygon = geojson_to_polygon(geom_b)?;
                polygon.contains(&point)
            }
            ("Polygon", "Polygon") => {
                let polygon_a = geojson_to_polygon(geom_a)?;
                let polygon_b = geojson_to_polygon(geom_b)?;
                polygon_b.contains(&polygon_a)
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_WITHIN not supported for {} within {}. Currently supports: Point/Polygon within Polygon",
                    type_a, type_b
                )))
            }
        };

        Ok(Literal::Boolean(within))
    }
}
