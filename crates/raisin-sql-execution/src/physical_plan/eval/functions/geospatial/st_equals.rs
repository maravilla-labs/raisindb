//! ST_EQUALS function - check if geometries are topologically equal

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{
    geojson_to_linestring, geojson_to_point, geojson_to_polygon, get_geometry_type,
};

/// Check if two geometries are topologically equal
///
/// # SQL Signature
/// `ST_EQUALS(geometry_a, geometry_b) -> BOOLEAN`
pub struct StEqualsFunction;

impl SqlFunction for StEqualsFunction {
    fn name(&self) -> &str {
        "ST_EQUALS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_EQUALS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_EQUALS requires exactly 2 arguments".to_string(),
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
                    "ST_EQUALS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_EQUALS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // Different types are never equal
        if type_a != type_b {
            return Ok(Literal::Boolean(false));
        }

        // 1e-8 degrees ≈ 1.1mm at the equator — absorbs floating-point rounding
        const COORD_EPSILON: f64 = 1e-8;

        let equals = match type_a {
            "Point" => {
                let point_a = geojson_to_point(geom_a)?;
                let point_b = geojson_to_point(geom_b)?;
                (point_a.x() - point_b.x()).abs() < COORD_EPSILON
                    && (point_a.y() - point_b.y()).abs() < COORD_EPSILON
            }
            "LineString" => {
                let line_a = geojson_to_linestring(geom_a)?;
                let line_b = geojson_to_linestring(geom_b)?;
                if line_a.0.len() != line_b.0.len() {
                    false
                } else {
                    line_a.0.iter().zip(line_b.0.iter()).all(|(a, b)| {
                        (a.x - b.x).abs() < COORD_EPSILON && (a.y - b.y).abs() < COORD_EPSILON
                    })
                }
            }
            "Polygon" => {
                let poly_a = geojson_to_polygon(geom_a)?;
                let poly_b = geojson_to_polygon(geom_b)?;
                let ext_a = poly_a.exterior();
                let ext_b = poly_b.exterior();
                if ext_a.0.len() != ext_b.0.len() {
                    false
                } else {
                    ext_a.0.iter().zip(ext_b.0.iter()).all(|(a, b)| {
                        (a.x - b.x).abs() < COORD_EPSILON && (a.y - b.y).abs() < COORD_EPSILON
                    })
                }
            }
            _ => {
                // Fallback: compare JSON representations
                geom_a == geom_b
            }
        };

        Ok(Literal::Boolean(equals))
    }
}
