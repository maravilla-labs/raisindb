//! ST_DIFFERENCE - the part of the first geometry not in the second.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{geom_pair, geom_result};
use super::setops::{self, SetOp};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_DIFFERENCE(geometry1, geometry2) -> GEOMETRY";

/// Every point of `geometry1` that does not lie in `geometry2`.
///
/// # SQL Signature
/// `ST_DIFFERENCE(geometry1, geometry2) -> GEOMETRY`
///
/// # Behaviour
/// * Defined for **every** pair of geometry types; the previous implementation
///   supported Polygon+Polygon only.
/// * Only equal-or-higher-dimensional overlap removes anything, which is the part
///   people find surprising and is correct: a line *crossing* another line loses
///   nothing (the intersection is a point, which has no length), while a line
///   running *along* another loses that stretch. A polygon clips both lines and
///   points.
/// * Not commutative. `ST_DIFFERENCE(a, b)` and `ST_DIFFERENCE(b, a)` differ, and
///   their union is [`ST_SYMDIFFERENCE`](super::StSymDifferenceFunction).
/// * Subtracting a geometry from itself gives the empty geometry.
/// * Planar, in the operands' shared coordinate space. Two *different* explicit
///   SRIDs are an error naming `ST_TRANSFORM`.
/// * `NULL` in, `NULL` out.
pub struct StDifferenceFunction;

impl SqlFunction for StDifferenceFunction {
    fn name(&self) -> &str {
        "ST_DIFFERENCE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_DIFFERENCE", SIGNATURE, args, 2)?;
        match geom_pair("ST_DIFFERENCE", args, row)? {
            None => Ok(Literal::Null),
            Some((a, b)) => geom_result(&setops::apply(SetOp::Difference, &a, &b)?),
        }
    }
}
