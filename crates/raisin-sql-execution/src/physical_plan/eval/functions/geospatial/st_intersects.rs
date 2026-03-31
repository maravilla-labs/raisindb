//! ST_INTERSECTS function - check if geometries intersect

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Contains, Intersects};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_linestring, geojson_to_point, geojson_to_polygon, get_geometry_type,
};

/// Check if two geometries have any points in common
///
/// # SQL Signature
/// `ST_INTERSECTS(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Arguments
/// * `geometry_a` - First geometry
/// * `geometry_b` - Second geometry
///
/// # Returns
/// * TRUE if geometries share any points
/// * FALSE if geometries are disjoint
/// * NULL if any input is NULL
///
/// # Examples
/// ```sql
/// -- Find routes that cross a boundary
/// SELECT * FROM routes
/// WHERE ST_INTERSECTS(route_line, boundary_polygon)
///
/// -- Check if delivery zone overlaps with another
/// SELECT ST_INTERSECTS(zone_a.boundary, zone_b.boundary)
/// FROM zones zone_a, zones zone_b
/// ```
///
/// # Notes
/// - More general than ST_CONTAINS/ST_WITHIN
/// - Returns TRUE if geometries touch, overlap, or one contains the other
/// - Supports: Point-Polygon, Polygon-Polygon, LineString-Polygon
pub struct StIntersectsFunction;

impl SqlFunction for StIntersectsFunction {
    fn name(&self) -> &str {
        "ST_INTERSECTS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_INTERSECTS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_INTERSECTS requires exactly 2 arguments".to_string(),
            ));
        }

        // Evaluate geometries
        let geom_a_val = eval_expr(&args[0], row)?;
        if matches!(geom_a_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom_b_val = eval_expr(&args[1], row)?;
        if matches!(geom_b_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Extract GeoJSON values
        let geom_a = match &geom_a_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_INTERSECTS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_INTERSECTS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // Handle supported geometry combinations
        let intersects = match (type_a, type_b) {
            // Point intersects Polygon (same as Point within Polygon)
            ("Point", "Polygon") => {
                let point = geojson_to_point(geom_a)?;
                let polygon = geojson_to_polygon(geom_b)?;
                polygon.contains(&point)
            }
            // Polygon intersects Point
            ("Polygon", "Point") => {
                let polygon = geojson_to_polygon(geom_a)?;
                let point = geojson_to_point(geom_b)?;
                polygon.contains(&point)
            }
            // Polygon intersects Polygon
            ("Polygon", "Polygon") => {
                let polygon_a = geojson_to_polygon(geom_a)?;
                let polygon_b = geojson_to_polygon(geom_b)?;
                polygon_a.intersects(&polygon_b)
            }
            // LineString intersects Polygon
            ("LineString", "Polygon") => {
                let line = geojson_to_linestring(geom_a)?;
                let polygon = geojson_to_polygon(geom_b)?;
                polygon.intersects(&line)
            }
            // Polygon intersects LineString
            ("Polygon", "LineString") => {
                let polygon = geojson_to_polygon(geom_a)?;
                let line = geojson_to_linestring(geom_b)?;
                polygon.intersects(&line)
            }
            // Point intersects Point (exact match)
            ("Point", "Point") => {
                let point_a = geojson_to_point(geom_a)?;
                let point_b = geojson_to_point(geom_b)?;
                point_a == point_b
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_INTERSECTS not supported for {} and {}. Supported: Point/Polygon/LineString combinations",
                    type_a, type_b
                )))
            }
        };

        Ok(Literal::Boolean(intersects))
    }
}
