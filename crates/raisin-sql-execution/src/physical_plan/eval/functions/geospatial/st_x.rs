//! ST_X function - get X coordinate (longitude) of a point

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_point, get_geometry_type};

/// Get the X coordinate (longitude) of a Point geometry
///
/// # SQL Signature
/// `ST_X(point) -> DOUBLE`
///
/// # Arguments
/// * `point` - A Point geometry
///
/// # Returns
/// * X coordinate (longitude in WGS84)
/// * NULL if input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_X(location) AS longitude FROM stores
/// SELECT ST_X(ST_POINT(-122.4194, 37.7749))
/// -- Returns: -122.4194
/// ```
///
/// # Notes
/// - Only works with Point geometries
/// - Returns longitude for WGS84 coordinates
pub struct StXFunction;

impl SqlFunction for StXFunction {
    fn name(&self) -> &str {
        "ST_X"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_X(point) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_X requires exactly 1 argument".to_string(),
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
                    "ST_X requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;
        if geom_type != "Point" {
            return Err(Error::Validation(format!(
                "ST_X requires Point geometry, got {}",
                geom_type
            )));
        }

        let point = geojson_to_point(geom)?;
        Ok(Literal::Double(point.x()))
    }
}
