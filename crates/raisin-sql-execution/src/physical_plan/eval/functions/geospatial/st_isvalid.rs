//! ST_ISVALID function - check if geometry is valid

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Check if a geometry is valid (has type, coordinates, and values in range)
///
/// # SQL Signature
/// `ST_ISVALID(geometry) -> BOOLEAN`
pub struct StIsValidFunction;

impl SqlFunction for StIsValidFunction {
    fn name(&self) -> &str {
        "ST_ISVALID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ISVALID(geometry) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_ISVALID requires exactly 1 argument".to_string(),
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
                    "ST_ISVALID requires GEOMETRY argument".to_string(),
                ))
            }
        };

        // Check that the geometry has a valid type
        let type_result = get_geometry_type(geom);
        if type_result.is_err() {
            return Ok(Literal::Boolean(false));
        }

        let geom_type = type_result.unwrap();

        // Check that coordinates exist
        let has_coordinates = geom.get("coordinates").is_some();
        if !has_coordinates && geom_type != "GeometryCollection" {
            return Ok(Literal::Boolean(false));
        }

        // Validate based on type
        let valid = match geom_type {
            "Point" => {
                if let Some(coords) = geom.get("coordinates").and_then(|v| v.as_array()) {
                    coords.len() >= 2
                        && coords[0].as_f64().is_some()
                        && coords[1].as_f64().is_some()
                } else {
                    false
                }
            }
            "LineString" => {
                if let Some(coords) = geom.get("coordinates").and_then(|v| v.as_array()) {
                    coords.len() >= 2
                        && coords.iter().all(|c| {
                            c.as_array()
                                .map(|a| a.len() >= 2)
                                .unwrap_or(false)
                        })
                } else {
                    false
                }
            }
            "Polygon" => {
                if let Some(rings) = geom.get("coordinates").and_then(|v| v.as_array()) {
                    !rings.is_empty()
                        && rings.iter().all(|ring| {
                            ring.as_array()
                                .map(|r| {
                                    r.len() >= 4
                                        && r.iter().all(|c| {
                                            c.as_array()
                                                .map(|a| a.len() >= 2)
                                                .unwrap_or(false)
                                        })
                                })
                                .unwrap_or(false)
                        })
                } else {
                    false
                }
            }
            "MultiPoint" | "MultiLineString" | "MultiPolygon" => {
                geom.get("coordinates")
                    .and_then(|v| v.as_array())
                    .map(|arr| !arr.is_empty())
                    .unwrap_or(false)
            }
            "GeometryCollection" => {
                geom.get("geometries")
                    .and_then(|v| v.as_array())
                    .is_some()
            }
            _ => false,
        };

        Ok(Literal::Boolean(valid))
    }
}
