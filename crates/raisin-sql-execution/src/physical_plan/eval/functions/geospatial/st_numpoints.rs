//! ST_NUMPOINTS function - return number of coordinate points in a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Return the number of coordinate points in a geometry
///
/// # SQL Signature
/// `ST_NUMPOINTS(geometry) -> INTEGER`
pub struct StNumPointsFunction;

impl SqlFunction for StNumPointsFunction {
    fn name(&self) -> &str {
        "ST_NUMPOINTS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_NUMPOINTS(geometry) -> INTEGER"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_NUMPOINTS requires exactly 1 argument".to_string(),
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
                    "ST_NUMPOINTS requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        let count = match geom_type {
            "Point" => 1i64,
            "LineString" => {
                let coords = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        Error::Validation("Missing coordinates array".to_string())
                    })?;
                coords.len() as i64
            }
            "Polygon" => {
                let rings = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        Error::Validation("Missing coordinates array".to_string())
                    })?;
                rings
                    .iter()
                    .filter_map(|r| r.as_array())
                    .map(|r| r.len() as i64)
                    .sum()
            }
            "MultiPoint" => {
                let coords = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        Error::Validation("Missing coordinates array".to_string())
                    })?;
                coords.len() as i64
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_NUMPOINTS not supported for geometry type: {}",
                    geom_type
                )))
            }
        };

        Ok(Literal::Int(count as i32))
    }
}
