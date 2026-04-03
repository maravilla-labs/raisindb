//! ST_SRID function - return the spatial reference identifier

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Return the SRID (Spatial Reference Identifier) of a geometry
///
/// # SQL Signature
/// `ST_SRID(geometry) -> INTEGER`
///
/// RaisinDB always uses WGS84 (EPSG:4326), so this always returns 4326.
pub struct StSridFunction;

impl SqlFunction for StSridFunction {
    fn name(&self) -> &str {
        "ST_SRID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_SRID(geometry) -> INTEGER"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_SRID requires exactly 1 argument".to_string(),
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
                    "ST_SRID requires GEOMETRY argument".to_string(),
                ))
            }
        };

        // Validate it's a valid geometry
        let _geom_type = get_geometry_type(geom)?;

        // RaisinDB always uses WGS84
        Ok(Literal::Int(4326))
    }
}
