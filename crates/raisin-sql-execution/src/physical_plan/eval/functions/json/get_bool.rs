//! JSON_GET_BOOL function - extract boolean value

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};

/// Extract a boolean value from JSON by key
///
/// # SQL Signature
/// `JSON_GET_BOOL(jsonb, key) -> BOOLEAN`
///
/// # Arguments
/// * `jsonb` - JSONB object to query
/// * `key` - Object key as TEXT
///
/// # Returns
/// * Boolean value
/// * NULL if key doesn't exist or either argument is NULL
/// * Error if value is not a boolean
///
/// # Examples
/// ```sql
/// SELECT JSON_GET_BOOL('{"active": true}', 'active') -> TRUE
/// SELECT JSON_GET_BOOL('{"enabled": false}', 'enabled') -> FALSE
/// SELECT JSON_GET_BOOL('{"active": true}', 'missing') -> NULL
/// ```
pub struct JsonGetBoolFunction;

impl SqlFunction for JsonGetBoolFunction {
    fn name(&self) -> &str {
        "JSON_GET_BOOL"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "JSON_GET_BOOL(jsonb, key) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 2 {
            return Err(Error::Validation(
                "JSON_GET_BOOL requires exactly 2 arguments (jsonb, key)".to_string(),
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
                "JSON_GET_BOOL first argument must be JSONB".to_string(),
            ));
        };

        let Literal::Text(key) = key_val else {
            return Err(Error::Validation(
                "JSON_GET_BOOL second argument must be TEXT".to_string(),
            ));
        };

        // Extract value
        match json.get(&key) {
            Some(serde_json::Value::Bool(b)) => Ok(Literal::Boolean(*b)),
            Some(_) => Err(Error::Validation(format!(
                "Value at key '{}' is not a boolean",
                key
            ))),
            None => Ok(Literal::Null),
        }
    }
}
