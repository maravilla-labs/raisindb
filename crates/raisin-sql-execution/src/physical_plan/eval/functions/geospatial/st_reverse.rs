//! ST_REVERSE function - reverse coordinate order of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Reverse the order of coordinates in a geometry
///
/// # SQL Signature
/// `ST_REVERSE(geometry) -> GEOMETRY`
pub struct StReverseFunction;

impl SqlFunction for StReverseFunction {
    fn name(&self) -> &str {
        "ST_REVERSE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_REVERSE(geometry) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_REVERSE requires exactly 1 argument".to_string(),
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
                    "ST_REVERSE requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        match geom_type {
            "Point" => {
                // Reversing a point is a no-op
                Ok(Literal::Geometry(geom.clone()))
            }
            "LineString" => {
                let coords = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        Error::Validation("LineString missing coordinates".to_string())
                    })?;
                let mut reversed = coords.clone();
                reversed.reverse();
                let result = serde_json::json!({
                    "type": "LineString",
                    "coordinates": reversed
                });
                Ok(Literal::Geometry(result))
            }
            "Polygon" => {
                let rings = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| Error::Validation("Polygon missing coordinates".to_string()))?;
                let reversed_rings: Vec<serde_json::Value> = rings
                    .iter()
                    .map(|ring| {
                        let mut coords = ring.as_array().cloned().unwrap_or_default();
                        coords.reverse();
                        serde_json::Value::Array(coords)
                    })
                    .collect();
                let result = serde_json::json!({
                    "type": "Polygon",
                    "coordinates": reversed_rings
                });
                Ok(Literal::Geometry(result))
            }
            other => Err(Error::Validation(format!(
                "ST_REVERSE not supported for geometry type: {}",
                other
            ))),
        }
    }
}
