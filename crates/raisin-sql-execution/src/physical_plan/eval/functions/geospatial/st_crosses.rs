//! ST_CROSSES function - check if geometries cross each other

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if two geometries spatially cross — they have some, but not all,
/// interior points in common, and the shared part is of lower dimension than the
/// larger input.
///
/// # SQL Signature
/// `ST_CROSSES(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if the interiors intersect in a set of dimension strictly lower than
///   the maximum dimension of the two inputs, and neither geometry is contained
///   in the other
/// * FALSE otherwise
/// * NULL if either input is NULL
///
/// # Examples
/// ```sql
/// -- Roads that cut across a flood zone rather than staying inside it
/// SELECT * FROM roads WHERE ST_CROSSES(path, flood_zone);
///
/// -- Two rivers meeting at a confluence
/// SELECT ST_CROSSES(a.course, b.course) FROM rivers a, rivers b;
/// ```
///
/// # Notes
/// - Two polygons can never cross (their intersection is 2-D, the same dimension
///   as the inputs) and two points can never cross. A `MultiPoint` *can* cross a
///   line or a polygon, when some of its points are inside and some outside —
///   the old implementation returned a hardcoded FALSE for any `Point`-family
///   argument and so got that case wrong.
/// - Accepts every geometry type on both sides; only `LineString`/`Polygon` and
///   `LineString`/`LineString` were really handled before.
/// - DE-9IM `[T*T******]` when `dim(a) < dim(b)`, `[T*****T**]` when
///   `dim(a) > dim(b)`, `[0********]` for line/line.
pub struct StCrossesFunction;

impl SqlFunction for StCrossesFunction {
    fn name(&self) -> &str {
        "ST_CROSSES"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_CROSSES(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_CROSSES", args, row, IntersectionMatrix::is_crosses)
    }
}
