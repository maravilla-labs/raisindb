//! ST_ENVELOPE - the axis-aligned bounding box of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{BoundingRect, Geometry};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, empty_result, geom_arg};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_ENVELOPE(geometry) -> GEOMETRY";

/// The smallest axis-aligned rectangle containing the geometry.
///
/// # SQL Signature
/// `ST_ENVELOPE(geometry) -> GEOMETRY`
///
/// # Behaviour
/// * Works on every geometry type, including `Multi*` and `GeometryCollection`,
///   which the previous raw-JSON coordinate walker rejected.
/// * Degenerate cases collapse rather than producing a zero-area polygon: a Point
///   returns that Point, and a horizontal or vertical line returns a two-point
///   LineString. This matches PostGIS and keeps the result a faithful description
///   of the extent.
/// * An empty geometry has no extent, so the result is the empty geometry.
/// * Computed in the geometry's own coordinate space; the CRS survives.
/// * `NULL` in, `NULL` out.
///
/// # Relationship to the spatial index
/// This is the same envelope the planner uses for bbox pushdown: an envelope test
/// is a **superset** filter, so it can supply candidate rows but can never replace
/// the original predicate.
pub struct StEnvelopeFunction;

impl SqlFunction for StEnvelopeFunction {
    fn name(&self) -> &str {
        "ST_ENVELOPE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ENVELOPE", SIGNATURE, args, 1)?;
        let Some(g) = geom_arg("ST_ENVELOPE", args, 0, row)? else {
            return Ok(Literal::Null);
        };

        let Some(rect) = g.geometry.bounding_rect() else {
            return Ok(empty_result());
        };

        let (min, max) = (rect.min(), rect.max());
        let degenerate = min.x == max.x || min.y == max.y;
        let envelope = if min == max {
            Geometry::Point(min.into())
        } else if degenerate {
            Geometry::LineString(geo::LineString::from(vec![min, max]))
        } else {
            Geometry::Polygon(rect.to_polygon())
        };

        derived_result(envelope, &g)
    }
}
