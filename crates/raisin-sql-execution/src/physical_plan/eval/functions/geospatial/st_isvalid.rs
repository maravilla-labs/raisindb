//! ST_ISVALID - OGC validity of a geometry.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::validate;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_ISVALID(geometry) -> BOOLEAN";

/// True when a geometry satisfies the OGC Simple Feature validity rules.
///
/// # SQL Signature
/// `ST_ISVALID(geometry) -> BOOLEAN`
///
/// # What changed
/// The previous implementation inspected the JSON's *array shape* — that rings had
/// at least four entries and that the ordinates were numbers — so a
/// self-intersecting bow-tie polygon passed as valid. This now runs `geo`'s real
/// OGC validation, which catches ring self-intersection, rings crossing each
/// other, and holes that escape their shell.
///
/// # Behaviour
/// * Points and lines are valid by definition; a `LineString` is permitted to
///   cross itself (that is a question for
///   [`ST_ISSIMPLE`](super::StIsSimpleFunction), not for validity).
/// * `Multi*` and `GeometryCollection` are valid only if every member is.
/// * The empty geometry is valid.
/// * A value that is not parseable as a geometry is a query **error**, not
///   `false`: `ST_ISVALID` answers a question about geometries, and silently
///   reporting malformed JSON as "an invalid geometry" hides a data problem.
/// * `NULL` in, `NULL` out.
///
/// Use [`ST_ISVALIDREASON`](super::StIsValidReasonFunction) for the explanation
/// and [`ST_MAKEVALID`](super::StMakeValidFunction) for the repair.
///
/// # Known limitation
/// Inherited verbatim from `geo`: simple connectivity of a polygon's interior is
/// not checked, so rings that touch in a way that pinches the interior into two
/// parts are reported valid.
pub struct StIsValidFunction;

impl SqlFunction for StIsValidFunction {
    fn name(&self) -> &str {
        "ST_ISVALID"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ISVALID", SIGNATURE, args, 1)?;
        match geom_arg("ST_ISVALID", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Boolean(validate::is_valid(&g.geometry))),
        }
    }
}
