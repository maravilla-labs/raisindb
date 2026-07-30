//! ST_PERIMETER - boundary length of the areal components of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::measure;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_PERIMETER(geometry) -> DOUBLE";

/// Boundary length of a geometry's 2-dimensional components: **metres** on a
/// geographic CRS, native units on a projected one.
///
/// # SQL Signature
/// `ST_PERIMETER(geometry) -> DOUBLE`
///
/// # Behaviour
/// * Every ring counts, **interior rings included** — a polygon with a hole has a
///   longer perimeter than the same polygon without one. The previous
///   implementation measured only the exterior ring.
/// * `MultiPolygon` and `GeometryCollection` sum their areal members.
/// * Puntal and linear components contribute 0, matching PostGIS.
/// * `NULL` in, `NULL` out.
pub struct StPerimeterFunction;

impl SqlFunction for StPerimeterFunction {
    fn name(&self) -> &str {
        "ST_PERIMETER"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_PERIMETER", SIGNATURE, args, 1)?;
        match geom_arg("ST_PERIMETER", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Double(measure::perimeter(&g))),
        }
    }
}
