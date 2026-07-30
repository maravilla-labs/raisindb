//! ST_DISJOINT function - check if geometries do not intersect

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if two geometries are disjoint — they share no point at all.
///
/// # SQL Signature
/// `ST_DISJOINT(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if the geometries have no point in common
/// * FALSE if they touch, cross, overlap or one covers the other
/// * NULL if either input is NULL
///
/// # Notes
/// - The exact complement of `ST_INTERSECTS`. That is now structural
///   rather than aspirational: both read the same DE-9IM matrix and `geo` defines
///   `is_intersects` as `!is_disjoint`. Previously the two functions listed
///   different sets of geometry-type pairs — `ST_DISJOINT` supported nine and
///   `ST_INTERSECTS` six — so `NOT ST_INTERSECTS(a, b)` and `ST_DISJOINT(a, b)`
///   could give different answers for the same rows.
/// - Two empty geometries are disjoint (TRUE), which is the JTS/PostGIS answer.
/// - DE-9IM `[FF*FF****]`.
pub struct StDisjointFunction;

impl SqlFunction for StDisjointFunction {
    fn name(&self) -> &str {
        "ST_DISJOINT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_DISJOINT(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_DISJOINT", args, row, IntersectionMatrix::is_disjoint)
    }
}
