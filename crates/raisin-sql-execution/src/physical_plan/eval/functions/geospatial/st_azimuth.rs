//! ST_AZIMUTH - bearing from one point to another.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_pair;
use super::measure;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_AZIMUTH(point1, point2) -> DOUBLE";

/// Bearing from `point1` to `point2` in **radians**, north-clockwise, normalized
/// to `[0, 2pi)`.
///
/// # SQL Signature
/// `ST_AZIMUTH(point1, point2) -> DOUBLE`
///
/// # Behaviour
/// * Geodesic on a geographic CRS, planar on a projected one.
/// * `NULL` — not an error — when either argument is not a single location, or
///   when the two coincide. The azimuth from a point to itself is undefined, and
///   PostGIS likewise returns NULL rather than an arbitrary 0.
/// * A one-member `MultiPoint` is accepted as a location: the answer must not
///   depend on how the same place is spelled.
///
/// # Examples
/// ```sql
/// SELECT DEGREES(ST_AZIMUTH(ST_POINT(8.54, 47.37), ST_POINT(8.54, 48.37)));
/// -- 0, due north
/// ```
pub struct StAzimuthFunction;

impl SqlFunction for StAzimuthFunction {
    fn name(&self) -> &str {
        "ST_AZIMUTH"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_AZIMUTH", SIGNATURE, args, 2)?;
        match geom_pair("ST_AZIMUTH", args, row)? {
            None => Ok(Literal::Null),
            Some((a, b)) => Ok(measure::bearing_radians(&a, &b)
                .map(Literal::Double)
                .unwrap_or(Literal::Null)),
        }
    }
}
