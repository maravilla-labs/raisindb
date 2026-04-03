//! ST_MAKEPOLYGON function - create a Polygon from a closed LineString

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::get_geometry_type;

/// Create a Polygon from a closed LineString (exterior ring)
///
/// # SQL Signature
/// `ST_MAKEPOLYGON(linestring) -> GEOMETRY`
pub struct StMakePolygonFunction;

impl SqlFunction for StMakePolygonFunction {
    fn name(&self) -> &str {
        "ST_MAKEPOLYGON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_MAKEPOLYGON(linestring) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_MAKEPOLYGON requires exactly 1 argument".to_string(),
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
                    "ST_MAKEPOLYGON requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;
        if geom_type != "LineString" {
            return Err(Error::Validation(format!(
                "ST_MAKEPOLYGON requires LineString, got {}",
                geom_type
            )));
        }

        let coords = geom
            .get("coordinates")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                Error::Validation("LineString missing coordinates".to_string())
            })?;

        if coords.len() < 4 {
            return Err(Error::Validation(
                "LineString must have at least 4 coordinates to form a valid polygon ring"
                    .to_string(),
            ));
        }

        // Validate that first and last coordinate are the same
        let first = &coords[0];
        let last = &coords[coords.len() - 1];
        if first != last {
            return Err(Error::Validation(
                "LineString must be closed (first and last coordinate must be the same)"
                    .to_string(),
            ));
        }

        let result = serde_json::json!({
            "type": "Polygon",
            "coordinates": [coords]
        });

        Ok(Literal::Geometry(result))
    }
}
