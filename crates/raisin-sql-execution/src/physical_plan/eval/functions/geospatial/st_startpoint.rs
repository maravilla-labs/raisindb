//! ST_STARTPOINT - first vertex of a linear geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::Geometry;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{derived_result, geom_arg};
use super::line_access::sole_line;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_STARTPOINT(geometry) -> GEOMETRY";

/// The first vertex of a linear geometry, as a Point.
///
/// # SQL Signature
/// `ST_STARTPOINT(geometry) -> GEOMETRY`
///
/// # Behaviour
/// * A `MultiLineString` with exactly one component is accepted, so the answer does
///   not depend on how a single path is spelled. Two or more components, or a
///   non-linear geometry, give `NULL` as in PostGIS.
/// * On a closed ring the start and end points coincide, so
///   `ST_STARTPOINT = ST_ENDPOINT` there — that is the definition of closed, not a
///   bug.
/// * `NULL` in, `NULL` out. The CRS survives.
pub struct StStartPointFunction;

impl SqlFunction for StStartPointFunction {
    fn name(&self) -> &str {
        "ST_STARTPOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_STARTPOINT", SIGNATURE, args, 1)?;
        let Some(g) = geom_arg("ST_STARTPOINT", args, 0, row)? else {
            return Ok(Literal::Null);
        };
        match sole_line(&g) {
            None => Ok(Literal::Null),
            Some(line) => derived_result(Geometry::Point(line.0[0].into()), &g),
        }
    }
}
