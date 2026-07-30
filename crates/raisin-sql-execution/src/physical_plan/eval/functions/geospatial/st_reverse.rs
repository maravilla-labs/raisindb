//! ST_REVERSE - reverse the vertex order of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Geometry, LineString, MultiLineString, MultiPolygon, Polygon};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_arg, value_arg};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_REVERSE(geometry) -> GEOMETRY";

/// The same geometry with its vertex order reversed, and therefore its ring
/// winding flipped.
///
/// # SQL Signature
/// `ST_REVERSE(geometry) -> GEOMETRY`
///
/// # Behaviour
/// * The **type is preserved**: a LineString reverses to a LineString, a Polygon to
///   a Polygon with every ring reversed. `Multi*` and `GeometryCollection` reverse
///   each member and keep the member order — previously they were rejected.
/// * Points and MultiPoints come back **byte-identical**; a location has no
///   direction, and PostGIS behaves the same way.
/// * Rings stay closed: reversing a closed ring keeps its first and last vertex
///   coincident.
/// * Reversing twice is the identity.
/// * `NULL` in, `NULL` out. The CRS survives.
///
/// # Altitude
/// A third ordinate survives on a puntal geometry (that path does not touch `geo`)
/// but is dropped on a linear or areal one, because `geo`'s coordinates are
/// strictly 2-D. That is the same rule every geometry-returning ST_\* function
/// follows; use `ST_FORCE3D` to reattach a height.
///
/// # What this is for
/// Direction is meaningful for linear features (the travel direction of a route)
/// and winding is meaningful for rings: GeoJSON RFC 7946 asks for counter-clockwise
/// exteriors and clockwise holes, and this is how a ring is flipped to match.
pub struct StReverseFunction;

impl SqlFunction for StReverseFunction {
    fn name(&self) -> &str {
        "ST_REVERSE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_REVERSE", SIGNATURE, args, 1)?;
        let Some(value) = value_arg("ST_REVERSE", args, 0, row)? else {
            return Ok(Literal::Null);
        };

        // A puntal geometry has no order to reverse, so hand the ORIGINAL value
        // back rather than round-tripping it through `geo`. That keeps a genuine
        // no-op byte-exact and, because `geo`'s coordinates are strictly 2-D, keeps
        // any altitude the caller stored.
        if matches!(
            raisin_geometry::to_geo(&value, None)?.geometry,
            Geometry::Point(_) | Geometry::MultiPoint(_)
        ) {
            return Ok(Literal::Geometry(value));
        }

        let Some(g) = geom_arg("ST_REVERSE", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        derived_result(reverse(&g.geometry), &g)
    }
}

fn reverse(g: &Geometry<f64>) -> Geometry<f64> {
    match g {
        // A set of locations has no order that means anything.
        Geometry::Point(_) | Geometry::MultiPoint(_) => g.clone(),

        Geometry::Line(l) => Geometry::Line(geo::Line::new(l.end, l.start)),
        Geometry::LineString(ls) => Geometry::LineString(reverse_ring(ls)),
        Geometry::MultiLineString(mls) => {
            Geometry::MultiLineString(MultiLineString(mls.0.iter().map(reverse_ring).collect()))
        }
        Geometry::Polygon(p) => Geometry::Polygon(reverse_polygon(p)),
        Geometry::MultiPolygon(mp) => {
            Geometry::MultiPolygon(MultiPolygon(mp.0.iter().map(reverse_polygon).collect()))
        }

        // A rectangle is defined by two corners, so reversal is a no-op; a
        // triangle's vertex order is its winding.
        Geometry::Rect(_) => g.clone(),
        Geometry::Triangle(t) => Geometry::Triangle(geo::Triangle::new(t.v3(), t.v2(), t.v1())),

        Geometry::GeometryCollection(gc) => {
            Geometry::GeometryCollection(gc.0.iter().map(reverse).collect::<Vec<_>>().into())
        }
    }
}

fn reverse_ring(ls: &LineString<f64>) -> LineString<f64> {
    LineString::new(ls.0.iter().rev().copied().collect())
}

fn reverse_polygon(p: &Polygon<f64>) -> Polygon<f64> {
    Polygon::new(
        reverse_ring(p.exterior()),
        p.interiors().iter().map(reverse_ring).collect(),
    )
}
