//! ST_COLLECT function - collect two geometries into a GeometryCollection

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Collect two geometries into a GeometryCollection
///
/// # SQL Signature
/// `ST_COLLECT(geometry1, geometry2) -> GEOMETRY`
pub struct StCollectFunction;

impl SqlFunction for StCollectFunction {
    fn name(&self) -> &str {
        "ST_COLLECT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_COLLECT(geometry1, geometry2) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_COLLECT requires exactly 2 arguments".to_string(),
            ));
        }

        let geom1_val = eval_expr(&args[0], row)?;
        if matches!(geom1_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom2_val = eval_expr(&args[1], row)?;
        if matches!(geom2_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom1 = match &geom1_val {
            Literal::Geometry(v) => v.clone(),
            Literal::JsonB(v) => v.clone(),
            _ => {
                return Err(Error::Validation(
                    "ST_COLLECT requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom2 = match &geom2_val {
            Literal::Geometry(v) => v.clone(),
            Literal::JsonB(v) => v.clone(),
            _ => {
                return Err(Error::Validation(
                    "ST_COLLECT requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let result = serde_json::json!({
            "type": "GeometryCollection",
            "geometries": [geom1, geom2]
        });

        Ok(Literal::Geometry(result))
    }
}
