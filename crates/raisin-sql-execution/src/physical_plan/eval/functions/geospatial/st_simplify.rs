//! ST_SIMPLIFY function - simplify geometry using Douglas-Peucker algorithm

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Simplify;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_linestring, geojson_to_polygon, get_geometry_type, linestring_to_geojson,
    polygon_to_geojson,
};

/// Simplify a geometry using the Douglas-Peucker algorithm
///
/// # SQL Signature
/// `ST_SIMPLIFY(geometry, tolerance) -> GEOMETRY`
pub struct StSimplifyFunction;

impl SqlFunction for StSimplifyFunction {
    fn name(&self) -> &str {
        "ST_SIMPLIFY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_SIMPLIFY(geometry, tolerance) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_SIMPLIFY requires exactly 2 arguments".to_string(),
            ));
        }

        let geom_val = eval_expr(&args[0], row)?;
        if matches!(geom_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let tol_val = eval_expr(&args[1], row)?;
        if matches!(tol_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom = match &geom_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_SIMPLIFY requires GEOMETRY as first argument".to_string(),
                ))
            }
        };

        let tolerance = match &tol_val {
            Literal::Double(d) => *d,
            Literal::Int(i) => *i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_SIMPLIFY requires numeric tolerance".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        match geom_type {
            "Point" => Ok(Literal::Geometry(geom.clone())),
            "LineString" => {
                let line = geojson_to_linestring(geom)?;
                let simplified = line.simplify(&tolerance);
                let result = linestring_to_geojson(&simplified);
                Ok(Literal::Geometry(result))
            }
            "Polygon" => {
                let polygon = geojson_to_polygon(geom)?;
                let simplified = polygon.simplify(&tolerance);
                let result = polygon_to_geojson(&simplified);
                Ok(Literal::Geometry(result))
            }
            other => Err(Error::Validation(format!(
                "ST_SIMPLIFY not supported for geometry type: {}",
                other
            ))),
        }
    }
}
