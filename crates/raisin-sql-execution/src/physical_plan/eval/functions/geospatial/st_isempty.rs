//! ST_ISEMPTY function - check if geometry has no coordinates

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Check if a geometry is empty (has no coordinates)
///
/// # SQL Signature
/// `ST_ISEMPTY(geometry) -> BOOLEAN`
pub struct StIsEmptyFunction;

impl SqlFunction for StIsEmptyFunction {
    fn name(&self) -> &str {
        "ST_ISEMPTY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ISEMPTY(geometry) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_ISEMPTY requires exactly 1 argument".to_string(),
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
                    "ST_ISEMPTY requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        let is_empty = match geom_type {
            "Point" => {
                // A point with coordinates is not empty
                geom.get("coordinates")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.is_empty())
                    .unwrap_or(true)
            }
            "LineString" | "MultiPoint" => {
                geom.get("coordinates")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.is_empty())
                    .unwrap_or(true)
            }
            "Polygon" | "MultiLineString" | "MultiPolygon" => {
                geom.get("coordinates")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.is_empty())
                    .unwrap_or(true)
            }
            "GeometryCollection" => {
                geom.get("geometries")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.is_empty())
                    .unwrap_or(true)
            }
            _ => true,
        };

        Ok(Literal::Boolean(is_empty))
    }
}
