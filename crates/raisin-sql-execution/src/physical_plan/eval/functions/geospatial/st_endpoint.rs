//! ST_ENDPOINT function - return the last point of a LineString

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{get_geometry_type, point_to_geojson};

/// Return the last point of a LineString
///
/// # SQL Signature
/// `ST_ENDPOINT(geometry) -> GEOMETRY`
pub struct StEndPointFunction;

impl SqlFunction for StEndPointFunction {
    fn name(&self) -> &str {
        "ST_ENDPOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ENDPOINT(geometry) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_ENDPOINT requires exactly 1 argument".to_string(),
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
                    "ST_ENDPOINT requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;
        if geom_type != "LineString" {
            return Ok(Literal::Null);
        }

        let coords = geom
            .get("coordinates")
            .and_then(|v| v.as_array())
            .ok_or_else(|| Error::Validation("LineString missing coordinates".to_string()))?;

        if coords.is_empty() {
            return Ok(Literal::Null);
        }

        let last = coords[coords.len() - 1]
            .as_array()
            .ok_or_else(|| Error::Validation("Invalid coordinate".to_string()))?;
        let lon = last[0].as_f64().unwrap_or(0.0);
        let lat = last[1].as_f64().unwrap_or(0.0);

        Ok(Literal::Geometry(point_to_geojson(lon, lat)))
    }
}
