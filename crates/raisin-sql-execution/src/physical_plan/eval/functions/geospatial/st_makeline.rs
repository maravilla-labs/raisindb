//! ST_MAKELINE - join geometries into a LineString.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Coord, Geometry, LineString};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_pair};
use super::walk::{for_each_line_string, for_each_point};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_MAKELINE(geometry1, geometry2) -> GEOMETRY";

/// A LineString through the vertices of both arguments, in order.
///
/// # SQL Signature
/// `ST_MAKELINE(geometry1, geometry2) -> GEOMETRY`
///
/// # Behaviour
/// * Accepts points **and** linear geometries, so two route fragments can be
///   spliced and a `MultiPoint` can become a path. The previous implementation
///   required two Points exactly.
/// * Vertices are taken in argument order, then component order, then vertex
///   order: `ST_MAKELINE(a, b)` and `ST_MAKELINE(b, a)` differ, as they must for a
///   directed feature.
/// * Consecutive duplicate vertices are collapsed, so splicing two fragments that
///   share an endpoint does not leave a zero-length segment behind.
/// * Areal arguments contribute nothing — a polygon is not a path. If neither
///   argument yields at least two distinct vertices the result is the empty
///   geometry rather than an invalid one-point LineString.
/// * `NULL` in, `NULL` out.
pub struct StMakeLineFunction;

impl SqlFunction for StMakeLineFunction {
    fn name(&self) -> &str {
        "ST_MAKELINE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_MAKELINE", SIGNATURE, args, 2)?;
        let Some((a, b)) = geom_pair("ST_MAKELINE", args, row)? else {
            return Ok(Literal::Null);
        };

        let mut coords: Vec<Coord<f64>> = Vec::new();
        push_vertices(&a.geometry, &mut coords);
        push_vertices(&b.geometry, &mut coords);
        coords.dedup();

        let line = if coords.len() >= 2 {
            Geometry::LineString(LineString::new(coords))
        } else {
            Geometry::GeometryCollection(Default::default())
        };
        derived_result(line, &a)
    }
}

/// Append a geometry's path vertices: its points, then its linear components.
fn push_vertices(g: &Geometry<f64>, out: &mut Vec<Coord<f64>>) {
    for_each_point(g, &mut |p| out.push(p.into()));
    for_each_line_string(g, &mut |ls| out.extend(ls.0.iter().copied()));
}
