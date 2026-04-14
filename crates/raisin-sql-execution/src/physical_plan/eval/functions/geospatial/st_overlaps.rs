//! ST_OVERLAPS function - check if geometries overlap

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Contains, Intersects};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_linestring, geojson_to_point, geojson_to_polygon, get_geometry_type,
};

/// Check if two same-dimension geometries overlap (share space but neither contains the other)
///
/// # SQL Signature
/// `ST_OVERLAPS(geometry_a, geometry_b) -> BOOLEAN`
pub struct StOverlapsFunction;

impl SqlFunction for StOverlapsFunction {
    fn name(&self) -> &str {
        "ST_OVERLAPS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_OVERLAPS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_OVERLAPS requires exactly 2 arguments".to_string(),
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
                    "ST_OVERLAPS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_OVERLAPS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // Overlaps only applies to same-dimension geometries
        let overlaps = match (type_a, type_b) {
            ("Point", "Point") => {
                let point_a = geojson_to_point(geom_a)?;
                let point_b = geojson_to_point(geom_b)?;
                // Points overlap if they are the same point
                point_a == point_b
            }
            ("Polygon", "Polygon") => {
                let polygon_a = geojson_to_polygon(geom_a)?;
                let polygon_b = geojson_to_polygon(geom_b)?;
                // Overlap = intersect but neither contains the other
                polygon_a.intersects(&polygon_b)
                    && !polygon_a.contains(&polygon_b)
                    && !polygon_b.contains(&polygon_a)
            }
            ("LineString", "LineString") => {
                let line_a = geojson_to_linestring(geom_a)?;
                let line_b = geojson_to_linestring(geom_b)?;
                line_a.intersects(&line_b) && !line_a.contains(&line_b) && !line_b.contains(&line_a)
            }
            _ => {
                // Different dimensions cannot overlap by definition
                false
            }
        };

        Ok(Literal::Boolean(overlaps))
    }
}
