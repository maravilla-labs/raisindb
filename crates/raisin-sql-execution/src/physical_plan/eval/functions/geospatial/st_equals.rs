//! ST_EQUALS function - check if geometries are topologically equal

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if two geometries are *topologically* equal — they occupy the same point
/// set.
///
/// # SQL Signature
/// `ST_EQUALS(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if each geometry covers the other
/// * FALSE otherwise
/// * NULL if either input is NULL
///
/// # Notes
/// - **Topological, not structural.** `LINESTRING(0 0, 1 1, 2 2)` equals
///   `LINESTRING(0 0, 2 2)`, and a polygon equals the same polygon with its
///   vertices rotated or its winding reversed. The type names need not match
///   either: a one-member `MultiPoint` equals the corresponding `Point`. The old
///   implementation compared vertex arrays pairwise after an early
///   `type_a != type_b → false`, so it answered FALSE to all of those.
/// - **Exact, with no coordinate tolerance.** Two coordinates that differ at all
///   are different points. The old implementation applied a `1e-8` degree
///   (≈1.1 mm) epsilon, which reported distinct locations as equal — the same
///   class of silent wrongness this work removes, and it also made `ST_EQUALS`
///   inconsistent with `ST_COVERS`/`ST_COVEREDBY`, which have no epsilon. Use
///   `ST_DWITHIN(a, b, tolerance)` when a tolerance is what you want.
/// - Any two empty geometries are equal, which is the JTS/PostGIS answer.
/// - Accepts every geometry type on both sides.
/// - DE-9IM `[T*F**FFF*]`.
pub struct StEqualsFunction;

impl SqlFunction for StEqualsFunction {
    fn name(&self) -> &str {
        "ST_EQUALS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_EQUALS(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_EQUALS", args, row, IntersectionMatrix::is_equal_topo)
    }
}
