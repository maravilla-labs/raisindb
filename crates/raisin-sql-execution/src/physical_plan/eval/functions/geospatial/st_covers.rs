//! ST_COVERS function - check if geometry A covers geometry B

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Contains, Intersects};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_point, geojson_to_polygon, get_geometry_type};

/// Check if geometry A covers geometry B (no point of B is outside A)
///
/// # SQL Signature
/// `ST_COVERS(geometry_a, geometry_b) -> BOOLEAN`
///
/// Similar to ST_CONTAINS but includes boundary points.
pub struct StCoversFunction;

impl SqlFunction for StCoversFunction {
    fn name(&self) -> &str {
        "ST_COVERS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_COVERS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_COVERS requires exactly 2 arguments".to_string(),
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
                    "ST_COVERS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_COVERS requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // Covers includes boundary, so we check contains OR on boundary
        let covers = match (type_a, type_b) {
            ("Polygon", "Point") => {
                let polygon = geojson_to_polygon(geom_a)?;
                let point = geojson_to_point(geom_b)?;
                // Covers = contains OR point on boundary
                polygon.contains(&point) || polygon.exterior().intersects(&point)
            }
            ("Polygon", "Polygon") => {
                let polygon_a = geojson_to_polygon(geom_a)?;
                let polygon_b = geojson_to_polygon(geom_b)?;
                polygon_a.contains(&polygon_b)
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_COVERS not supported for {} covers {}. Currently supports: Polygon covers Point/Polygon",
                    type_a, type_b
                )))
            }
        };

        Ok(Literal::Boolean(covers))
    }
}
