//! ST_CENTROID function - return the centroid of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{get_centroid, point_to_geojson};

/// Return the centroid of a geometry as a Point
///
/// # SQL Signature
/// `ST_CENTROID(geometry) -> GEOMETRY`
pub struct StCentroidFunction;

impl SqlFunction for StCentroidFunction {
    fn name(&self) -> &str {
        "ST_CENTROID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_CENTROID(geometry) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_CENTROID requires exactly 1 argument".to_string(),
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
                    "ST_CENTROID requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let centroid = get_centroid(geom)?;
        let result = point_to_geojson(centroid.x(), centroid.y());

        Ok(Literal::Geometry(result))
    }
}
