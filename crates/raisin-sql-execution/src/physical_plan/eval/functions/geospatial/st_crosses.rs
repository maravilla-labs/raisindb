//! ST_CROSSES function - check if geometries cross each other

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Contains, Intersects};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_linestring, geojson_to_point, geojson_to_polygon, get_geometry_type,
};

/// Check if two geometries cross (line enters and exits polygon, or lines cross)
///
/// # SQL Signature
/// `ST_CROSSES(geometry_a, geometry_b) -> BOOLEAN`
pub struct StCrossesFunction;

impl SqlFunction for StCrossesFunction {
    fn name(&self) -> &str {
        "ST_CROSSES"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_CROSSES(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_CROSSES requires exactly 2 arguments".to_string(),
            ));
        }

        let geom_a_val = eval_expr(&args[0], row)?;
        if matches!(geom_a_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom_b_val = eval_expr(&args[1], row)?;
        if matches!(geom_b_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom_a = match &geom_a_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_CROSSES requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_CROSSES requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        let crosses = match (type_a, type_b) {
            // Points never cross
            ("Point", _) | (_, "Point") => false,
            // LineString crosses Polygon: line intersects but is not fully contained
            ("LineString", "Polygon") => {
                let line = geojson_to_linestring(geom_a)?;
                let polygon = geojson_to_polygon(geom_b)?;
                polygon.intersects(&line) && !polygon.contains(&line)
            }
            ("Polygon", "LineString") => {
                let polygon = geojson_to_polygon(geom_a)?;
                let line = geojson_to_linestring(geom_b)?;
                polygon.intersects(&line) && !polygon.contains(&line)
            }
            // LineString crosses LineString
            ("LineString", "LineString") => {
                let line_a = geojson_to_linestring(geom_a)?;
                let line_b = geojson_to_linestring(geom_b)?;
                line_a.intersects(&line_b) && !line_a.contains(&line_b) && !line_b.contains(&line_a)
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_CROSSES not supported for {} and {}",
                    type_a, type_b
                )))
            }
        };

        Ok(Literal::Boolean(crosses))
    }
}
