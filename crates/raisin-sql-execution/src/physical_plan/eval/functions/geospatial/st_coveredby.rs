//! ST_COVEREDBY function - check if geometry A is covered by geometry B

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Contains, Intersects};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_point, geojson_to_polygon, get_geometry_type};

/// Check if geometry A is covered by geometry B (no point of A is outside B)
///
/// # SQL Signature
/// `ST_COVEREDBY(geometry_a, geometry_b) -> BOOLEAN`
///
/// Inverse of ST_COVERS: ST_COVEREDBY(A, B) = ST_COVERS(B, A)
pub struct StCoveredByFunction;

impl SqlFunction for StCoveredByFunction {
    fn name(&self) -> &str {
        "ST_COVEREDBY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_COVEREDBY(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_COVEREDBY requires exactly 2 arguments".to_string(),
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
                    "ST_COVEREDBY requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let geom_b = match &geom_b_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_COVEREDBY requires GEOMETRY arguments".to_string(),
                ))
            }
        };

        let type_a = get_geometry_type(geom_a)?;
        let type_b = get_geometry_type(geom_b)?;

        // CoveredBy is the inverse of Covers: swap A and B
        let covered_by = match (type_a, type_b) {
            ("Point", "Polygon") => {
                let point = geojson_to_point(geom_a)?;
                let polygon = geojson_to_polygon(geom_b)?;
                polygon.contains(&point) || polygon.exterior().intersects(&point)
            }
            ("Polygon", "Polygon") => {
                let polygon_a = geojson_to_polygon(geom_a)?;
                let polygon_b = geojson_to_polygon(geom_b)?;
                polygon_b.contains(&polygon_a)
            }
            _ => {
                return Err(Error::Validation(format!(
                    "ST_COVEREDBY not supported for {} covered by {}. Currently supports: Point/Polygon covered by Polygon",
                    type_a, type_b
                )))
            }
        };

        Ok(Literal::Boolean(covered_by))
    }
}
