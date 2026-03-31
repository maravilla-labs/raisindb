//! UPPER function - convert text to uppercase

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Convert text to uppercase
///
/// # SQL Signature
/// `UPPER(text) -> TEXT`
///
/// # Arguments
/// * `text` - Input string to convert to uppercase
///
/// # Returns
/// * Uppercase version of the input string
/// * NULL if input is NULL (SQL NULL propagation)
///
/// # Examples
/// ```sql
/// SELECT UPPER('hello') -> 'HELLO'
/// SELECT UPPER('Hello World') -> 'HELLO WORLD'
/// SELECT UPPER(NULL) -> NULL
/// ```
pub struct UpperFunction;

impl SqlFunction for UpperFunction {
    fn name(&self) -> &str {
        "UPPER"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::String
    }

    fn signature(&self) -> &str {
        "UPPER(text) -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 1 {
            return Err(Error::Validation(
                "UPPER requires exactly 1 argument".to_string(),
            ));
        }

        // Evaluate the argument
        let val = eval_expr(&args[0], row)?;

        // Handle NULL propagation
        if matches!(val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Validate type and convert
        let Literal::Text(s) = val else {
            return Err(Error::Validation(
                "UPPER requires a text argument".to_string(),
            ));
        };

        Ok(Literal::Text(s.to_uppercase()))
    }
}
