//! ST_AREA function - calculate area of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::ChamberlainDuquetteArea;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_polygon, get_geometry_type};

/// Calculate the area of a geometry in square meters
///
/// # SQL Signature
/// `ST_AREA(geometry) -> DOUBLE`
///
/// # Arguments
/// * `geometry` - A geometry value
///
/// # Returns
/// * Area in square meters (geodesic)
/// * 0.0 for Point and LineString geometries
/// * NULL if input is NULL
///
/// # Examples
/// ```sql
/// SELECT ST_AREA(boundary) FROM regions
/// ```
///
/// # Notes
/// - Uses Chamberlain-Duquette formula for geodesic area on the sphere
/// - Returns unsigned area in square meters
pub struct StAreaFunction;

impl SqlFunction for StAreaFunction {
    fn name(&self) -> &str {
        "ST_AREA"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_AREA(geometry) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_AREA requires exactly 1 argument".to_string(),
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
                    "ST_AREA requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        match geom_type {
            "Point" | "LineString" => Ok(Literal::Double(0.0)),
            "Polygon" => {
                let polygon = geojson_to_polygon(geom)?;
                let area = polygon.chamberlain_duquette_unsigned_area();
                Ok(Literal::Double(area))
            }
            other => Err(Error::Validation(format!(
                "ST_AREA not supported for geometry type: {}",
                other
            ))),
        }
    }
}
