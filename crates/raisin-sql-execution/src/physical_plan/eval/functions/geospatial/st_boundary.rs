//! ST_BOUNDARY - the topological boundary of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Coord, Geometry, LineString, MultiLineString, MultiPoint};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_arg};
use super::walk::{for_each_line_string, for_each_polygon};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_BOUNDARY(geometry) -> GEOMETRY";

/// The topological boundary of a geometry: one dimension lower than the input.
///
/// # SQL Signature
/// `ST_BOUNDARY(geometry) -> GEOMETRY`
///
/// # Behaviour by type
/// * **Point / MultiPoint** — empty. A 0-dimensional geometry has no boundary.
/// * **LineString** — a MultiPoint of its two endpoints, or **empty** when the line
///   is closed. The previous implementation returned the coincident endpoints of a
///   closed ring, which is wrong: a ring has no boundary.
/// * **MultiLineString** — the endpoints that appear an **odd** number of times
///   across the components. This is the OGC mod-2 rule, and it is why two lines
///   joined end to end have the boundary of the single line they form rather than
///   four points.
/// * **Polygon / MultiPolygon** — the rings, interior rings included. A single-ring
///   polygon gives a LineString, anything else a MultiLineString. The previous
///   implementation returned only the exterior ring.
/// * **GeometryCollection** — the boundaries of its members, collected. PostGIS
///   errors here; propagating is more useful and cannot be wrong.
/// * `NULL` in, `NULL` out. The CRS survives.
pub struct StBoundaryFunction;

impl SqlFunction for StBoundaryFunction {
    fn name(&self) -> &str {
        "ST_BOUNDARY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_BOUNDARY", SIGNATURE, args, 1)?;
        let Some(g) = geom_arg("ST_BOUNDARY", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        derived_result(boundary(&g.geometry), &g)
    }
}

/// The boundary of any geometry.
///
/// Areal components dominate: when a geometry has both, the areal boundary (1-D)
/// and the linear boundary (0-D) are reported together in a collection, which is
/// the only faithful answer for mixed input.
fn boundary(g: &Geometry<f64>) -> Geometry<f64> {
    let mut rings: Vec<LineString<f64>> = Vec::new();
    for_each_polygon(g, &mut |p| {
        rings.push(p.exterior().clone());
        rings.extend(p.interiors().iter().cloned());
    });

    let mut endpoints: Vec<Coord<f64>> = Vec::new();
    for_each_line_string(g, &mut |ls| {
        if !ls.is_closed() && ls.0.len() >= 2 {
            endpoints.push(ls.0[0]);
            endpoints.push(ls.0[ls.0.len() - 1]);
        }
    });
    let odd = odd_occurrences(endpoints);

    let mut members: Vec<Geometry<f64>> = Vec::with_capacity(2);
    match rings.len() {
        0 => {}
        1 => members.push(Geometry::LineString(rings.remove(0))),
        _ => members.push(Geometry::MultiLineString(MultiLineString(rings))),
    }
    if !odd.is_empty() {
        members.push(Geometry::MultiPoint(MultiPoint(
            odd.into_iter().map(Into::into).collect(),
        )));
    }

    match members.len() {
        0 => Geometry::GeometryCollection(Default::default()),
        1 => members.remove(0),
        _ => Geometry::GeometryCollection(members.into()),
    }
}

/// Keep the coordinates occurring an odd number of times — the OGC mod-2 rule for
/// the boundary of a 1-dimensional geometry.
fn odd_occurrences(coords: Vec<Coord<f64>>) -> Vec<Coord<f64>> {
    let mut tally: Vec<(Coord<f64>, usize)> = Vec::with_capacity(coords.len());
    for c in coords {
        match tally.iter_mut().find(|(seen, _)| *seen == c) {
            Some((_, count)) => *count += 1,
            None => tally.push((c, 1)),
        }
    }
    tally
        .into_iter()
        .filter(|(_, count)| count % 2 == 1)
        .map(|(c, _)| c)
        .collect()
}
