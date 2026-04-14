//! ST_ISCLOSED function - check if geometry is closed

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Check if a geometry is closed (first point equals last point)
///
/// # SQL Signature
/// `ST_ISCLOSED(geometry) -> BOOLEAN`
///
/// - Point: always true
/// - LineString: first point == last point
/// - Polygon: always true (rings are closed by definition)
pub struct StIsClosedFunction;

impl SqlFunction for StIsClosedFunction {
    fn name(&self) -> &str {
        "ST_ISCLOSED"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ISCLOSED(geometry) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_ISCLOSED requires exactly 1 argument".to_string(),
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
                    "ST_ISCLOSED requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        let is_closed = match geom_type {
            "Point" => true,
            "Polygon" => true, // Polygon rings are closed by definition
            "LineString" => {
                let coords = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| Error::Validation("Missing coordinates array".to_string()))?;
                if coords.len() < 2 {
                    false
                } else {
                    let first = &coords[0];
                    let last = &coords[coords.len() - 1];
                    // Compare coordinate arrays
                    first == last
                }
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_ISCLOSED not supported for geometry type: {}",
                    geom_type
                )))
            }
        };

        Ok(Literal::Boolean(is_closed))
    }
}
