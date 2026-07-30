//! ST_MAKEPOLYGON - turn a closed ring into a Polygon.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Geometry, LineString, MultiPolygon, Polygon};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_arg};
use super::walk::for_each_line_string;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_MAKEPOLYGON(linestring) -> GEOMETRY";

/// Build a Polygon from a closed LineString.
///
/// # SQL Signature
/// `ST_MAKEPOLYGON(linestring) -> GEOMETRY`
///
/// # Behaviour
/// * A `MultiLineString` of closed rings becomes a `MultiPolygon` — one polygon per
///   ring — which the previous LineString-only implementation rejected.
/// * The ring must be **closed** (first vertex equal to last) and have at least
///   four vertices. Anything else is a validation error naming which rule failed,
///   because silently closing a ring for the caller invents geometry they did not
///   supply.
/// * Ring winding is preserved; use [`ST_REVERSE`](super::StReverseFunction) to
///   flip it. The result is not guaranteed valid — a self-intersecting ring makes a
///   self-intersecting polygon — so pair it with
///   [`ST_ISVALID`](super::StIsValidFunction) or
///   [`ST_MAKEVALID`](super::StMakeValidFunction) for untrusted input.
/// * `NULL` in, `NULL` out.
pub struct StMakePolygonFunction;

impl SqlFunction for StMakePolygonFunction {
    fn name(&self) -> &str {
        "ST_MAKEPOLYGON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_MAKEPOLYGON", SIGNATURE, args, 1)?;
        let Some(g) = geom_arg("ST_MAKEPOLYGON", args, 0, row)? else {
            return Ok(Literal::Null);
        };

        let mut rings: Vec<LineString<f64>> = Vec::new();
        for_each_line_string(&g.geometry, &mut |ls| rings.push(ls.clone()));

        if rings.is_empty() {
            return Err(Error::Validation(
                "ST_MAKEPOLYGON requires a LineString or MultiLineString; a polygon or a point \
                 has no ring to close"
                    .to_string(),
            ));
        }

        for ring in &rings {
            if ring.0.len() < 4 {
                return Err(Error::Validation(format!(
                    "ST_MAKEPOLYGON: a ring needs at least 4 vertices to enclose an area, got {}",
                    ring.0.len()
                )));
            }
            if !ring.is_closed() {
                return Err(Error::Validation(
                    "ST_MAKEPOLYGON: the ring must be closed — its first and last vertex must be \
                     identical"
                        .to_string(),
                ));
            }
        }

        let polygon = if rings.len() == 1 {
            Geometry::Polygon(Polygon::new(rings.remove(0), vec![]))
        } else {
            Geometry::MultiPolygon(MultiPolygon(
                rings.into_iter().map(|r| Polygon::new(r, vec![])).collect(),
            ))
        };
        derived_result(polygon, &g)
    }
}
