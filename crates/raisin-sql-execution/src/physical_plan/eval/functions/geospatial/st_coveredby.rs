//! ST_COVEREDBY function - check if geometry A is covered by geometry B

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if geometry A is covered by geometry B — no point of A lies outside B.
///
/// # SQL Signature
/// `ST_COVEREDBY(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if every point of A is in the interior *or* the boundary of B
/// * FALSE otherwise
/// * NULL if either input is NULL
///
/// # Notes
/// - The boundary-inclusive sibling of `ST_WITHIN`. A point exactly on
///   the polygon boundary is FALSE for `ST_WITHIN` and TRUE here, so
///   `ST_WITHIN(a, b)` implies `ST_COVEREDBY(a, b)` but not the reverse.
/// - `ST_COVEREDBY(a, b)` is `ST_COVERS(b, a)`.
/// - Accepts every geometry type on both sides; the previous implementation
///   handled only `Point` covered by `Polygon` and `Polygon` covered by `Polygon`.
/// - DE-9IM `[T*F**F***] | [*TF**F***] | [**FT*F***] | [**F*TF***]`.
pub struct StCoveredByFunction;

impl SqlFunction for StCoveredByFunction {
    fn name(&self) -> &str {
        "ST_COVEREDBY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_COVEREDBY(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_COVEREDBY", args, row, IntersectionMatrix::is_coveredby)
    }
}
