//! ST_DISJOINT function - check if geometries do not intersect

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Contains, Intersects};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_linestring, geojson_to_point, geojson_to_polygon, get_geometry_type,
};

/// Check if two geometries are disjoint (do not share any points)
///
/// # SQL Signature
/// `ST_DISJOINT(geometry_a, geometry_b) -> BOOLEAN`
///
/// This is the opposite of ST_INTERSECTS.
pub struct StDisjointFunction;

impl SqlFunction for StDisjointFunction {
    fn name(&self) -> &str {
        "ST_DISJOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_DISJOINT(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_DISJOINT requires exactly 2 arguments".to_string(),
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
                    "ST_DISJOINT requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_DISJOINT requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // Disjoint is the negation of intersects
        let intersects = match (type_a, type_b) {
            ("Point", "Point") => {
                let point_a = geojson_to_point(geom_a)?;
                let point_b = geojson_to_point(geom_b)?;
                point_a == point_b
            }
            ("Point", "Polygon") => {
                let point = geojson_to_point(geom_a)?;
                let polygon = geojson_to_polygon(geom_b)?;
                polygon.contains(&point) || polygon.exterior().intersects(&point)
            }
            ("Polygon", "Point") => {
                let polygon = geojson_to_polygon(geom_a)?;
                let point = geojson_to_point(geom_b)?;
                polygon.contains(&point) || polygon.exterior().intersects(&point)
            }
            ("Polygon", "Polygon") => {
                let polygon_a = geojson_to_polygon(geom_a)?;
                let polygon_b = geojson_to_polygon(geom_b)?;
                polygon_a.intersects(&polygon_b)
            }
            ("LineString", "Polygon") => {
                let line = geojson_to_linestring(geom_a)?;
                let polygon = geojson_to_polygon(geom_b)?;
                polygon.intersects(&line)
            }
            ("Polygon", "LineString") => {
                let polygon = geojson_to_polygon(geom_a)?;
                let line = geojson_to_linestring(geom_b)?;
                polygon.intersects(&line)
            }
            ("LineString", "LineString") => {
                let line_a = geojson_to_linestring(geom_a)?;
                let line_b = geojson_to_linestring(geom_b)?;
                line_a.intersects(&line_b)
            }
            ("Point", "LineString") => {
                let point = geojson_to_point(geom_a)?;
                let line = geojson_to_linestring(geom_b)?;
                line.intersects(&point)
            }
            ("LineString", "Point") => {
                let line = geojson_to_linestring(geom_a)?;
                let point = geojson_to_point(geom_b)?;
                line.intersects(&point)
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_DISJOINT not supported for {} and {}",
                    type_a, type_b
                )))
            }
        };

        Ok(Literal::Boolean(!intersects))
    }
}
