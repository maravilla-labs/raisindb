//! ST_WITHIN function - check if geometry A is within geometry B

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use geo::relate::IntersectionMatrix;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::relate;

/// Check if geometry A lies within geometry B.
///
/// # SQL Signature
/// `ST_WITHIN(geometry_a, geometry_b) -> BOOLEAN`
///
/// # Returns
/// * TRUE if no point of A lies outside B **and** at least one point of A's
///   interior lies in B's interior
/// * FALSE otherwise
/// * NULL if either input is NULL
///
/// # Examples
/// ```sql
/// -- Stores inside the city limits
/// SELECT * FROM stores WHERE ST_WITHIN(location, city_boundary);
///
/// -- Bus routes entirely inside one district
/// SELECT r.name FROM routes r, districts d WHERE ST_WITHIN(r.path, d.shape);
/// ```
///
/// # Notes
/// - `ST_WITHIN(a, b)` is `ST_CONTAINS(b, a)`, and the two are now literally the
///   same DE-9IM computation with the operands swapped.
/// - **The boundary is excluded**: a point exactly on the polygon boundary is
///   FALSE here and TRUE from `ST_COVEREDBY`. Same rule as PostGIS.
/// - Accepts every geometry type on both sides. The previous implementation only
///   handled `Point` within `Polygon` and `Polygon` within `Polygon` — its doc
///   comment claimed point-in-polygon only, and `LineString` within `Polygon`
///   raised an "unsupported" error.
/// - DE-9IM `[T*F**F***]`.
pub struct StWithinFunction;

impl SqlFunction for StWithinFunction {
    fn name(&self) -> &str {
        "ST_WITHIN"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        "ST_WITHIN(geometry_a, geometry_b) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        relate::predicate("ST_WITHIN", args, row, IntersectionMatrix::is_within)
    }
}
