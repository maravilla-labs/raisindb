//! ST_LENGTH function - calculate length of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::HaversineLength;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_linestring, geojson_to_polygon, get_geometry_type};

/// Calculate the length of a geometry in meters
///
/// # SQL Signature
/// `ST_LENGTH(geometry) -> DOUBLE`
///
/// # Arguments
/// * `geometry` - A geometry value
///
/// # Returns
/// * Length in meters (geodesic)
/// * 0.0 for Point geometries
/// * Perimeter for Polygon geometries (exterior ring length)
/// * NULL if input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_LENGTH(route) FROM paths
/// ```
///
/// # Notes
/// - Uses Haversine formula for geodesic length
/// - For Polygon, returns the perimeter (exterior ring length)
pub struct StLengthFunction;

impl SqlFunction for StLengthFunction {
    fn name(&self) -> &str {
        "ST_LENGTH"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_LENGTH(geometry) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_LENGTH requires exactly 1 argument".to_string(),
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
                    "ST_LENGTH requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        match geom_type {
            "Point" => Ok(Literal::Double(0.0)),
            "LineString" => {
                let linestring = geojson_to_linestring(geom)?;
                let length = linestring.haversine_length();
                Ok(Literal::Double(length))
            }
            "Polygon" => {
                let polygon = geojson_to_polygon(geom)?;
                let exterior = polygon.exterior().clone();
                let perimeter = exterior.haversine_length();
                Ok(Literal::Double(perimeter))
            }
            other => Err(Error::Validation(format!(
                "ST_LENGTH not supported for geometry type: {}",
                other
            ))),
        }
    }
}
