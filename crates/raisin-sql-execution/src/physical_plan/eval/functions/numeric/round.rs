//! ROUND function - round numeric values to specified decimal places

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Round a numeric value to a specified number of decimal places
///
/// # SQL Signature
/// `ROUND(numeric, decimals?) -> DOUBLE`
///
/// # Arguments
/// * `numeric` - The numeric value to round (INT, BIGINT, or DOUBLE)
/// * `decimals` - Optional number of decimal places (default: 0)
///
/// # Returns
/// * Rounded value as DOUBLE
/// * NULL if input is NULL (SQL NULL propagation)
///
/// # Examples
/// ```sql
/// SELECT ROUND(3.14159) -> 3.0
/// SELECT ROUND(3.14159, 2) -> 3.14
/// SELECT ROUND(3.14159, 4) -> 3.1416
/// SELECT ROUND(42) -> 42.0
/// SELECT ROUND(NULL) -> NULL
/// ```
///
/// # Notes
/// - Follows SQL standard rounding behavior (round half away from zero)
/// - Integer inputs are converted to DOUBLE
/// - Negative decimal places are not supported (use TRUNC for that)
pub struct RoundFunction;

impl SqlFunction for RoundFunction {
    fn name(&self) -> &str {
        "ROUND"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Numeric
    }

    fn signature(&self) -> &str {
        "ROUND(numeric, decimals?) -> DOUBLE"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count (1 or 2)
        if args.is_empty() || args.len() > 2 {
            return Err(Error::Validation(
                "ROUND requires 1 or 2 arguments".to_string(),
            ));
        }

        // Evaluate the numeric value
        let val = eval_expr(&args[0], row)?;

        // Handle NULL propagation
        if matches!(val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Parse optional decimals parameter
        let decimals = if args.len() == 2 {
            match eval_expr(&args[1], row)? {
                Literal::Int(n) => n,
                Literal::BigInt(n) => n as i32,
                Literal::Null => 0, // NULL decimals defaults to 0
                _ => {
                    return Err(Error::Validation(
                        "ROUND second argument must be an integer".to_string(),
                    ))
                }
            }
        } else {
            0 // Default: no decimal places
        };

        // Convert value to double and round
        match val {
            Literal::Double(d) => {
                let multiplier = 10_f64.powi(decimals);
                let rounded = (d * multiplier).round() / multiplier;
                Ok(Literal::Double(rounded))
            }
            Literal::Int(i) => {
                // Convert integer to double
                let d = i as f64;
                if decimals == 0 {
                    Ok(Literal::Double(d))
                } else {
                    // For integers with decimals > 0, no rounding needed
                    Ok(Literal::Double(d))
                }
            }
            Literal::BigInt(i) => {
                // Convert bigint to double
                let d = i as f64;
                if decimals == 0 {
                    Ok(Literal::Double(d))
                } else {
                    // For integers with decimals > 0, no rounding needed
                    Ok(Literal::Double(d))
                }
            }
            _ => Err(Error::Validation(
                "ROUND requires a numeric argument".to_string(),
            )),
        }
    }
}
