//! ST_NUMPOINTS - vertex count of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::CoordsIter;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_NUMPOINTS(geometry) -> INTEGER";

/// The number of vertices in a geometry.
///
/// # SQL Signature
/// `ST_NUMPOINTS(geometry) -> INTEGER`
///
/// # Behaviour
/// * Counts every vertex of every component, so `Multi*` and `GeometryCollection`
///   are supported — previously they were an error, as was `MultiLineString`.
/// * A polygon counts all of its rings' vertices, including the repeated closing
///   vertex of each ring, which is how PostGIS counts them.
/// * The empty geometry has 0 vertices.
/// * `NULL` in, `NULL` out.
///
/// # Divergence from PostGIS
/// PostGIS's `ST_NumPoints` is defined for LineStrings only and returns NULL for
/// anything else; its general form is `ST_NPoints`. Answering for every type is
/// strictly more useful and cannot mislead, so RaisinDB does not carry the
/// restriction.
pub struct StNumPointsFunction;

impl SqlFunction for StNumPointsFunction {
    fn name(&self) -> &str {
        "ST_NUMPOINTS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_NUMPOINTS", SIGNATURE, args, 1)?;
        match geom_arg("ST_NUMPOINTS", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Int(
                g.geometry.coords_count().try_into().unwrap_or(i32::MAX),
            )),
        }
    }
}
