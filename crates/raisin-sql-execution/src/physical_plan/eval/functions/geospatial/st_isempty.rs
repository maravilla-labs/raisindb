//! ST_ISEMPTY - whether a geometry holds any coordinates.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_ISEMPTY(geometry) -> BOOLEAN";

/// True when a geometry contains no coordinates.
///
/// # SQL Signature
/// `ST_ISEMPTY(geometry) -> BOOLEAN`
///
/// # Behaviour
/// * Emptiness is judged after parsing and **recursively**: a GeometryCollection
///   whose only member is an empty MultiPolygon is empty. The previous
///   implementation looked one level down at the JSON's `coordinates` array, so it
///   reported such a collection as non-empty.
/// * Distinct from `NULL`. `NULL` means "no value"; empty means "a geometry with no
///   extent" — the canonical
///   `{"type":"GeometryCollection","geometries":[]}` that set operations return
///   when nothing is left. `NULL` in, `NULL` out.
/// * Empty geometries propagate through every ST_\* function rather than raising
///   errors, which is what makes chained set operations safe.
pub struct StIsEmptyFunction;

impl SqlFunction for StIsEmptyFunction {
    fn name(&self) -> &str {
        "ST_ISEMPTY"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ISEMPTY", SIGNATURE, args, 1)?;
        match geom_arg("ST_ISEMPTY", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Boolean(g.is_empty())),
        }
    }
}
