//! NOW function - return current UTC timestamp

use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use chrono::Utc;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Return current UTC timestamp
///
/// # SQL Signature
/// `NOW() -> TIMESTAMPTZ`
///
/// # Arguments
/// None
///
/// # Returns
/// * Current timestamp in UTC
///
/// # Examples
/// ```sql
/// SELECT NOW() -> '2024-11-24 10:30:00+00:00'
/// SELECT NOW() - INTERVAL '1 hour' -> timestamp one hour ago
/// ```
///
/// # Notes
/// - Returns the current time at the start of query execution
/// - All timestamps are normalized to UTC
/// - Function is volatile (returns different value each execution)
pub struct NowFunction;

impl SqlFunction for NowFunction {
    fn name(&self) -> &str {
        "NOW"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Temporal
    }

    fn signature(&self) -> &str {
        "NOW() -> TIMESTAMPTZ"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], _row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if !args.is_empty() {
            return Err(Error::Validation(
                "NOW requires exactly 0 arguments".to_string(),
            ));
        }

        // Return current UTC timestamp
        Ok(Literal::Timestamp(Utc::now()))
    }
}
