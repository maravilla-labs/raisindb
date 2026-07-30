//! ST_LENGTH - length of the linear components of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::measure;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_LENGTH(geometry) -> DOUBLE";

/// Total length of a geometry's 1-dimensional components: **metres** on a
/// geographic CRS, native units on a projected one.
///
/// # SQL Signature
/// `ST_LENGTH(geometry) -> DOUBLE`
///
/// # Behaviour
/// * `LineString` and `MultiLineString` sum their segments; a
///   `GeometryCollection` sums its linear members.
/// * Puntal components contribute 0.
/// * **Areal components contribute 0.** A polygon's boundary is measured by
///   [`ST_PERIMETER`](super::StPerimeterFunction), which is why both functions
///   exist. This matches PostGIS and changes the previous behaviour, which
///   returned a polygon's exterior-ring length here and so made `ST_LENGTH` and
///   `ST_PERIMETER` indistinguishable.
/// * `NULL` in, `NULL` out.
///
/// # Divergence from PostGIS
/// On EPSG:4326 the result is Haversine metres, not degrees — the same choice
/// `ST_AREA` makes, for the same reason.
pub struct StLengthFunction;

impl SqlFunction for StLengthFunction {
    fn name(&self) -> &str {
        "ST_LENGTH"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_LENGTH", SIGNATURE, args, 1)?;
        match geom_arg("ST_LENGTH", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Double(measure::length(&g))),
        }
    }
}
