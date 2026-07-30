//! ST_INTERSECTS function - check if geometries intersect

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if two geometries have any point in common.
///
/// # SQL Signature
/// `ST_INTERSECTS(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if the geometries share at least one point (interior *or* boundary)
/// * FALSE if they are disjoint
/// * NULL if either input is NULL
///
/// # Examples
/// ```sql
/// -- Routes that enter a restricted zone
/// SELECT * FROM routes WHERE ST_INTERSECTS(route_line, boundary_polygon);
///
/// -- Two delivery zones that share any ground at all
/// SELECT ST_INTERSECTS(a.boundary, b.boundary) FROM zones a, zones b;
/// ```
///
/// # Notes
/// - The weakest of the topological predicates: TRUE if the geometries touch,
///   cross, overlap, or one covers the other.
/// - The exact complement of `ST_DISJOINT`: `geo` defines
///   `is_intersects` as `!is_disjoint`, so the two can no longer disagree. They
///   used to, because each hand-listed a different set of geometry-type pairs.
/// - Accepts **every** geometry type on both sides, including `Multi*` and
///   nested `GeometryCollection`s.
/// - DE-9IM `[T********] | [*T*******] | [***T*****] | [****T****]`.
pub struct StIntersectsFunction;

impl SqlFunction for StIntersectsFunction {
    fn name(&self) -> &str {
        "ST_INTERSECTS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_INTERSECTS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate(
            "ST_INTERSECTS",
            args,
            row,
            IntersectionMatrix::is_intersects,
        )
    }
}
