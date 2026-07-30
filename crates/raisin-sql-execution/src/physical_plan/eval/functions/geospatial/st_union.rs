//! ST_UNION - everything in either geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::{geom_pair, geom_result};
use super::setops::{self, SetOp};
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_UNION(geometry1, geometry2) -> GEOMETRY";

/// Every point that lies in either geometry.
///
/// # SQL Signature
/// `ST_UNION(geometry1, geometry2) -> GEOMETRY`
///
/// # Behaviour
/// * Defined for **every** pair of geometry types. The previous implementation
///   accepted only Polygon+Polygon and Point+Point and returned "not supported"
///   for the other seven combinations.
/// * The result is the narrowest type that represents it: one polygon is a
///   `Polygon`, two disjoint polygons a `MultiPolygon`, a mix of dimensions a
///   `GeometryCollection`. So `ST_AREA(ST_UNION(a, b))` works when the union
///   yields a MultiPolygon — the failure the brief names.
/// * Lower-dimensional parts covered by higher-dimensional ones are absorbed: the
///   union of a polygon and a line through it is just the polygon. See
///   [`super::setops`] for the full dimension table.
/// * Planar, in the operands' shared coordinate space. Two *different* explicit
///   SRIDs are an error naming `ST_TRANSFORM`.
/// * `NULL` in, `NULL` out. The empty geometry is the identity.
pub struct StUnionFunction;

impl SqlFunction for StUnionFunction {
    fn name(&self) -> &str {
        "ST_UNION"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_UNION", SIGNATURE, args, 2)?;
        match geom_pair("ST_UNION", args, row)? {
            None => Ok(Literal::Null),
            Some((a, b)) => geom_result(&setops::apply(SetOp::Union, &a, &b)?),
        }
    }
}
