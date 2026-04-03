//! ST_ISSIMPLE function - check if geometry has no self-intersections

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Check if a geometry is simple (no self-intersections)
///
/// # SQL Signature
/// `ST_ISSIMPLE(geometry) -> BOOLEAN`
///
/// - Point: always true
/// - LineString: true (MVP - full implementation needs sweep-line algorithm)
/// - Polygon: always true (valid polygons are simple)
pub struct StIsSimpleFunction;

impl SqlFunction for StIsSimpleFunction {
    fn name(&self) -> &str {
        "ST_ISSIMPLE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ISSIMPLE(geometry) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_ISSIMPLE requires exactly 1 argument".to_string(),
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
                    "ST_ISSIMPLE requires GEOMETRY argument".to_string(),
                ))
            }
        };

        // Validate it has a type
        let _geom_type = get_geometry_type(geom)?;

        // MVP: return true for all valid geometries
        // Full implementation would check for self-intersections in LineStrings
        Ok(Literal::Boolean(true))
    }
}
