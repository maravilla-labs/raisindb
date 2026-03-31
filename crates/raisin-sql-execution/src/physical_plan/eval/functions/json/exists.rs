//! JSON_EXISTS function - check if JSONPath exists in JSON

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Literal, TypedExpr};
use serde_json_path::JsonPath;

/// Check if a JSONPath exists in JSON
///
/// # SQL Signature
/// `JSON_EXISTS(jsonb, path) -> BOOLEAN`
///
/// # Arguments
/// * `jsonb` - JSONB value to query
/// * `path` - JSONPath expression as TEXT
///
/// # Returns
/// * TRUE if path exists and matches at least one value
/// * FALSE if path doesn't exist or matches nothing
/// * FALSE if either argument is NULL
///
/// # Examples
/// ```sql
/// SELECT JSON_EXISTS('{"name": "Alice"}', '$.name') -> TRUE
/// SELECT JSON_EXISTS('{"name": "Alice"}', '$.age') -> FALSE
/// SELECT JSON_EXISTS('{"items": [1,2,3]}', '$.items[*]') -> TRUE
/// SELECT JSON_EXISTS(NULL, '$.name') -> FALSE
/// ```
pub struct JsonExistsFunction;

impl SqlFunction for JsonExistsFunction {
    fn name(&self) -> &str {
        "JSON_EXISTS"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "JSON_EXISTS(jsonb, path) -> BOOLEAN"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 2 {
            return Err(Error::Validation(
                "JSON_EXISTS requires exactly 2 arguments (jsonb, path)".to_string(),
            ));
        }

        let json_val = eval_expr(&args[0], row)?;
        let path_val = eval_expr(&args[1], row)?;

        // Handle NULL propagation - return FALSE for NULLs
        if matches!(json_val, Literal::Null) || matches!(path_val, Literal::Null) {
            return Ok(Literal::Boolean(false));
        }

        // Validate types
        let Literal::JsonB(json) = json_val else {
            return Err(Error::Validation(
                "JSON_EXISTS first argument must be JSONB".to_string(),
            ));
        };

        let Literal::Text(path_str) = path_val else {
            return Err(Error::Validation(
                "JSON_EXISTS second argument must be TEXT (JSONPath)".to_string(),
            ));
        };

        // Parse JSONPath
        let path = JsonPath::parse(&path_str)
            .map_err(|e| Error::Validation(format!("Invalid JSONPath '{}': {}", path_str, e)))?;

        // Check if path exists
        let result = path.query(&json);
        let matches = result.all();
        Ok(Literal::Boolean(!matches.is_empty()))
    }
}
