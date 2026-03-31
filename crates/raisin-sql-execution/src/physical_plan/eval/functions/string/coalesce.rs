//! COALESCE function - return first non-NULL value

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Return the first non-NULL argument
///
/// # SQL Signature
/// `COALESCE(value1, value2, ...) -> ANY`
///
/// # Arguments
/// * `value1, value2, ...` - Values to check (at least 1 required)
///
/// # Returns
/// * The first non-NULL argument
/// * NULL if all arguments are NULL
///
/// # Examples
/// ```sql
/// SELECT COALESCE(NULL, 'hello', 'world') -> 'hello'
/// SELECT COALESCE(NULL, NULL, 42) -> 42
/// SELECT COALESCE(NULL, NULL) -> NULL
/// SELECT COALESCE('first', 'second') -> 'first'
/// ```
///
/// # Notes
/// This is a standard SQL function used for NULL handling and default values.
/// Arguments can be of different types - the return type is determined by the
/// first non-NULL value encountered.
pub struct CoalesceFunction;

impl SqlFunction for CoalesceFunction {
    fn name(&self) -> &str {
        "COALESCE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::String
    }

    fn signature(&self) -> &str {
        "COALESCE(value1, value2, ...) -> ANY"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count - at least 1 required
        if args.is_empty() {
            return Err(Error::Validation(
                "COALESCE requires at least 1 argument".to_string(),
            ));
        }

        // Iterate through arguments and return the first non-NULL value
        for arg in args {
            let val = eval_expr(arg, row)?;
            if !matches!(val, Literal::Null) {
                return Ok(val);
            }
        }

        // All arguments were NULL
        Ok(Literal::Null)
    }
}
