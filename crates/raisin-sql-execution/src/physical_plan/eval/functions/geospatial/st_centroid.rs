//! ST_CENTROID - the geometric centre of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::{Centroid, Geometry};
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, empty_result, geom_arg};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_CENTROID(geometry) -> GEOMETRY";

/// The area-weighted geometric centre of a geometry, as a Point.
///
/// # SQL Signature
/// `ST_CENTROID(geometry) -> GEOMETRY`
///
/// # Behaviour
/// * Works on every geometry type. `Multi*` and `GeometryCollection` — previously
///   rejected — are weighted by the highest dimension present: an areal member
///   dominates a linear one, which dominates a puntal one, so the centroid of a
///   polygon plus a stray point is the polygon's centroid.
/// * An empty geometry has no centre, so the result is the empty geometry rather
///   than an error or an arbitrary origin.
/// * The centroid may lie **outside** the geometry (a crescent, a `MultiPolygon`
///   of two distant lobes). That is correct and is why the spatial index cannot
///   rely on centroid containment alone.
/// * Planar: computed in the geometry's own coordinate space, so on lon/lat it is
///   the centroid in degree space. The CRS survives.
/// * `NULL` in, `NULL` out.
pub struct StCentroidFunction;

impl SqlFunction for StCentroidFunction {
    fn name(&self) -> &str {
        "ST_CENTROID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_CENTROID", SIGNATURE, args, 1)?;
        let Some(g) = geom_arg("ST_CENTROID", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        match g.geometry.centroid() {
            Some(point) => derived_result(Geometry::Point(point), &g),
            None => Ok(empty_result()),
        }
    }
}
