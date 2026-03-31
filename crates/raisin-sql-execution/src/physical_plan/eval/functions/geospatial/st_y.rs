//! ST_Y function - get Y coordinate (latitude) of a point

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_point, get_geometry_type};

/// Get the Y coordinate (latitude) of a Point geometry
///
/// # SQL Signature
/// `ST_Y(point) -> DOUBLE`
///
/// # Arguments
/// * `point` - A Point geometry
///
/// # Returns
/// * Y coordinate (latitude in WGS84)
/// * NULL if input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_Y(location) AS latitude FROM stores
/// SELECT ST_Y(ST_POINT(-122.4194, 37.7749))
/// -- Returns: 37.7749
/// ```
///
/// # Notes
/// - Only works with Point geometries
/// - Returns latitude for WGS84 coordinates
pub struct StYFunction;

impl SqlFunction for StYFunction {
    fn name(&self) -> &str {
        "ST_Y"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_Y(point) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_Y requires exactly 1 argument".to_string(),
            ));
        }

        let geom_val = eval_expr(&args[0], row)?;

        if matches!(geom_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom = match &geom_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_Y requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;
        if geom_type != "Point" {
            return Err(Error::Validation(format!(
                "ST_Y requires Point geometry, got {}",
                geom_type
            )));
        }

        let point = geojson_to_point(geom)?;
        Ok(Literal::Double(point.y()))
    }
}
