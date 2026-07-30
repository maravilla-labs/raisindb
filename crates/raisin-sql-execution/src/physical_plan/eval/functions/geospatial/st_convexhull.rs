//! ST_CONVEXHULL - the smallest convex shape enclosing a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{ConvexHull, Coord, Geometry, LineString};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, empty_result, geom_arg};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_CONVEXHULL(geometry) -> GEOMETRY";

/// The smallest convex geometry containing all of the input's vertices — the shape
/// a rubber band would take around it.
///
/// # SQL Signature
/// `ST_CONVEXHULL(geometry) -> GEOMETRY`
///
/// # Behaviour
/// * Works on every geometry type. `MultiPoint`, `MultiPolygon` and
///   `GeometryCollection` — the useful cases, and the ones the previous
///   implementation rejected — are hulled over all their vertices at once, so the
///   hull of two distant polygons spans both.
/// * The result collapses to the lowest faithful dimension, as in PostGIS: one
///   distinct vertex gives a Point, two or more collinear vertices give a
///   LineString, and anything with area gives a Polygon.
/// * An empty geometry gives the empty geometry.
/// * Planar, in the geometry's own coordinate space; the CRS survives.
/// * `NULL` in, `NULL` out.
///
/// # Examples
/// ```sql
/// -- The territory covered by a delivery run.
/// SELECT ST_CONVEXHULL(ST_COLLECT(a.location, b.location)) FROM stops a, stops b;
/// ```
pub struct StConvexHullFunction;

impl SqlFunction for StConvexHullFunction {
    fn name(&self) -> &str {
        "ST_CONVEXHULL"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_CONVEXHULL", SIGNATURE, args, 1)?;
        let Some(g) = geom_arg("ST_CONVEXHULL", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        if g.is_empty() {
            return Ok(empty_result());
        }

        // `ConvexHull` has a blanket impl over `CoordsIter`, so it already covers
        // every geometry type; it always hands back a `Polygon`, which has to be
        // narrowed for degenerate input.
        let hull = g.geometry.convex_hull();
        derived_result(narrow(hull.exterior()), &g)
    }
}

/// Collapse a hull ring whose vertices do not enclose any area.
fn narrow(ring: &LineString<f64>) -> Geometry<f64> {
    let mut distinct: Vec<Coord<f64>> = Vec::with_capacity(ring.0.len());
    for c in &ring.0 {
        if !distinct.contains(c) {
            distinct.push(*c);
        }
    }
    match distinct.len() {
        0 => Geometry::GeometryCollection(Default::default()),
        1 => Geometry::Point(distinct[0].into()),
        2 => Geometry::LineString(LineString::new(distinct)),
        _ => Geometry::Polygon(geo::Polygon::new(ring.clone(), vec![])),
    }
}
