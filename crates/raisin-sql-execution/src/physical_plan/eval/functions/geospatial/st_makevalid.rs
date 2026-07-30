//! ST_MAKEVALID - repair an invalid geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{geom_arg, geom_result};
use super::validate;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_MAKEVALID(geometry) -> GEOMETRY";

/// Repair a geometry so that [`ST_ISVALID`](super::StIsValidFunction) accepts it,
/// keeping as much of it as possible.
///
/// # SQL Signature
/// `ST_MAKEVALID(geometry) -> GEOMETRY`
///
/// # Behaviour
/// * **A valid geometry is returned unchanged**, byte for byte. This is a repair,
///   not a normalization, so it is safe to apply across a whole column.
/// * A self-intersecting bow-tie polygon becomes a valid `MultiPolygon` of its two
///   lobes, preserving the total area. Overlapping rings are merged.
/// * The CRS and the vertical extent survive.
/// * Puntal and linear components pass through: a non-finite ordinate is already
///   rejected at the parse boundary, and a self-crossing `LineString` is valid.
/// * `NULL` in, `NULL` out.
///
/// The mechanism is an overlay of the polygonal parts with themselves, which
/// recomputes the arrangement of their edges from scratch — see
/// [`super::validate::make_valid`].
///
/// # Examples
/// ```sql
/// UPDATE 'regions' SET boundary = ST_MAKEVALID(boundary) WHERE NOT ST_ISVALID(boundary);
/// ```
pub struct StMakeValidFunction;

impl SqlFunction for StMakeValidFunction {
    fn name(&self) -> &str {
        "ST_MAKEVALID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_MAKEVALID", SIGNATURE, args, 1)?;
        match geom_arg("ST_MAKEVALID", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => geom_result(&validate::make_valid(&g)),
        }
    }
}
