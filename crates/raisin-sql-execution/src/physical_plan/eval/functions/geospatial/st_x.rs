//! ST_X - longitude (or easting) of a point.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::walk::single_point;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_X(point) -> DOUBLE";

/// The X ordinate of a point: **longitude** on a geographic CRS, easting on a
/// projected one.
///
/// # SQL Signature
/// `ST_X(point) -> DOUBLE`
///
/// # Behaviour
/// * Defined for a single location only. Anything else — a line, a polygon, a
///   multi-point with two members — gives `NULL`, matching PostGIS, which returns
///   NULL rather than erroring so that a mixed-geometry column does not abort the
///   query on its first non-point row.
/// * A one-member `MultiPoint` is accepted: it is the same location as the Point,
///   and the answer must not depend on the spelling.
/// * `NULL` in, `NULL` out.
///
/// Axis order is `(longitude, latitude)` throughout RaisinDB, so `ST_X` is the
/// first ordinate and `ST_Y` the second.
pub struct StXFunction;

impl SqlFunction for StXFunction {
    fn name(&self) -> &str {
        "ST_X"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_X", SIGNATURE, args, 1)?;
        match geom_arg("ST_X", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(single_point(&g.geometry)
                .map(|p| Literal::Double(p.x()))
                .unwrap_or(Literal::Null)),
        }
    }
}
