//! ST_SIMPLIFY - drop vertices that do not change the shape much.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{geom_arg, geom_result};
use super::metric_ops;
use super::z_support::{expect_arity, numeric_arg};

const SIGNATURE: &str = "ST_SIMPLIFY(geometry, tolerance) -> GEOMETRY";

/// Ramer-Douglas-Peucker simplification with `tolerance` in **metres** on a
/// geographic CRS, native units on a projected one.
///
/// # SQL Signature
/// `ST_SIMPLIFY(geometry, tolerance) -> GEOMETRY`
///
/// # Behaviour
/// * Works on every geometry type. Puntal components pass through unchanged, and
///   areal components keep their rings closed. `Multi*` and `GeometryCollection`
///   simplify member by member — previously they were rejected outright.
/// * A tolerance of 0 is a no-op; a negative or non-finite tolerance is an error.
/// * The CRS and the vertical extent survive.
/// * `NULL` in, `NULL` out.
///
/// # Units, and the trap
/// Like `ST_BUFFER`, `geo`'s `Simplify` is planar and works in the geometry's own
/// units, so on EPSG:4326 a raw tolerance would be **degrees**. A geographic
/// simplification is therefore projected into a metric CRS, simplified, and
/// projected back — which is why `ST_SIMPLIFY(track, 10)` means "flatten
/// deviations under ten metres" and not "under ten degrees".
///
/// # Caveat inherited from the algorithm
/// Douglas-Peucker is per-component and does not preserve topology: a large
/// tolerance can make a polygon self-intersect or make neighbouring polygons
/// overlap. Check the result with [`ST_ISVALID`](super::StIsValidFunction) when the
/// tolerance is a significant fraction of the feature size.
pub struct StSimplifyFunction;

impl SqlFunction for StSimplifyFunction {
    fn name(&self) -> &str {
        "ST_SIMPLIFY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_SIMPLIFY", SIGNATURE, args, 2)?;

        let Some(tolerance) = numeric_arg("ST_SIMPLIFY", args, 1, row)? else {
            return Ok(Literal::Null);
        };

        match geom_arg("ST_SIMPLIFY", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => geom_result(&metric_ops::simplify(&g, tolerance)?),
        }
    }
}
