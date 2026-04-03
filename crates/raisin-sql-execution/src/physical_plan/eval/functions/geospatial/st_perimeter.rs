//! ST_PERIMETER function - calculate perimeter of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::HaversineLength;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_polygon, get_geometry_type};

/// Calculate the perimeter of a geometry in meters
///
/// # SQL Signature
/// `ST_PERIMETER(geometry) -> DOUBLE`
///
/// # Arguments
/// * `geometry` - A geometry value
///
/// # Returns
/// * Perimeter in meters (geodesic)
/// * 0.0 for Point and LineString geometries
/// * NULL if input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_PERIMETER(boundary) FROM regions
/// ```
///
/// # Notes
/// - Uses Haversine formula on the exterior ring
/// - Only meaningful for Polygon geometries
pub struct StPerimeterFunction;

impl SqlFunction for StPerimeterFunction {
    fn name(&self) -> &str {
        "ST_PERIMETER"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_PERIMETER(geometry) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_PERIMETER requires exactly 1 argument".to_string(),
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
                    "ST_PERIMETER requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        match geom_type {
            "Point" | "LineString" => Ok(Literal::Double(0.0)),
            "Polygon" => {
                let polygon = geojson_to_polygon(geom)?;
                let exterior = polygon.exterior().clone();
                let perimeter = exterior.haversine_length();
                Ok(Literal::Double(perimeter))
            }
            other => Err(Error::Validation(format!(
                "ST_PERIMETER not supported for geometry type: {}",
                other
            ))),
        }
    }
}
