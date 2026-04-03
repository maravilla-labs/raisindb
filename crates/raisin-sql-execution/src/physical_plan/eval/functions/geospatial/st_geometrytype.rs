//! ST_GEOMETRYTYPE function - return geometry type as string

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Return the geometry type as a string (e.g., "ST_Point", "ST_Polygon")
///
/// # SQL Signature
/// `ST_GEOMETRYTYPE(geometry) -> TEXT`
pub struct StGeometryTypeFunction;

impl SqlFunction for StGeometryTypeFunction {
    fn name(&self) -> &str {
        "ST_GEOMETRYTYPE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_GEOMETRYTYPE(geometry) -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_GEOMETRYTYPE requires exactly 1 argument".to_string(),
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
                    "ST_GEOMETRYTYPE requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;
        Ok(Literal::Text(format!("ST_{}", geom_type)))
    }
}
