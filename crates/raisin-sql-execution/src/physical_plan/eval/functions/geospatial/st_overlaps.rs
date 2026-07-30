//! ST_OVERLAPS function - check if geometries overlap

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if two geometries of the same dimension spatially overlap.
///
/// # SQL Signature
/// `ST_OVERLAPS(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if the geometries have the same dimension, their interiors intersect in
///   that dimension, and each has at least one point outside the other
/// * FALSE otherwise — including for mixed dimensions, where overlap is undefined
/// * NULL if either input is NULL
///
/// # Examples
/// ```sql
/// -- Two sales territories that partially share ground
/// SELECT a.id, b.id FROM territories a, territories b
///  WHERE a.id < b.id AND ST_OVERLAPS(a.area, b.area);
/// ```
///
/// # Notes
/// - "Same dimension" is measured from the DE-9IM matrix, not from the GeoJSON
///   type name, so `Polygon`/`MultiPolygon` and `LineString`/`MultiLineString`
///   pairs are handled as the 2-D and 1-D cases they are.
/// - Nesting is excluded by definition: if one geometry covers the other they do
///   not overlap.
/// - Previously this function returned a **silent catch-all FALSE** for every
///   type pair it did not name explicitly — three pairs out of the possible
///   forty-nine — so, for instance, two overlapping `MultiPolygon`s reported
///   FALSE with no error and no signal. That was the single most dangerous defect
///   in this family, because a wrong boolean is indistinguishable from a right one.
/// - DE-9IM `[1*T***T**]` for line/line, `[T*T***T**]` for point/point and
///   area/area.
pub struct StOverlapsFunction;

impl SqlFunction for StOverlapsFunction {
    fn name(&self) -> &str {
        "ST_OVERLAPS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_OVERLAPS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_OVERLAPS", args, row, IntersectionMatrix::is_overlaps)
    }
}
