//! JSON_GET_INT function - extract integer value

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Extract an integer value from JSON by key
///
/// # SQL Signature
/// `JSON_GET_INT(jsonb, key) -> INT`
///
/// # Arguments
/// * `jsonb` - JSONB object to query
/// * `key` - Object key as TEXT
///
/// # Returns
/// * Integer value as INT
/// * NULL if key doesn't exist or either argument is NULL
/// * Error if value is not a number
///
/// # Examples
/// ```sql
/// SELECT JSON_GET_INT('{"count": 42}', 'count') -> 42
/// SELECT JSON_GET_INT('{"price": 19.99}', 'price') -> 19  -- truncated
/// SELECT JSON_GET_INT('{"count": 42}', 'missing') -> NULL
/// ```
///
/// # Notes
/// - Floating point values are truncated to integers
pub struct JsonGetIntFunction;

impl SqlFunction for JsonGetIntFunction {
    fn name(&self) -> &str {
        "JSON_GET_INT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "JSON_GET_INT(jsonb, key) -> INT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 2 {
            return Err(Error::Validation(
                "JSON_GET_INT requires exactly 2 arguments (jsonb, key)".to_string(),
            ));
        }

        let json_val = eval_expr(&args[0], row)?;
        let key_val = eval_expr(&args[1], row)?;

        // Handle NULL propagation
        if matches!(json_val, Literal::Null) || matches!(key_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Validate types
        let Literal::JsonB(json) = json_val else {
            return Err(Error::Validation(
                "JSON_GET_INT first argument must be JSONB".to_string(),
            ));
        };

        let Literal::Text(key) = key_val else {
            return Err(Error::Validation(
                "JSON_GET_INT second argument must be TEXT".to_string(),
            ));
        };

        // Extract value
        match json.get(&key) {
            Some(serde_json::Value::Number(n)) => {
                // Try as integer first, then as float truncated to int
                if let Some(i) = n.as_i64() {
                    Ok(Literal::Int(i as i32))
                } else if let Some(f) = n.as_f64() {
                    Ok(Literal::Int(f as i32))
                } else {
                    Err(Error::Validation(format!("Cannot convert {} to INT", n)))
                }
            }
            Some(_) => Err(Error::Validation(format!(
                "Value at key '{}' is not a number",
                key
            ))),
            None => Ok(Literal::Null),
        }
    }
}
