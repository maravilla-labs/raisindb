//! ST_POINTN - the Nth vertex of a linear geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Geometry;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_arg};
use super::line_access::sole_line;
use super::z_support::{expect_arity, numeric_arg};

const SIGNATURE: &str = "ST_POINTN(geometry, n) -> GEOMETRY";

/// The `n`th vertex of a linear geometry, **1-based**.
///
/// # SQL Signature
/// `ST_POINTN(geometry, n) -> GEOMETRY`
///
/// # Behaviour
/// * Indexing is 1-based, matching PostGIS and SQL convention: `ST_POINTN(line, 1)`
///   is [`ST_STARTPOINT`](super::StStartPointFunction).
/// * A **negative** index counts back from the end, so `-1` is the last vertex —
///   PostGIS 3.4 behaviour, and the reason `0` is the one index that never names a
///   vertex.
/// * Out of range gives `NULL`, not an error: paths in a column have different
///   lengths, and a query asking for the tenth vertex of every route should not
///   abort on the first short one.
/// * A one-component `MultiLineString` is accepted; anything else gives `NULL`.
/// * `NULL` in, `NULL` out. The CRS survives.
pub struct StPointNFunction;

impl SqlFunction for StPointNFunction {
    fn name(&self) -> &str {
        "ST_POINTN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_POINTN", SIGNATURE, args, 2)?;

        let Some(n) = numeric_arg("ST_POINTN", args, 1, row)? else {
            return Ok(Literal::Null);
        };
        let Some(g) = geom_arg("ST_POINTN", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        let Some(line) = sole_line(&g) else {
            return Ok(Literal::Null);
        };

        let len = line.0.len() as i64;
        let n = n.trunc() as i64;
        let index = if n > 0 {
            n - 1
        } else if n < 0 {
            len + n
        } else {
            return Ok(Literal::Null);
        };

        if index < 0 || index >= len {
            return Ok(Literal::Null);
        }
        derived_result(Geometry::Point(line.0[index as usize].into()), &g)
    }
}
