//! ST_SYMDIFFERENCE - the parts belonging to exactly one of two geometries.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{geom_pair, geom_result};
use super::setops::{self, SetOp};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_SYMDIFFERENCE(geometry1, geometry2) -> GEOMETRY";

/// Every point in exactly one of the two geometries — the union minus the
/// intersection.
///
/// # SQL Signature
/// `ST_SYMDIFFERENCE(geometry1, geometry2) -> GEOMETRY`
///
/// # Behaviour
/// * Defined for **every** pair of geometry types; the previous implementation
///   supported Polygon+Polygon only.
/// * Commutative, unlike [`ST_DIFFERENCE`](super::StDifferenceFunction), and equal
///   to the union of the two one-sided differences. The identity
///   `area(sym) == area(union) - area(intersection)` is asserted in the set
///   operation tests.
/// * A geometry against itself gives the empty geometry; against the empty
///   geometry it gives itself back.
/// * Planar, in the operands' shared coordinate space. Two *different* explicit
///   SRIDs are an error naming `ST_TRANSFORM`.
/// * `NULL` in, `NULL` out.
pub struct StSymDifferenceFunction;

impl SqlFunction for StSymDifferenceFunction {
    fn name(&self) -> &str {
        "ST_SYMDIFFERENCE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_SYMDIFFERENCE", SIGNATURE, args, 2)?;
        match geom_pair("ST_SYMDIFFERENCE", args, row)? {
            None => Ok(Literal::Null),
            Some((a, b)) => geom_result(&setops::apply(SetOp::SymDifference, &a, &b)?),
        }
    }
}
