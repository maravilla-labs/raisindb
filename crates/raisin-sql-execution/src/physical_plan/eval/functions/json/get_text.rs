//! JSON_GET_TEXT function - simple key extraction as text

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Extract a value from JSON by key and convert to TEXT
///
/// # SQL Signature
/// `JSON_GET_TEXT(jsonb, key) -> TEXT`
///
/// # Arguments
/// * `jsonb` - JSONB object to query
/// * `key` - Object key as TEXT
///
/// # Returns
/// * Value converted to TEXT
/// * NULL if key doesn't exist or either argument is NULL
///
/// # Examples
/// ```sql
/// SELECT JSON_GET_TEXT('{"name": "Alice"}', 'name') -> 'Alice'
/// SELECT JSON_GET_TEXT('{"age": 30}', 'age') -> '30'
/// SELECT JSON_GET_TEXT('{"name": "Alice"}', 'missing') -> NULL
/// ```
///
/// # Notes
/// - Simpler than JSON_VALUE, only works with direct object keys
/// - Automatically converts non-string values to strings
pub struct JsonGetTextFunction;

impl SqlFunction for JsonGetTextFunction {
    fn name(&self) -> &str {
        "JSON_GET_TEXT"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "JSON_GET_TEXT(jsonb, key) -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 2 {
            return Err(Error::Validation(
                "JSON_GET_TEXT requires exactly 2 arguments (jsonb, key)".to_string(),
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
                "JSON_GET_TEXT first argument must be JSONB".to_string(),
            ));
        };

        let Literal::Text(key) = key_val else {
            return Err(Error::Validation(
                "JSON_GET_TEXT second argument must be TEXT".to_string(),
            ));
        };

        // Extract value
        match json.get(&key) {
            Some(serde_json::Value::String(s)) => Ok(Literal::Text(s.clone())),
            Some(v) => Ok(Literal::Text(v.to_string())),
            None => Ok(Literal::Null),
        }
    }
}
