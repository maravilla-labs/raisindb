//! ST_TOUCHES function - check if geometries touch but interiors don't intersect

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if two geometries touch — they meet, but their interiors do not.
///
/// # SQL Signature
/// `ST_TOUCHES(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if the geometries share at least one point and their interiors are
///   disjoint
/// * FALSE otherwise
/// * NULL if either input is NULL
///
/// # Examples
/// ```sql
/// -- Parcels that share a fence line but no ground
/// SELECT a.id, b.id FROM parcels a, parcels b
///  WHERE a.id < b.id AND ST_TOUCHES(a.shape, b.shape);
/// ```
///
/// # Notes
/// - Two `Point`s can never touch: a point has no boundary, so there is nothing
///   to meet at without the interiors meeting. That answer used to be produced by
///   a hardcoded `false` for *any* `Point` argument, which also — wrongly —
///   returned FALSE for a point sitting on the end of a line or on the edge of a
///   polygon. Those are now TRUE, correctly.
/// - Accepts every geometry type on both sides; only `Point`/`Point`,
///   `Point`/`Polygon` and `Polygon`/`Polygon` were handled before, and the
///   `Polygon`/`Polygon` arm approximated the test with a `1e-10` area threshold
///   on the boolean intersection.
/// - DE-9IM `[FT*******] | [F**T*****] | [F***T****]`.
pub struct StTouchesFunction;

impl SqlFunction for StTouchesFunction {
    fn name(&self) -> &str {
        "ST_TOUCHES"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_TOUCHES(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_TOUCHES", args, row, IntersectionMatrix::is_touches)
    }
}
