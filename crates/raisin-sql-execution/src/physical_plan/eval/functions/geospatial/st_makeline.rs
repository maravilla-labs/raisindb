//! ST_MAKELINE function - create a LineString from two points

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::geojson_to_point;

/// Create a LineString from two Point geometries
///
/// # SQL Signature
/// `ST_MAKELINE(point1, point2) -> GEOMETRY`
pub struct StMakeLineFunction;

impl SqlFunction for StMakeLineFunction {
    fn name(&self) -> &str {
        "ST_MAKELINE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_MAKELINE(point1, point2) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_MAKELINE requires exactly 2 arguments".to_string(),
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
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_MAKELINE requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom2 = match &geom2_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_MAKELINE requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let p1 = geojson_to_point(geom1)?;
        let p2 = geojson_to_point(geom2)?;

        let result = serde_json::json!({
            "type": "LineString",
            "coordinates": [
                [p1.x(), p1.y()],
                [p2.x(), p2.y()]
            ]
        });

        Ok(Literal::Geometry(result))
    }
}
