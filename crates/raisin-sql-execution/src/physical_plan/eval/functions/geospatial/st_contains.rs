//! ST_CONTAINS function - check if geometry A contains geometry B

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if geometry A contains geometry B.
///
/// # SQL Signature
/// `ST_CONTAINS(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if no point of B lies outside A **and** at least one point of B's
///   interior lies in A's interior
/// * FALSE otherwise
/// * NULL if either input is NULL
///
/// # Examples
/// ```sql
/// -- Orders whose drop-off is inside the delivery zone
/// SELECT * FROM orders WHERE ST_CONTAINS(delivery_zone, delivery_location);
///
/// -- Districts that fully contain a park
/// SELECT d.name FROM districts d, parks p WHERE ST_CONTAINS(d.shape, p.shape);
/// ```
///
/// # Notes
/// - Exactly the mirror of `ST_WITHIN`: `ST_CONTAINS(a, b)` and
///   `ST_WITHIN(b, a)` are the same DE-9IM test with the operands swapped, so the
///   two always agree.
/// - **A polygon does not contain its own boundary.** A point lying exactly on
///   the boundary is FALSE here and TRUE from `ST_COVERS`. That is the
///   DE-9IM definition and PostGIS behaves identically; prefer `ST_COVERS` unless
///   the interior-only test is what you mean.
/// - Accepts every geometry type on both sides in any combination. The previous
///   implementation supported only `Polygon` contains `Point` and `Polygon`
///   contains `Polygon`, and errored on everything else — including
///   `Polygon` contains `LineString`, which is one of the commonest uses.
/// - DE-9IM `[T*****FF*]`.
pub struct StContainsFunction;

impl SqlFunction for StContainsFunction {
    fn name(&self) -> &str {
        "ST_CONTAINS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_CONTAINS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_CONTAINS", args, row, IntersectionMatrix::is_contains)
    }
}
