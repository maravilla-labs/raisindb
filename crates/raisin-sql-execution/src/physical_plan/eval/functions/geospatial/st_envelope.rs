//! ST_ENVELOPE function - return the bounding box of a geometry

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{extract_all_coords, get_geometry_type};

/// Return the bounding box (envelope) of a geometry as a Polygon
///
/// # SQL Signature
/// `ST_ENVELOPE(geometry) -> GEOMETRY`
pub struct StEnvelopeFunction;

impl SqlFunction for StEnvelopeFunction {
    fn name(&self) -> &str {
        "ST_ENVELOPE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_ENVELOPE(geometry) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 1 {
            return Err(Error::Validation(
                "ST_ENVELOPE requires exactly 1 argument".to_string(),
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
                    "ST_ENVELOPE requires GEOMETRY argument".to_string(),
                ))
            }
        };

        let geom_type = get_geometry_type(geom)?;

        // For Point, return the point itself
        if geom_type == "Point" {
            return Ok(Literal::Geometry(geom.clone()));
        }

        let coords = extract_all_coords(geom)?;
        if coords.is_empty() {
            return Ok(Literal::Null);
        }

        let mut min_lon = f64::MAX;
        let mut max_lon = f64::MIN;
        let mut min_lat = f64::MAX;
        let mut max_lat = f64::MIN;

        for c in &coords {
            min_lon = min_lon.min(c[0]);
            max_lon = max_lon.max(c[0]);
            min_lat = min_lat.min(c[1]);
            max_lat = max_lat.max(c[1]);
        }

        let result = serde_json::json!({
            "type": "Polygon",
            "coordinates": [[
                [min_lon, min_lat],
                [max_lon, min_lat],
                [max_lon, max_lat],
                [min_lon, max_lat],
                [min_lon, min_lat]
            ]]
        });

        Ok(Literal::Geometry(result))
    }
}
