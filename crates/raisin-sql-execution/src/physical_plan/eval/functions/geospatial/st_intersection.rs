//! ST_INTERSECTION - the shared part of two geometries.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{geom_pair, geom_result};
use super::setops::{self, SetOp};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_INTERSECTION(geometry1, geometry2) -> GEOMETRY";

/// Every point that lies in both geometries.
///
/// # SQL Signature
/// `ST_INTERSECTION(geometry1, geometry2) -> GEOMETRY`
///
/// # Behaviour
/// * Defined for **every** pair of geometry types; the previous implementation
///   supported Polygon+Polygon only.
/// * The result's dimension is that of the overlap, not of the inputs: two
///   polygons sharing area intersect in a polygon, two lines crossing intersect
///   in a **point**, two collinear lines in a line, and a line entering a polygon
///   in the clipped portion of that line.
/// * No overlap gives the canonical empty geometry rather than an error.
/// * Planar, in the operands' shared coordinate space. Two *different* explicit
///   SRIDs are an error naming `ST_TRANSFORM`.
/// * `NULL` in, `NULL` out.
pub struct StIntersectionFunction;

impl SqlFunction for StIntersectionFunction {
    fn name(&self) -> &str {
        "ST_INTERSECTION"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_INTERSECTION", SIGNATURE, args, 2)?;
        match geom_pair("ST_INTERSECTION", args, row)? {
            None => Ok(Literal::Null),
            Some((a, b)) => geom_result(&setops::apply(SetOp::Intersection, &a, &b)?),
        }
    }
}
