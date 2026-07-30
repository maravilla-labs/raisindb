//! ST_COVERS function - check if geometry A covers geometry B

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if geometry A covers geometry B — no point of B lies outside A.
///
/// # SQL Signature
/// `ST_COVERS(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if every point of B is in the interior *or* the boundary of A
/// * FALSE otherwise
/// * NULL if either input is NULL
///
/// # Notes
/// - The boundary-inclusive sibling of `ST_CONTAINS`, and the one you
///   almost always want. `ST_CONTAINS` is FALSE for a point sitting exactly on a
///   polygon's edge; `ST_COVERS` is TRUE. So `ST_CONTAINS(a, b)` implies
///   `ST_COVERS(a, b)` but never the reverse.
/// - Unlike `ST_CONTAINS`, this predicate is well behaved on a geometry compared
///   with its own boundary — `ST_COVERS(polygon, ST_BOUNDARY(polygon))` is TRUE.
/// - `ST_COVERS(a, b)` is `ST_COVEREDBY(b, a)`; both read the same matrix.
/// - Accepts every geometry type on both sides. The old implementation supported
///   `Polygon` covers `Point` and `Polygon` covers `Polygon` only, and its
///   `Polygon`/`Polygon` arm used interior-only `contains`, so it disagreed with
///   its own `Polygon`/`Point` arm about whether the boundary counts.
/// - DE-9IM `[T*****FF*] | [*T****FF*] | [***T**FF*] | [****T*FF*]`.
pub struct StCoversFunction;

impl SqlFunction for StCoversFunction {
    fn name(&self) -> &str {
        "ST_COVERS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_COVERS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_COVERS", args, row, IntersectionMatrix::is_covers)
    }
}
