//! ST_POINTN function - return the Nth point of a LineString

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{get_geometry_type, point_to_geojson};

/// Return the Nth point of a LineString (1-based index)
///
/// # SQL Signature
/// `ST_POINTN(geometry, n) -> GEOMETRY`
pub struct StPointNFunction;

impl SqlFunction for StPointNFunction {
    fn name(&self) -> &str {
        "ST_POINTN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_POINTN(geometry, n) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_POINTN requires exactly 2 arguments".to_string(),
            ));
        }

        let geom_val = eval_expr(&args[0], row)?;
        if matches!(geom_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let n_val = eval_expr(&args[1], row)?;
        if matches!(n_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom = match &geom_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_POINTN requires GEOMETRY as first argument".to_string(),
                ))
            }
        };

        let n = match &n_val {
            Literal::Int(i) => *i as usize,
            Literal::Double(d) => *d as usize,
            _ => {
                return Err(Error::Validation(
                    "ST_POINTN requires integer index".to_string(),
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

        // 1-based index
        if n == 0 || n > coords.len() {
            return Ok(Literal::Null);
        }

        let coord = coords[n - 1]
            .as_array()
            .ok_or_else(|| Error::Validation("Invalid coordinate".to_string()))?;
        let lon = coord[0].as_f64().unwrap_or(0.0);
        let lat = coord[1].as_f64().unwrap_or(0.0);

        Ok(Literal::Geometry(point_to_geojson(lon, lat)))
    }
}
