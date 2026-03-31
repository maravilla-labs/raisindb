//! JSON_VALUE function - extract scalar value from JSON using JSONPath

use crate::physical_plan::eval::core::eval_expr;
use crate::physical_plan::eval::functions::traits::{FunctionCategory, SqlFunction};
use crate::physical_plan::executor::Row;
use raisin_error::Error;
use raisin_sql::analyzer::{Expr, Literal, TypedExpr};
use serde_json_path::JsonPath;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// Global cache for compiled JSONPath expressions
///
/// Since JSONPath strings are typically constant literals in queries (e.g., '$.username'),
/// we can compile them once and reuse the compiled version across all row evaluations.
/// This avoids the overhead of parsing the same path string thousands of times.
static JSONPATH_CACHE: OnceLock<RwLock<HashMap<String, JsonPath>>> = OnceLock::new();

/// Get or initialize the JSONPath cache
fn get_jsonpath_cache() -> &'static RwLock<HashMap<String, JsonPath>> {
    JSONPATH_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Extract a scalar value from JSON using JSONPath
///
/// # SQL Signature
/// `JSON_VALUE(jsonb, path) -> TEXT`
///
/// # Arguments
/// * `jsonb` - JSONB value to query
/// * `path` - JSONPath expression as TEXT
///
/// # Returns
/// * Scalar value as TEXT (strings, numbers, booleans)
/// * NULL if path doesn't exist or matches NULL
/// * Error if path matches multiple values or non-scalar values
///
/// # Examples
/// ```sql
/// SELECT JSON_VALUE('{"name": "Alice"}', '$.name') -> 'Alice'
/// SELECT JSON_VALUE('{"age": 30}', '$.age') -> '30'
/// SELECT JSON_VALUE('{"active": true}', '$.active') -> 'true'
/// SELECT JSON_VALUE('{"name": "Alice"}', '$.missing') -> NULL
/// ```
///
/// # Notes
/// - Follows SQL/JSON standard behavior
/// - Only returns scalar values (not arrays or objects)
/// - Use JSON_QUERY for extracting arrays/objects
pub struct JsonValueFunction;

impl SqlFunction for JsonValueFunction {
    fn name(&self) -> &str {
        "JSON_VALUE"
    }

    fn category(&self) -> FunctionCategory {
        FunctionCategory::Json
    }

    fn signature(&self) -> &str {
        "JSON_VALUE(jsonb, path) -> TEXT"
    }

    #[inline]
    fn evaluate(&self, args: &[TypedExpr], row: &Row) -> Result<Literal, Error> {
        // Validate argument count
        if args.len() != 2 {
            return Err(Error::Validation(
                "JSON_VALUE requires exactly 2 arguments (jsonb, path)".to_string(),
            ));
        }

        let json_val = eval_expr(&args[0], row)?;
        let path_val = eval_expr(&args[1], row)?;

        // Handle NULL propagation
        if matches!(json_val, Literal::Null) {
            return Ok(Literal::Null);
        }
        if matches!(path_val, Literal::Null) {
            return Ok(Literal::Null);
        }

        // Validate types
        let Literal::JsonB(json) = json_val else {
            return Err(Error::Validation(
                "JSON_VALUE first argument must be JSONB".to_string(),
            ));
        };

        let Literal::Text(path_str) = path_val else {
            return Err(Error::Validation(
                "JSON_VALUE second argument must be TEXT (JSONPath)".to_string(),
            ));
        };

        // Compile or retrieve cached JSONPath
        // If the path argument is a literal (common case), we can cache the compiled version
        let path = if matches!(&args[1].expr, Expr::Literal(_)) {
            // Path is a constant literal - use cache
            let cache_lock = get_jsonpath_cache();
            {
                let cache = cache_lock.read().unwrap();
                if let Some(cached_path) = cache.get(&path_str) {
                    // Cache hit - clone the compiled path
                    cached_path.clone()
                } else {
                    // Cache miss - compile and store
                    drop(cache); // Release read lock before acquiring write lock

                    let compiled = JsonPath::parse(&path_str).map_err(|e| {
                        Error::Validation(format!("Invalid JSONPath '{}': {}", path_str, e))
                    })?;

                    cache_lock
                        .write()
                        .unwrap()
                        .insert(path_str.clone(), compiled.clone());

                    compiled
                }
            }
        } else {
            // Path is dynamic (rare case) - parse each time
            JsonPath::parse(&path_str)
                .map_err(|e| Error::Validation(format!("Invalid JSONPath '{}': {}", path_str, e)))?
        };

        // Query the JSON
        let result = path.query(&json);
        let matches = result.all();

        match matches.len() {
            0 => Ok(Literal::Null), // No matches
            1 => {
                // Extract scalar value as text (PostgreSQL behavior)
                let value = matches[0];
                match value {
                    serde_json::Value::String(s) => Ok(Literal::Text(s.clone())),
                    serde_json::Value::Number(n) => Ok(Literal::Text(n.to_string())),
                    serde_json::Value::Bool(b) => Ok(Literal::Text(b.to_string())),
                    serde_json::Value::Null => Ok(Literal::Null),
                    _ => Err(Error::Validation(format!(
                        "JSON_VALUE can only extract scalar values, got {}",
                        value
                    ))),
                }
            }
            _ => Err(Error::Validation(
                "JSON_VALUE path returned multiple values (use JSON_QUERY for arrays/objects)"
                    .to_string(),
            )),
        }
    }
}
