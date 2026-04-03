//! ST_BOUNDARY function - return the boundary of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Return the boundary of a geometry
///
/// # SQL Signature
/// `ST_BOUNDARY(geometry) -> GEOMETRY`
///
/// - Point -> empty GeometryCollection
/// - LineString -> MultiPoint of start and end points
/// - Polygon -> exterior ring as LineString
pub struct StBoundaryFunction;

impl SqlFunction for StBoundaryFunction {
    fn name(&self) -> &str {
        "ST_BOUNDARY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_BOUNDARY(geometry) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_BOUNDARY requires exactly 1 argument".to_string(),
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
                    "ST_BOUNDARY requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        match geom_type {
            "Point" => {
                let result = serde_json::json!({
                    "type": "GeometryCollection",
                    "geometries": []
                });
                Ok(Literal::Geometry(result))
            }
            "LineString" => {
                let coords = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        Error::Validation("LineString missing coordinates".to_string())
                    })?;
                if coords.is_empty() {
                    return Ok(Literal::Geometry(serde_json::json!({
                        "type": "GeometryCollection",
                        "geometries": []
                    })));
                }
                let start = &coords[0];
                let end = &coords[coords.len() - 1];
                let result = serde_json::json!({
                    "type": "MultiPoint",
                    "coordinates": [start, end]
                });
                Ok(Literal::Geometry(result))
            }
            "Polygon" => {
                let rings = geom
                    .get("coordinates")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| {
                        Error::Validation("Polygon missing coordinates".to_string())
                    })?;
                if rings.is_empty() {
                    return Ok(Literal::Null);
                }
                let exterior = &rings[0];
                let result = serde_json::json!({
                    "type": "LineString",
                    "coordinates": exterior
                });
                Ok(Literal::Geometry(result))
            }
            other => Err(Error::Validation(format!(
                "ST_BOUNDARY not supported for geometry type: {}",
                other
            ))),
        }
    }
}
