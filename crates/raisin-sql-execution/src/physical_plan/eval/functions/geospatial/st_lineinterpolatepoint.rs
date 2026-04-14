//! ST_LINEINTERPOLATEPOINT function - return a point at a fraction along a LineString

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::helpers::{geojson_to_linestring, get_geometry_type, point_to_geojson};

/// Return a point at a given fraction (0.0-1.0) along a LineString
///
/// # SQL Signature
/// `ST_LINEINTERPOLATEPOINT(linestring, fraction) -> GEOMETRY`
pub struct StLineInterpolatePointFunction;

impl SqlFunction for StLineInterpolatePointFunction {
    fn name(&self) -> &str {
        "ST_LINEINTERPOLATEPOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_LINEINTERPOLATEPOINT(linestring, fraction) -> GEOMETRY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        if args.len() != 2 {
            return Err(Error::Validation(
                "ST_LINEINTERPOLATEPOINT requires exactly 2 arguments".to_string(),
            ));
        }

        let geom_val = eval_expr(&args[0], row)?;
        if matches!(geom_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let frac_val = eval_expr(&args[1], row)?;
        if matches!(frac_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        let geom = match &geom_val {
            Literal::Geometry(v) => v,
            Literal::JsonB(v) => v,
            _ => {
                return Err(Error::Validation(
                    "ST_LINEINTERPOLATEPOINT requires GEOMETRY as first argument".to_string(),
                ))
            }
        };

        let fraction = match &frac_val {
            Literal::Double(d) => *d,
            Literal::Int(i) => *i as f64,
            _ => {
                return Err(Error::Validation(
                    "ST_LINEINTERPOLATEPOINT requires numeric fraction".to_string(),
                ))
            }
        };

        if !(0.0..=1.0).contains(&fraction) {
            return Err(Error::Validation(
                "ST_LINEINTERPOLATEPOINT fraction must be between 0.0 and 1.0".to_string(),
            ));
        }

        let geom_type = get_geometry_type(geom)?;
        if geom_type != "LineString" {
            return Err(Error::Validation(format!(
                "ST_LINEINTERPOLATEPOINT requires LineString, got {}",
                geom_type
            )));
        }

        let line = geojson_to_linestring(geom)?;
        let coords: Vec<_> = line.coords().collect();

        if coords.is_empty() {
            return Ok(Literal::Null);
        }

        if coords.len() == 1 || fraction == 0.0 {
            return Ok(Literal::Geometry(point_to_geojson(
                coords[0].x,
                coords[0].y,
            )));
        }

        if fraction == 1.0 {
            let last = coords[coords.len() - 1];
            return Ok(Literal::Geometry(point_to_geojson(last.x, last.y)));
        }

        // Calculate total length of segments (Euclidean in coordinate space)
        let mut segment_lengths = Vec::with_capacity(coords.len() - 1);
        let mut total_length = 0.0;
        for i in 0..coords.len() - 1 {
            let dx = coords[i + 1].x - coords[i].x;
            let dy = coords[i + 1].y - coords[i].y;
            let len = (dx * dx + dy * dy).sqrt();
            segment_lengths.push(len);
            total_length += len;
        }

        if total_length == 0.0 {
            return Ok(Literal::Geometry(point_to_geojson(
                coords[0].x,
                coords[0].y,
            )));
        }

        let target_length = fraction * total_length;
        let mut accumulated = 0.0;

        for (i, seg_len) in segment_lengths.iter().enumerate() {
            if accumulated + seg_len >= target_length {
                let remaining = target_length - accumulated;
                let t = remaining / seg_len;
                let x = coords[i].x + t * (coords[i + 1].x - coords[i].x);
                let y = coords[i].y + t * (coords[i + 1].y - coords[i].y);
                return Ok(Literal::Geometry(point_to_geojson(x, y)));
            }
            accumulated += seg_len;
        }

        // Fallback to last point
        let last = coords[coords.len() - 1];
        Ok(Literal::Geometry(point_to_geojson(last.x, last.y)))
    }
}
