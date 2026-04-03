//! ST_NUMGEOMETRIES function - return number of sub-geometries

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Return the number of sub-geometries in a geometry
///
/// # SQL Signature
/// `ST_NUMGEOMETRIES(geometry) -> INTEGER`
///
/// For single geometries returns 1, for multi-geometries returns the count.
pub struct StNumGeometriesFunction;

impl SqlFunction for StNumGeometriesFunction {
    fn name(&self) -> &str {
        "ST_NUMGEOMETRIES"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_NUMGEOMETRIES(geometry) -> INTEGER"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_NUMGEOMETRIES requires exactly 1 argument".to_string(),
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
                    "ST_NUMGEOMETRIES requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        let count = match geom_type {
            "Point" | "LineString" | "Polygon" => 1i64,
            "MultiPoint" | "MultiLineString" | "MultiPolygon" | "GeometryCollection" => {
                let key = if geom_type == "GeometryCollection" {
                    "geometries"
                } else {
                    "coordinates"
                };
                geom.get(key)
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len() as i64)
                    .unwrap_or(0)
            }
            _ => 1,
        };

        Ok(Literal::Int(count as i32))
    }
}
