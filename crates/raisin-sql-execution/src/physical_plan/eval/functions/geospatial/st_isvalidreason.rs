//! ST_ISVALIDREASON - why a geometry is invalid.

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

use super::convert::geom_arg;
use super::validate;
use super::z_support::expect_arity;

const SIGNATURE: &str = "ST_ISVALIDREASON(geometry) -> TEXT";

/// A human-readable explanation of why a geometry fails OGC validity, or
/// `"Valid Geometry"` when it does not.
///
/// # SQL Signature
/// `ST_ISVALIDREASON(geometry) -> TEXT`
///
/// # Behaviour
/// * Reports the **first** reason, like PostGIS. `geo` can enumerate all of them,
///   but one actionable sentence is what a user acts on.
/// * Typical messages: `exterior ring has a self-intersection`,
///   `interior ring at index 0 is not contained within the polygon's exterior`,
///   `exterior ring must have at least 3 distinct points`.
/// * Returns the literal string `Valid Geometry` for a valid geometry, matching
///   PostGIS so that existing diagnostic queries port unchanged.
/// * `NULL` in, `NULL` out.
///
/// # Examples
/// ```sql
/// -- Find and explain the broken rows before running ST_MAKEVALID over them.
/// SELECT path, ST_ISVALIDREASON(boundary) FROM 'regions'
///  WHERE NOT ST_ISVALID(boundary);
/// ```
pub struct StIsValidReasonFunction;

impl SqlFunction for StIsValidReasonFunction {
    fn name(&self) -> &str {
        "ST_ISVALIDREASON"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Geospatial
    }

    fn signature(&self) -> &str {
        SIGNATURE
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        expect_arity("ST_ISVALIDREASON", SIGNATURE, args, 1)?;
        match geom_arg("ST_ISVALIDREASON", args, 0, row)? {
            None => Ok(Literal::Null),
            Some(g) => Ok(Literal::Text(
                validate::invalid_reason(&g.geometry)
                    .unwrap_or_else(|| "Valid Geometry".to_string()),
            )),
        }
    }
}
